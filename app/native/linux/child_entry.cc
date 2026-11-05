// With the host's flag first on the command line this process is one of the host's own commands (provider, tool, launch),
// and ends here, before the runner's main and so before GTK or Flutter. The library is found by the executable's run path,
// which names its lib directory.
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

__attribute__((constructor)) static void RunAsChildIfAsked(int argc, char** argv, char**) {
  if (argc < 2 || strcmp(argv[1], "--canoa-child") != 0) {
    return;
  }
  void* library = dlopen("libcanoa_ffi.so", RTLD_NOW);
  if (library == nullptr) {
    fprintf(stderr, "canoa: %s\n", dlerror());
    _exit(70);
  }
  using ChildMain = int (*)();
  auto child_main = reinterpret_cast<ChildMain>(dlsym(library, "canoa_ffi_child_main"));
  if (child_main == nullptr) {
    fprintf(stderr, "canoa: %s\n", dlerror());
    _exit(70);
  }
  _exit(child_main());
}
