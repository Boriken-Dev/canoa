#include "encoder/pcm_encoder.h"
#include "utils.h"

namespace record_windows {

std::vector<std::vector<uint8_t>> PcmEncoder::Feed(IMFSample* pSample)
{
	IMFMediaBuffer* pBuffer = NULL;
	if (FAILED(pSample->ConvertToContiguousBuffer(&pBuffer)))
		return {};

	BYTE* pData = NULL;
	DWORD size  = 0;
	std::vector<std::vector<uint8_t>> result;

	if (SUCCEEDED(pBuffer->Lock(&pData, NULL, &size)) && size > 0)
	{
		result.push_back(std::vector<uint8_t>(pData, pData + size));
		pBuffer->Unlock();
	}

	SafeRelease(pBuffer);
	return result;
}

} // namespace record_windows
