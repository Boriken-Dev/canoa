# pasteboard (vendored copy)

A copy of the `pasteboard` Flutter plugin, version 0.5.0, from
https://github.com/MixinNetwork/flutter-plugins/tree/main/packages/pasteboard
(MIT, see `LICENSE`). It reads images, files and HTML from the clipboard and
writes files and images to it.

Only what builds on the platforms the app targets is kept: `lib/`, the
platform folders' sources and build files, `pubspec.yaml`, `analysis_options.yaml`
and `LICENSE`. The example, the changelog, the Gradle wrapper and the upstream
README are left out.

## Changes from upstream

- Windows, `files`: each CF_HDROP name is read with `DragQueryFileW`, sized by
  the length the API reports for that name, instead of into a fixed
  `MAX_PATH` buffer with a byte count passed where a character count belongs.
  A name over 260 characters is read whole, an empty name is skipped, and the
  clipboard's memory is unlocked before the clipboard is closed.
- Line endings are LF throughout.
- Windows: the name extraction is `ReadDropNames` in `windows/drop_names.{h,cpp}`
  so it can be tested without a clipboard.
- Linux: the path `g_file_get_path` returns is freed after each file URI.

## Testing the Windows name extraction

`windows/test/drop_names_test.cpp` builds a CF_HDROP block in memory (several
names, one of 382 UTF-16 units with non-ASCII text and a surrogate pair, and an
empty list), checks `ReadDropNames` returns each name whole, and runs the
original fixed-buffer logic beside it to count the characters it writes past
its buffer. It never touches the clipboard. From a Visual Studio developer
prompt, in `windows/test/`:

```
cl /EHsc /std:c++17 /W3 /WX /DUNICODE /D_UNICODE drop_names_test.cpp ..\drop_names.cpp shell32.lib user32.lib
drop_names_test.exe
```

Delete the `.exe` and `.obj` files afterwards.
