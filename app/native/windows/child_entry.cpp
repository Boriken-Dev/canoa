// With the host's flag first on the command line this process is one of the host's own commands (provider, tool, launch),
// and ends here, before the runner's wWinMain and so before Flutter or a window. A static initializer runs outside the
// loader lock, with the executable's own imports loaded.
#include <windows.h>

#include <shellapi.h>
#include <wchar.h>

namespace {

int RunAsChildIfAsked() {
  int count = 0;
  wchar_t** args = ::CommandLineToArgvW(::GetCommandLineW(), &count);
  // The flag, or the extension's origin a browser passes to a native messaging host it starts: then this process is the relay.
  const bool asked =
      args != nullptr && count >= 2 &&
      (::wcscmp(args[1], L"--canoa-child") == 0 ||
       ::wcsncmp(args[1], L"chrome-extension://", 19) == 0);
  if (args != nullptr) {
    ::LocalFree(args);
  }
  if (!asked) {
    return 0;
  }
  // Only beside this executable, and the system's own for what it imports: never the working directory or PATH.
  HMODULE library = ::LoadLibraryExW(L"canoa_ffi.dll", nullptr, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
  if (library == nullptr) {
    ::ExitProcess(70);
  }
  using ChildMain = int (*)();
  auto child_main = reinterpret_cast<ChildMain>(::GetProcAddress(library, "canoa_ffi_child_main"));
  if (child_main == nullptr) {
    ::ExitProcess(70);
  }
  ::ExitProcess(static_cast<UINT>(child_main()));
}

[[maybe_unused]] const int run_as_child = RunAsChildIfAsked();

}  // namespace
