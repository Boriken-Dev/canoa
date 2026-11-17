#pragma once

#include <windows.h>
#include <mfidl.h>
#include <vector>
#include <cstdint>

namespace record_windows {

class IStreamEncoder
{
public:
	virtual ~IStreamEncoder() = default;
	virtual std::vector<std::vector<uint8_t>> Feed(IMFSample* pSample) = 0;
};

} // namespace record_windows
