#pragma once

#include <windows.h>
#include <mfidl.h>
#include <mfapi.h>
#include <mferror.h>
#include <mftransform.h>
#include <vector>
#include <cstdint>

#include "record_config.h"
#include "utils.h"
#include "encoder/stream_encoder.h"

namespace record_windows {

class AacAdtsEncoder : public IStreamEncoder
{
public:
	static HRESULT Create(const RecordConfig& config, AacAdtsEncoder** ppOut);
	~AacAdtsEncoder() override;

	AacAdtsEncoder(const AacAdtsEncoder&) = delete;
	AacAdtsEncoder& operator=(const AacAdtsEncoder&) = delete;

	std::vector<std::vector<uint8_t>> Feed(IMFSample* pSample) override;

private:
	AacAdtsEncoder() = default;
	HRESULT Init(const RecordConfig& config);
	std::vector<std::vector<uint8_t>> Drain();

	static void BuildAdtsHeader(uint8_t* hdr, int sampleRate, int channels, DWORD frameLen);

	IMFTransform*  m_pTransform  = NULL;
	int            m_sampleRate  = 0;
	int            m_numChannels = 0;
};

} // namespace record_windows
