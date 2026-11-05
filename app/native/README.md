# canoa_native

A Flutter FFI plugin of the app's own: it builds `crates/canoa-ffi` with cargo when the desktop app builds, carries the
library inside the app, and holds the Dart declarations of its C ABI. `cargo` has to be on PATH or in `~/.cargo/bin`, and the
plugin has to stay in this repository, whose workspace its build files find from where they are.

Windows and Linux are built and tested. macOS is written and has not been built or run.
