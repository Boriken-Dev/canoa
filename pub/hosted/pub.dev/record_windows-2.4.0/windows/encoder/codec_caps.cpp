#include "encoder/codec_caps.h"

namespace record_windows {

size_t SelectBestCaps(const std::vector<CodecCapsEntry>& caps,
                      UINT32 reqCh, UINT32 reqSr, UINT32 reqBr)
{
	if (caps.empty()) return SIZE_MAX;

	auto absDiff = [](UINT32 a, UINT32 b) -> UINT32 { return a >= b ? a - b : b - a; };

	size_t best = 0;
	for (size_t i = 1; i < caps.size(); i++)
	{
		const auto& c = caps[i];
		const auto& b = caps[best];

		UINT32 cCh = absDiff(c.channels,   reqCh), bCh = absDiff(b.channels,   reqCh);
		UINT32 cSr = absDiff(c.sampleRate, reqSr), bSr = absDiff(b.sampleRate, reqSr);
		UINT32 cBr = absDiff(c.bitRate,    reqBr), bBr = absDiff(b.bitRate,    reqBr);

		if (cCh < bCh || (cCh == bCh && cSr < bSr) || (cCh == bCh && cSr == bSr && cBr < bBr))
			best = i;
	}

	return best;
}

} // namespace record_windows
