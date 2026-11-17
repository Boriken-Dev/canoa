#ifndef PASTEBOARD_DROP_NAMES_H_
#define PASTEBOARD_DROP_NAMES_H_

#include <Windows.h>
#include <ShlObj.h>

#include <string>
#include <vector>

// The file names in a locked CF_HDROP block, each at its full length (names
// are not limited to MAX_PATH). An entry that cannot be read is skipped.
std::vector<std::wstring> ReadDropNames(HDROP drop);

#endif  // PASTEBOARD_DROP_NAMES_H_
