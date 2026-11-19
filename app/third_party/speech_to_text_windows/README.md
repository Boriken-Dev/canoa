# speech_to_text_windows (Canoa's copy)

A copy of the `speech_to_text_windows` Flutter plugin, version 1.0.1, from
https://github.com/csdcorp/speech_to_text/tree/main/speech_to_text_windows
(BSD 3-Clause, see `LICENSE`). It is the Windows implementation of
[`speech_to_text`](https://pub.dev/packages/speech_to_text) over the desktop
speech recognizer (SAPI 5, the recognizer of Windows Speech Recognition), which
the app's dictation uses on Windows. The app's `pubspec.yaml` overrides the pub
package with this directory.

The example, the changelog and the upstream README are left out.

## Changes from upstream

The plugin as published listened and returned nothing a person could use:

- It never set the context's event interest, so a context queued only the final
  phrase of an utterance and never the words as they were recognized. It now
  asks for the hypotheses too, and the words so far reach Dart as they are said.
- `stop` set the dictation state inactive and ended its reading thread at once,
  so the phrase the recognizer still held (a person stops before the pause that
  ends a phrase) was dropped. It now asks the recognizer to finish, reports the
  phrase it holds, and then says `notListening` and `done`.
- Each phrase came up as a final result, and `speech_to_text` drops everything
  after the first final result of a listen, so a second sentence was lost. Each
  update now carries every phrase of the session so far as a partial result,
  and one final result carries them all when the session ends.
- A result's text was put into JSON without escaping, so a quotation mark or a
  backslash in it made the result undecodable. It is escaped.
- The recognition thread called Flutter's channel from a thread of its own.
  SAPI now lives on one worker thread with its own COM apartment, and what it
  reports goes to the platform thread through a message-only window.
- `locales` answered two fixed English locales. It lists the languages of the
  recognizers installed (`en-US:English (United States)`), the system's own
  first, and `listen` with a `localeId` uses a recognizer for that language.
- Failures are reported: `error_no_recognizer`, `error_language_unavailable`,
  `error_audio` (no microphone, or one the system will not open) and the
  `HRESULT` of anything else reach Dart through `notifyError` as a permanent
  error, and a native call that throws reaches it too. `listen` returning
  `false` is also an error.
- `initialize` takes the option `inputFile` (a PCM WAV path): that recording is
  played in real time in place of the microphone. Tests use it.
- Line endings are LF throughout.

## Layout

- `lib/speech_to_text_windows.dart`: the platform class.
- `windows/speech_to_text_windows_plugin.*`: the channel and the hand-over to the
  platform thread.
- `windows/sapi_dictation.*`: dictation over SAPI, with no Flutter in it. The only
  native code, and the only place COM is used.
- `windows/paced_audio.h`: a recording played as a device plays it.
- `windows/test/`: a native test of `sapi_dictation` against the installed
  recognizer, fed recordings the system's voice makes when the test runs.

## Native test

```
cmake -S windows/test -B build -G "Visual Studio 17 2022" -A ARM64   # or x64
cmake --build build --config Release
build/Release/sapi_dictation_test.exe
```

It needs a recognizer and a text-to-speech voice installed (the English ones
Windows ships); without them it says so and exits 0. It never opens the
microphone. `sapi_dictation_test.exe --make-wav OUT.wav "words"` writes the
recording of a sentence (SAPI XML, so `<silence msec="1500"/>` pauses), for a
test of the whole plugin through Flutter.
