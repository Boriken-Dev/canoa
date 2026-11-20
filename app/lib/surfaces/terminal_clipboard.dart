import 'package:flutter/foundation.dart' show VoidCallback;

import 'terminal_clipboard_native.dart' if (dart.library.js_interop) 'terminal_clipboard_web.dart' as platform;

/// The platform declined to give or take the clipboard; [message] says why, in words for the person.
final class ClipboardRefused implements Exception {
  const ClipboardRefused(this.message);

  final String message;

  @override
  String toString() => message;
}

/// The clipboard as a terminal uses it. The system's is [SystemTerminalClipboard]; a test gives its own.
abstract interface class TerminalClipboard {
  /// Puts [text] on the clipboard. Called from the person's own key or click, which is what a browser asks of it.
  /// Throws [ClipboardRefused] when the platform will not take it.
  Future<void> write(String text);

  /// The clipboard's text, or null when it holds none. A browser asks the person's permission for this, so a keyboard paste goes
  /// through [onPaste] instead. Throws [ClipboardRefused] when the platform will not give it.
  Future<String?> read();

  /// Calls [handler] with the text of each paste the person makes with the platform's own gesture (a browser's `paste` event, which
  /// needs no permission), until the returned function is called. [handler] answers whether it took the paste; a paste nobody took
  /// goes on to the page as it would have. Never called where the platform has no such event.
  VoidCallback onPaste(bool Function(String text) handler);

  /// Whether the paste keys are left to the platform, which then delivers the text to [onPaste], rather than read here.
  bool get pastesByEvent;
}

/// The system's clipboard; in a browser, the page's own.
final class SystemTerminalClipboard implements TerminalClipboard {
  const SystemTerminalClipboard();

  @override
  Future<void> write(String text) => platform.writeText(text);

  @override
  Future<String?> read() => platform.readText();

  @override
  VoidCallback onPaste(bool Function(String text) handler) => platform.listenPaste(handler);

  @override
  bool get pastesByEvent => platform.pastesByEvent;
}
