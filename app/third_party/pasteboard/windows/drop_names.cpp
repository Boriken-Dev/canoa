#include "drop_names.h"

std::vector<std::wstring> ReadDropNames(HDROP drop) {
  std::vector<std::wstring> names;
  UINT count = DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
  for (UINT i = 0; i < count; ++i) {
    // The length excludes the terminating null.
    UINT length = DragQueryFileW(drop, i, nullptr, 0);
    if (length == 0) continue;
    std::wstring name(static_cast<size_t>(length) + 1, L'\0');
    UINT copied = DragQueryFileW(drop, i, &name[0], length + 1);
    if (copied == 0) continue;
    name.resize(copied);
    names.push_back(std::move(name));
  }
  return names;
}
