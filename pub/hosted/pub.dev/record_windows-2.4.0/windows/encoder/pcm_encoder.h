#pragma once

#include "encoder/stream_encoder.h"

namespace record_windows {

class PcmEncoder : public IStreamEncoder
{
public:
	std::vector<std::vector<uint8_t>> Feed(IMFSample* pSample) override;
};

} // namespace record_windows
