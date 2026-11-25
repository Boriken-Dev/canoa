import 'web_whisper.dart';
import 'web_whisper_runtime_stub.dart' if (dart.library.js_interop) 'web_whisper_runtime_web.dart' as platform;

/// Whisper in the page's own web worker on a web page, else null: no other platform runs it.
WhisperRuntime? platformWhisperRuntime() => platform.create();
