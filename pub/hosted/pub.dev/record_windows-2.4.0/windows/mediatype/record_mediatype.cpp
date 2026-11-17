#include "mediatype/record_mediatype.h"
#include "utils.h"

#pragma warning(disable: 4201)
#include <aviriff.h>

namespace record_windows {
namespace MediaType {

static HRESULT CreateAACProfile(const RecordConfig& config, IMFMediaType* pType)
{
	HRESULT hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_AAC);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sampleRate);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, config.numChannels);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AVG_BITRATE, config.bitRate);
	return hr;
}

static HRESULT CreateFlacProfile(const RecordConfig& config, IMFMediaType* pType)
{
	HRESULT hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_FLAC);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sampleRate);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, config.numChannels);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AVG_BITRATE, config.bitRate);
	return hr;
}

static HRESULT CreateAmrNbProfile(IMFMediaType* pType)
{
	HRESULT hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_AMR_NB);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
	return hr;
}

static HRESULT CreateAmrWbProfile(IMFMediaType* pType)
{
	HRESULT hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_AMR_WB);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
	return hr;
}

static HRESULT CreatePcmProfile(const RecordConfig& config, IMFMediaType* pType)
{
	const UINT32 bitsPerSample = 16;
	HRESULT hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_PCM);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, bitsPerSample);

	UINT32 blockAlign     = config.numChannels * (bitsPerSample / 8);
	UINT32 bytesPerSecond = blockAlign * config.sampleRate;

	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, config.numChannels);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sampleRate);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BLOCK_ALIGNMENT, blockAlign);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytesPerSecond);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE);
	return hr;
}

HRESULT CreateInputProfile(const RecordConfig& config, IMFMediaType** ppType)
{
	IMFMediaType* pType = NULL;
	HRESULT hr = MFCreateMediaType(&pType);

	if (SUCCEEDED(hr)) hr = pType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Audio);
	if (SUCCEEDED(hr)) hr = pType->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_PCM);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sampleRate);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, config.numChannels);
	if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_AVG_BITRATE, config.bitRate);
	if (SUCCEEDED(hr))
	{
		*ppType = pType;
		(*ppType)->AddRef();
	}

	SafeRelease(&pType);

	return hr;
}

HRESULT CreateOutputProfile(const RecordConfig& config, IMFMediaType** ppType)
{
	IMFMediaType* pType = NULL;
	HRESULT hr = MFCreateMediaType(&pType);

	if (SUCCEEDED(hr)) hr = pType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Audio);
	if (SUCCEEDED(hr))
	{
		const auto& enc = config.encoderName;
		if      (enc == AudioEncoder::aacLc ||
		         enc == AudioEncoder::aacEld ||
		         enc == AudioEncoder::aacHe)        hr = CreateAACProfile(config, pType);
		else if (enc == AudioEncoder::amrNb)        hr = CreateAmrNbProfile(pType);
		else if (enc == AudioEncoder::amrWb)        hr = CreateAmrWbProfile(pType);
		else if (enc == AudioEncoder::flac)         hr = CreateFlacProfile(config, pType);
		else if (enc == AudioEncoder::pcm16bits ||
		         enc == AudioEncoder::wav)          hr = CreatePcmProfile(config, pType);
		else hr = E_NOTIMPL;
	}
	if (SUCCEEDED(hr))
	{
		*ppType = pType;
		(*ppType)->AddRef();
	}

	SafeRelease(&pType);
	
	return hr;
}

HRESULT FillWavHeader(const std::wstring& path)
{
	HANDLE hFile = CreateFile(
		path.c_str(),
		GENERIC_READ | GENERIC_WRITE, 0, NULL,
		OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL
	);

	if (hFile == INVALID_HANDLE_VALUE)
	{
		printf("Record: Error when opening WAVE file.");
		return E_FAIL;
	}

	DWORD fileSize = GetFileSize(hFile, NULL);
	DWORD bytesTransferred = 0;

	RIFFCHUNK riffHeader;
	DWORD fccWaveType = 0;

	if (fileSize == INVALID_FILE_SIZE ||
		!ReadFile(hFile, &riffHeader, sizeof(riffHeader), &bytesTransferred, NULL) ||
		bytesTransferred != sizeof(riffHeader) || riffHeader.fcc != MAKEFOURCC('R', 'I', 'F', 'F') ||
		!ReadFile(hFile, &fccWaveType, sizeof(fccWaveType), &bytesTransferred, NULL) ||
		bytesTransferred != sizeof(fccWaveType) || fccWaveType != MAKEFOURCC('W', 'A', 'V', 'E'))
	{
		printf("Record: Unrecognized WAVE file layout.");
		CloseHandle(hFile);
		return E_FAIL;
	}

	// Walk the chunks following the "WAVE" fourcc to locate the "data" chunk.
	DWORD offset = sizeof(RIFFCHUNK) + sizeof(DWORD);
	DWORD dataChunkOffset = 0;
	bool foundDataChunk = false;

	while (offset + sizeof(RIFFCHUNK) <= fileSize)
	{
		RIFFCHUNK chunk;
		if (!ReadFile(hFile, &chunk, sizeof(chunk), &bytesTransferred, NULL) || bytesTransferred != sizeof(chunk))
		{
			break;
		}

		if (chunk.fcc == MAKEFOURCC('d', 'a', 't', 'a'))
		{
			dataChunkOffset = offset;
			foundDataChunk = true;
			break;
		}

		offset += sizeof(RIFFCHUNK) + RIFFROUND(chunk.cb);
		if (SetFilePointer(hFile, offset, NULL, FILE_BEGIN) == INVALID_SET_FILE_POINTER)
		{
			break;
		}
	}

	if (!foundDataChunk)
	{
		printf("Record: Could not locate WAVE data chunk.");
		CloseHandle(hFile);
		return E_FAIL;
	}

	DWORD riffSize = fileSize - sizeof(RIFFCHUNK);
	DWORD dataSize = fileSize - dataChunkOffset - sizeof(RIFFCHUNK);

	HRESULT hr = S_OK;

	if (SetFilePointer(hFile, FIELD_OFFSET(RIFFCHUNK, cb), NULL, FILE_BEGIN) == INVALID_SET_FILE_POINTER ||
		!WriteFile(hFile, &riffSize, sizeof(riffSize), &bytesTransferred, NULL) || bytesTransferred != sizeof(riffSize))
	{
		printf("Record: Error when writing WAVE RIFF size.");
		hr = E_FAIL;
	}

	if (SUCCEEDED(hr) &&
		(SetFilePointer(hFile, dataChunkOffset + FIELD_OFFSET(RIFFCHUNK, cb), NULL, FILE_BEGIN) == INVALID_SET_FILE_POINTER ||
		!WriteFile(hFile, &dataSize, sizeof(dataSize), &bytesTransferred, NULL) || bytesTransferred != sizeof(dataSize)))
	{
		printf("Record: Error when writing WAVE data size.");
		hr = E_FAIL;
	}

	if (!CloseHandle(hFile) && SUCCEEDED(hr))
	{
		printf("Record: Error when closing WAVE file.");
		hr = E_FAIL;
	}

	return hr;
}

} // namespace MediaType
} // namespace record_windows
