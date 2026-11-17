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
