#pragma once

#include <windows.h>
#include <vector>
#include <cstddef>

namespace record_windows {

struct CodecCapsEntry { UINT32 channels, sampleRate, bitRate; };

// Returns the index of the best-matching entry using channel -> sampleRate -> bitRate
// priority, or SIZE_MAX if caps is empty.
size_t SelectBestCaps(const std::vector<CodecCapsEntry>& caps,
                      UINT32 reqCh, UINT32 reqSr, UINT32 reqBr);

} // namespace record_windows
