import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:xterm2/xterm.dart' show TerminalKey;

import 'terminal_clipboard.dart' show ClipboardRefused;
import 'terminal_controller.dart';

/// What a terminal's clipboard keys do, as the terminals people know do it:
///
/// - copy: Ctrl+Shift+C and Ctrl+Insert, Cmd+C on a Mac, and on Windows Ctrl+C while text is selected (with none it is the
///   interrupt, and goes to the shell);
/// - paste: Ctrl+Shift+V, Ctrl+V and Shift+Insert, Cmd+V on a Mac;
/// - select all: Ctrl+Shift+A, Cmd+A on a Mac (Ctrl+A is the shell's: the start of the line).
///
/// [say] is told, in words, what could not be done. Returns [KeyEventResult.ignored] for every other key, which the terminal then
/// sends to the shell. In a browser a paste key is left to the browser (`skipRemainingHandlers`: neither the terminal nor the page
/// consumes it), which then fires its `paste` event, the one paste that asks for no permission.
KeyEventResult terminalClipboardKey(
  KeyEvent event,
  TerminalPane pane, {
  required void Function(String problem) say,
  TargetPlatform? platform,
}) {
  if (event is KeyUpEvent) return KeyEventResult.ignored;
  final target = platform ?? defaultTargetPlatform;
  final keyboard = HardwareKeyboard.instance;
  final (ctrl, shift, alt, meta) = (keyboard.isControlPressed, keyboard.isShiftPressed, keyboard.isAltPressed, keyboard.isMetaPressed);
  if (alt) return KeyEventResult.ignored;
  final key = event.logicalKey;
  final apple = target == TargetPlatform.macOS || target == TargetPlatform.iOS;

  final bool copy;
  final bool paste;
  final bool all;
  var interrupt = false;
  var sendsInterrupt = false;
  if (apple) {
    final command = meta && !ctrl && !shift;
    copy = command && key == LogicalKeyboardKey.keyC;
    paste = command && key == LogicalKeyboardKey.keyV;
    all = command && key == LogicalKeyboardKey.keyA;
  } else {
    final chord = ctrl && shift && !meta;
    final plain = ctrl && !shift && !meta;
    copy = chord && key == LogicalKeyboardKey.keyC || plain && key == LogicalKeyboardKey.insert;
    paste = chord && key == LogicalKeyboardKey.keyV || plain && key == LogicalKeyboardKey.keyV || shift && !ctrl && !meta && key == LogicalKeyboardKey.insert;
    all = chord && key == LogicalKeyboardKey.keyA;
    // Windows Terminal: Ctrl+C copies what is selected, so a selection is what tells a copy from an interrupt.
    interrupt = target == TargetPlatform.windows && plain && key == LogicalKeyboardKey.keyC && pane.selection != null;
    // Elsewhere Ctrl+C is the interrupt even with a selection; the terminal view would copy it, so the key is sent from here.
    sendsInterrupt = target != TargetPlatform.windows && plain && key == LogicalKeyboardKey.keyC && pane.selection != null;
  }

  if (copy || interrupt) {
    if (event is KeyDownEvent) _copy(pane, say, clear: interrupt);
    return KeyEventResult.handled;
  }
  if (sendsInterrupt) {
    pane.terminal.keyInput(TerminalKey.keyC, ctrl: true);
    return KeyEventResult.handled;
  }
  if (all) {
    if (event is KeyDownEvent) pane.selectAll();
    return KeyEventResult.handled;
  }
  if (paste) {
    if (pane.clipboard.pastesByEvent) return KeyEventResult.skipRemainingHandlers;
    pane.pasteClipboard().then((problem) {
      if (problem != null) say(problem);
    });
    return KeyEventResult.handled;
  }
  return KeyEventResult.ignored;
}

void _copy(TerminalPane pane, void Function(String problem) say, {required bool clear}) {
  pane.copy(clear: clear).then((_) {}, onError: (Object error) => say(error is ClipboardRefused ? error.message : 'The text could not be copied.'));
}
