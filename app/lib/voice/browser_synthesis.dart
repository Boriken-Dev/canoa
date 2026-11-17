import 'browser_synthesis_stub.dart' if (dart.library.js_interop) 'browser_synthesis_web.dart' as platform;
import 'browser_voices.dart';

/// The browser's speech synthesis on a web page that has it, else null.
BrowserSynthesis? platformBrowserSynthesis() => platform.create();
