// Standalone check of ReadDropNames against an in-memory CF_HDROP block; it
// never touches the clipboard. Build and run: see the vendored README.
#include <Windows.h>
#include <ShlObj.h>

#include <cstdio>
#include <string>
#include <vector>

#include "../drop_names.h"

namespace {

int failures = 0;

void Check(bool ok, const char *what) {
  std::printf("  %s: %s\n", ok ? "ok  " : "FAIL", what);
  if (!ok) ++failures;
}

// A DROPFILES block: header, each name and its null, one more null.
HDROP MakeDrop(const std::vector<std::wstring> &names) {
  size_t chars = 1;
  for (const auto &n : names) chars += n.size() + 1;
  HGLOBAL mem = GlobalAlloc(GHND, sizeof(DROPFILES) + chars * sizeof(wchar_t));
  auto *drop = static_cast<DROPFILES *>(GlobalLock(mem));
  drop->pFiles = sizeof(DROPFILES);
  drop->fWide = TRUE;
  auto *out = reinterpret_cast<wchar_t *>(reinterpret_cast<BYTE *>(drop) +
                                          sizeof(DROPFILES));
  for (const auto &n : names) {
    for (wchar_t c : n) *out++ = c;
    *out++ = L'\0';
  }
  *out = L'\0';
  return reinterpret_cast<HDROP>(drop);
}

void FreeDrop(HDROP drop) {
  HGLOBAL mem = GlobalHandle(drop);
  GlobalUnlock(mem);
  GlobalFree(mem);
}

// Exactly 382 UTF-16 units: ASCII, Latin, CJK and a surrogate pair.
std::wstring LongName() {
  std::wstring s = L"C:\\dir\\";
  const wchar_t unit[] = {0x00e9, 0x4e2d, 0x6587, L'a'};
  while (s.size() < 378) s.push_back(unit[s.size() % 4]);
  s.push_back(0xD83D);  // U+1F600
  s.push_back(0xDE00);
  s += L".t";
  return s;
}

// The extraction as it was: a MAX_PATH buffer given a size in bytes. The
// buffer sits in front of a canary so the overflow is measured, not suffered.
struct OldBuffer {
  wchar_t filename[MAX_PATH];
  wchar_t canary[1024];
};

std::vector<std::wstring> OldReadNames(HDROP drop, size_t *overflow) {
  std::vector<std::wstring> names;
  *overflow = 0;
  UINT files = DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
  for (UINT i = 0; i < files; ++i) {
    static OldBuffer b;
    for (auto &c : b.canary) c = 0xAAAA;
    // Original: DragQueryFile(data, i, filename, sizeof(TCHAR) * MAX_PATH).
    DragQueryFileW(drop, i, b.filename, sizeof(wchar_t) * MAX_PATH);
    for (size_t k = 0; k < 1024; ++k)
      if (b.canary[k] != 0xAAAA && k + 1 > *overflow) *overflow = k + 1;
    names.emplace_back(b.filename, wcsnlen(b.filename, MAX_PATH + 1024));
  }
  return names;
}

}  // namespace

int main() {
  const std::wstring long_name = LongName();
  std::printf("long name: %zu wide characters\n", long_name.size());
  if (long_name.size() != 382) return 2;
  const std::vector<std::wstring> names = {
      L"C:\\a.txt", long_name, L"C:\\\u00fcber\\\u65e5\u672c.txt", L"D:\\z"};

  HDROP drop = MakeDrop(names);
  std::printf("ReadDropNames, several names:\n");
  auto got = ReadDropNames(drop);
  Check(got.size() == names.size(), "every name returned");
  Check(got == names, "each name whole and in order");
  Check(got.size() > 1 && got[1].size() == 382,
        "the 382-character name keeps its length");

  std::printf("old extraction (MAX_PATH buffer, byte count), for comparison:\n");
  size_t overflow = 0;
  auto old = OldReadNames(drop, &overflow);
  std::printf("  wrote %zu wide characters past the %d-character buffer\n",
              overflow, MAX_PATH);
  std::printf("  returned name length %zu of %zu\n",
              old.size() > 1 ? old[1].size() : 0, long_name.size());
  FreeDrop(drop);

  HDROP empty = MakeDrop({});
  std::printf("ReadDropNames, empty list:\n");
  Check(ReadDropNames(empty).empty(), "no names");
  FreeDrop(empty);

  std::printf("%s\n", failures ? "FAILED" : "PASSED");
  return failures ? 1 : 0;
}
