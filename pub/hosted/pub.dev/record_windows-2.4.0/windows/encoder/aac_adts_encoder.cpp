#include "encoder/aac_adts_encoder.h"
#include "encoder/codec_caps.h"
#include "mediatype/record_mediatype.h"

#include <cstring>

namespace record_windows {

// static
HRESULT AacAdtsEncoder::Create(const RecordConfig& config, AacAdtsEncoder** ppOut)
{
	auto* p = new (std::nothrow) AacAdtsEncoder();
	if (!p) return E_OUTOFMEMORY;

	HRESULT hr = p->Init(config);
	if (SUCCEEDED(hr))
		*ppOut = p;
	else
		delete p;

	return hr;
}

AacAdtsEncoder::~AacAdtsEncoder()
{
	if (m_pTransform)
	{
		m_pTransform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
		SafeRelease(m_pTransform);
	}
}

HRESULT AacAdtsEncoder::Init(const RecordConfig& config)
{
	IMFActivate** ppActivate = NULL;
	UINT32        count      = 0;
	IMFTransform* pTransform = NULL;
	IMFMediaType* pTypeIn    = NULL;

	MFT_REGISTER_TYPE_INFO outputInfo = { MFMediaType_Audio, MFAudioFormat_AAC };
	DWORD dwFlags = (MFT_ENUM_FLAG_ALL & (~MFT_ENUM_FLAG_FIELDOFUSE)) | MFT_ENUM_FLAG_SORTANDFILTER;

	HRESULT hr = MFTEnumEx(MFT_CATEGORY_AUDIO_ENCODER, dwFlags, NULL, &outputInfo, &ppActivate, &count);
	if (SUCCEEDED(hr) && count == 0) hr = MF_E_NOT_FOUND;
	if (SUCCEEDED(hr)) hr = ppActivate[0]->ActivateObject(IID_PPV_ARGS(&pTransform));

	// The encoder only accepts output types it advertises; enumerate and pick best match.
	if (SUCCEEDED(hr))
	{
		std::vector<IMFMediaType*>  types;
		std::vector<CodecCapsEntry> entries;

		for (DWORD i = 0; ; i++)
		{
			IMFMediaType* pType = NULL;
			if (FAILED(pTransform->GetOutputAvailableType(0, i, &pType))) break;

			CodecCapsEntry e = {};
			pType->GetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, &e.sampleRate);
			pType->GetUINT32(MF_MT_AUDIO_NUM_CHANNELS,       &e.channels);
			pType->GetUINT32(MF_MT_AVG_BITRATE,              &e.bitRate);
			if (e.sampleRate > 0 && e.channels > 0) {
				types.push_back(pType);
				entries.push_back(e);
			} else {
				SafeRelease(pType);
			}
		}

		size_t idx = SelectBestCaps(entries,
			(UINT32)config.numChannels, (UINT32)config.sampleRate, (UINT32)config.bitRate);

		hr = (idx != SIZE_MAX) ? pTransform->SetOutputType(0, types[idx], 0) : MF_E_NOT_FOUND;

		for (auto* t : types) SafeRelease(t);
	}

	if (SUCCEEDED(hr)) hr = MediaType::CreateInputProfile(config, &pTypeIn);
	if (SUCCEEDED(hr)) hr = pTransform->SetInputType(0, pTypeIn, 0);
	if (SUCCEEDED(hr)) hr = pTransform->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);

	if (SUCCEEDED(hr))
	{
		m_pTransform  = pTransform;
		m_pTransform->AddRef();
		m_sampleRate  = config.sampleRate;
		m_numChannels = config.numChannels;
	}

	for (UINT32 i = 0; i < count; i++) SafeRelease(ppActivate[i]);
	CoTaskMemFree(ppActivate);
	SafeRelease(&pTypeIn);
	SafeRelease(&pTransform);

	return hr;
}

std::vector<std::vector<uint8_t>> AacAdtsEncoder::Feed(IMFSample* pSample)
{
	HRESULT hr = m_pTransform->ProcessInput(0, pSample, 0);
	return SUCCEEDED(hr) ? Drain() : std::vector<std::vector<uint8_t>>{};
}

std::vector<std::vector<uint8_t>> AacAdtsEncoder::Drain()
{
	std::vector<std::vector<uint8_t>> packets;

	MFT_OUTPUT_STREAM_INFO si = {};
	m_pTransform->GetOutputStreamInfo(0, &si);
	const bool  mftProvidesBuffer = (si.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES) != 0;
	const DWORD bufSize = si.cbSize > 0 ? si.cbSize : 8192;

	while (true)
	{
		IMFSample*      pOut = NULL;
		IMFMediaBuffer* pBuf = NULL;

		if (!mftProvidesBuffer)
		{
			if (FAILED(MFCreateSample(&pOut))) break;
			if (FAILED(MFCreateMemoryBuffer(bufSize, &pBuf)))
			{
				SafeRelease(pOut);
				break;
			}
			pOut->AddBuffer(pBuf);
			SafeRelease(pBuf);
		}

		MFT_OUTPUT_DATA_BUFFER outputBuffer = {};
		outputBuffer.pSample = pOut;
		DWORD status = 0;

		HRESULT hr = m_pTransform->ProcessOutput(0, 1, &outputBuffer, &status);

		if (outputBuffer.pEvents) outputBuffer.pEvents->Release();

		if (hr == MF_E_TRANSFORM_NEED_MORE_INPUT)
		{
			SafeRelease(pOut);
			break;
		}

		if (SUCCEEDED(hr))
		{
			IMFSample* pResult = mftProvidesBuffer ? outputBuffer.pSample : pOut;
			if (pResult)
			{
				IMFMediaBuffer* pContiguous = NULL;
				if (SUCCEEDED(pResult->ConvertToContiguousBuffer(&pContiguous)))
				{
					BYTE* pData  = NULL;
					DWORD cbData = 0;
					if (SUCCEEDED(pContiguous->Lock(&pData, NULL, &cbData)))
					{
						if (cbData > 0)
						{
							std::vector<uint8_t> packet(7 + cbData);
							BuildAdtsHeader(packet.data(), m_sampleRate, m_numChannels, cbData);
							std::memcpy(packet.data() + 7, pData, cbData);
							packets.push_back(std::move(packet));
						}
						pContiguous->Unlock();
					}
					SafeRelease(pContiguous);
				}
			}
		}

		if (mftProvidesBuffer)
			SafeRelease(outputBuffer.pSample);
		else
			SafeRelease(pOut);

		if (FAILED(hr)) break;
	}

	return packets;
}

// static
void AacAdtsEncoder::BuildAdtsHeader(uint8_t* hdr, int sampleRate, int channels, DWORD frameLen)
{
	static const int kFreqTable[] = {
		96000, 88200, 64000, 48000, 44100, 32000,
		24000, 22050, 16000, 12000, 11025, 8000, 7350
	};
	int freqIdx = 4; // 44100 fallback
	for (int i = 0; i < 13; i++)
		if (kFreqTable[i] == sampleRate) { freqIdx = i; break; }

	DWORD totalLen = frameLen + 7;
	hdr[0] = 0xFF;
	hdr[1] = 0xF1; // MPEG-4, layer=0, no CRC
	hdr[2] = (uint8_t)((0x01 << 6) | (freqIdx << 2) | (channels >> 2)); // profile=1 (AAC-LC)
	hdr[3] = (uint8_t)(((channels & 3) << 6) | ((totalLen >> 11) & 0x3));
	hdr[4] = (uint8_t)((totalLen >> 3) & 0xFF);
	hdr[5] = (uint8_t)(((totalLen & 7) << 5) | 0x1F);
	hdr[6] = 0xFC;
}

} // namespace record_windows
