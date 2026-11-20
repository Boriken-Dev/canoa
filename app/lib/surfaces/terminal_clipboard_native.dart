import 'package:flutter/services.dart';

import 'terminal_clipboard.dart' show ClipboardRefused;

/// A desktop or phone app reads and writes the clipboard through its engine, with no permission to ask.
const pastesByEvent = false;

Future<void> writeText(String text) async {
  try {
    await Clipboard.setData(ClipboardData(text: text));
  } on PlatformException catch (error) {
    throw ClipboardRefused('The system did not take the text onto the clipboard${_why(error)}.');
  }
}

Future<String?> readText() async {
  try {
    final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
    return text == null || text.isEmpty ? null : text;
  } on PlatformException catch (error) {
    throw ClipboardRefused('The system did not give the clipboard\'s text${_why(error)}.');
  }
}

String _why(PlatformException error) => error.message == null || error.message!.isEmpty ? '' : ' (${error.message})';

/// No paste event comes from outside the app, so the handler is never called.
VoidCallback listenPaste(bool Function(String text) handler) => () {};
