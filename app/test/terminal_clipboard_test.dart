// Copy and paste in the terminal as the app draws it: the keys each platform's terminals use, the right-click menu, what the shell is
// sent (one write, line ends as Enter, bracketed when the shell asked), the interrupt that stays an interrupt, and, behind the
// clipboard seam, the way a browser delivers a paste.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/surfaces/terminal_clipboard.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Shell implements RemoteTerminal {
  @override
  String get folder => '/work';
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  final writes = <String>[];

  void print(String text) => _out.add(utf8.encode(text));

  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => _exit.future;
  @override
  Future<void> write(String text) async => writes.add(text);
  @override
  Future<void> resize(int cols, int rows) async {}
  @override
  Future<void> close() async {
    if (!_exit.isCompleted) _exit.complete(null);
    await _out.close();
  }
}

final class _Host implements TerminalHost {
  final opened = <_Shell>[];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final made = _Shell();
    opened.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

/// A clipboard the test holds: what it was given, how often it was read, and, when [byEvent], a browser's paste events.
final class _Clip implements TerminalClipboard {
  _Clip({this.text, this.byEvent = false, this.refuse});

  String? text;
  final bool byEvent;

  /// The words of a refusal to give instead of the text.
  String? refuse;
  final written = <String>[];
  var reads = 0;
  final _handlers = <bool Function(String)>[];

  @override
  Future<void> write(String value) async {
    if (refuse case final why?) throw ClipboardRefused(why);
    written.add(value);
    text = value;
  }

  @override
  Future<String?> read() async {
    reads++;
    if (refuse case final why?) throw ClipboardRefused(why);
    return text;
  }

  @override
  VoidCallback onPaste(bool Function(String text) handler) {
    _handlers.add(handler);
    return () => _handlers.remove(handler);
  }

  /// A paste event the browser delivers; whether a terminal took it.
  bool deliver(String value) => [..._handlers].any((handler) => handler(value));

  @override
  bool get pastesByEvent => byEvent;
}

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

class _Rig {
  _Rig(this.host, this.workspace);

  final _Host host;
  final TerminalWorkspace workspace;

  _Shell get shell => host.opened.single;
  TerminalPane get pane => workspace.focused!;
}

/// One shell on screen, with the keyboard, over [clip].
Future<_Rig> _rig(WidgetTester tester, TerminalClipboard clip) async {
  final host = _Host();
  final workspace = TerminalWorkspace(host: host, cwd: '/work', clipboard: clip)..newTab();
  addTearDown(() => tester.runAsync(workspace.release));
  await tester.binding.setSurfaceSize(const Size(900, 420));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(body: TerminalPanel(workspace: workspace, focusOnMount: true)),
  ));
  await tester.pump(const Duration(milliseconds: 50));
  await _settle(tester);
  expect(workspace.focused!.focusNode.hasPrimaryFocus, isTrue, reason: 'the shell has the keyboard');
  return _Rig(host, workspace);
}

/// The shell printed [text] and the first [length] cells of its first line are selected.
Future<void> _select(WidgetTester tester, _Rig rig, String text, int length) async {
  rig.shell.print(text);
  await _settle(tester);
  final buffer = rig.pane.terminal.buffer;
  rig.pane.view.setSelection(buffer.createAnchor(0, 0), buffer.createAnchor(length, 0));
  await tester.pump();
  expect(rig.pane.selection, isNotNull);
}

/// [key] pressed with [mods] held, as [os] reports it.
Future<void> _chord(WidgetTester tester, String os, List<LogicalKeyboardKey> mods, LogicalKeyboardKey key) async {
  for (final mod in mods) {
    await tester.sendKeyDownEvent(mod, platform: os);
  }
  await tester.sendKeyEvent(key, platform: os);
  for (final mod in mods.reversed) {
    await tester.sendKeyUpEvent(mod, platform: os);
  }
  await _settle(tester);
}

const _ctrl = LogicalKeyboardKey.controlLeft;
const _shift = LogicalKeyboardKey.shiftLeft;
const _meta = LogicalKeyboardKey.metaLeft;

void _on(String name, TargetPlatform platform, Future<void> Function(WidgetTester tester, String os) body) {
  final os = switch (platform) {
    TargetPlatform.windows => 'windows',
    TargetPlatform.macOS => 'macos',
    _ => 'linux',
  };
  testWidgets('$name [$os]', (tester) => body(tester, os), variant: TargetPlatformVariant.only(platform));
}

void main() {
  group('paste', () {
    for (final (platform, chords) in [
      (TargetPlatform.windows, [('Ctrl+Shift+V', [_ctrl, _shift], LogicalKeyboardKey.keyV), ('Ctrl+V', [_ctrl], LogicalKeyboardKey.keyV), ('Shift+Insert', [_shift], LogicalKeyboardKey.insert)]),
      (TargetPlatform.linux, [('Ctrl+Shift+V', [_ctrl, _shift], LogicalKeyboardKey.keyV), ('Ctrl+V', [_ctrl], LogicalKeyboardKey.keyV), ('Shift+Insert', [_shift], LogicalKeyboardKey.insert)]),
      (TargetPlatform.macOS, [('Cmd+V', [_meta], LogicalKeyboardKey.keyV)]),
    ]) {
      for (final (name, mods, key) in chords) {
        _on('$name sends the clipboard to the shell as one write, line ends as Enter', platform, (tester, os) async {
          final clip = _Clip(text: 'echo one\r\necho two\necho three');
          final rig = await _rig(tester, clip);
          await _chord(tester, os, mods, key);
          expect(rig.shell.writes, ['echo one\recho two\recho three']);
          expect(clip.reads, 1);
        });
      }
    }

    _on('a shell that turned bracketed paste on gets the markers, and the text cannot end the paste early', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip(text: 'a\nb\tc\x1b[201~rm -rf /\x1b[31mred\x03');
      final rig = await _rig(tester, clip);
      rig.shell.print('\x1b[?2004h');
      await _settle(tester);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyV);
      expect(rig.shell.writes, ['\x1b[200~a\rb\tcrm -rf /red \x1b[201~']);

      rig.shell.print('\x1b[?2004l');
      await _settle(tester);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyV);
      expect(rig.shell.writes.last, 'a\rb\tcrm -rf /red ', reason: 'with the mode off, ordinary text and line breaks arrive as copied and escape sequences do not');
    });

    _on('a large multi-line paste is sent whole: nothing here caps or asks', TargetPlatform.linux, (tester, os) async {
      final big = List.generate(20000, (i) => 'line $i of a long paste').join('\n');
      final clip = _Clip(text: big);
      final rig = await _rig(tester, clip);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyV);
      expect(rig.shell.writes, hasLength(1));
      expect(rig.shell.writes.single, big.replaceAll('\n', '\r'));
    });

    _on('an empty clipboard types nothing and says so', TargetPlatform.windows, (tester, os) async {
      final rig = await _rig(tester, _Clip());
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyV);
      expect(rig.shell.writes, isEmpty);
      expect(find.text('The clipboard holds no text.'), findsOneWidget);
    });
  });

  group('copy', () {
    for (final (platform, chords) in [
      (TargetPlatform.windows, [('Ctrl+Shift+C', [_ctrl, _shift], LogicalKeyboardKey.keyC), ('Ctrl+Insert', [_ctrl], LogicalKeyboardKey.insert)]),
      (TargetPlatform.linux, [('Ctrl+Shift+C', [_ctrl, _shift], LogicalKeyboardKey.keyC), ('Ctrl+Insert', [_ctrl], LogicalKeyboardKey.insert)]),
      (TargetPlatform.macOS, [('Cmd+C', [_meta], LogicalKeyboardKey.keyC)]),
    ]) {
      for (final (name, mods, key) in chords) {
        _on('$name copies the selection, keeps it, and sends nothing to the shell', platform, (tester, os) async {
          final clip = _Clip();
          final rig = await _rig(tester, clip);
          await _select(tester, rig, 'build ok\r\n', 8);
          await _chord(tester, os, mods, key);
          expect(clip.written, ['build ok']);
          expect(rig.pane.selection, 'build ok', reason: 'the selection stays, as in the terminals that copy with a chord');
          expect(rig.shell.writes, isEmpty);
        });
      }
    }

    _on('Ctrl+C with a selection copies it and clears it; the next Ctrl+C interrupts', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip();
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(clip.written, ['build ok']);
      expect(rig.shell.writes, isEmpty, reason: 'a copy is not an interrupt');
      expect(rig.pane.selection, isNull);

      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(rig.shell.writes, ['\x03']);
      expect(clip.written, hasLength(1));
    });

    _on('Ctrl+C with nothing selected is the interrupt', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip();
      final rig = await _rig(tester, clip);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(rig.shell.writes, ['\x03']);
      expect(clip.written, isEmpty);
    });

    _on('Ctrl+C is the interrupt where the terminals do not copy with it, selection or not', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip();
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(rig.shell.writes, ['\x03']);
      expect(clip.written, isEmpty);
    });

    _on('Ctrl+C is the interrupt on a Mac, whose copy is Cmd+C', TargetPlatform.macOS, (tester, os) async {
      final clip = _Clip();
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(rig.shell.writes, ['\x03']);
      expect(clip.written, isEmpty);
    });

    _on('a copy with nothing selected does nothing', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip();
      final rig = await _rig(tester, clip);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyC);
      expect(clip.written, isEmpty);
      expect(rig.shell.writes, isEmpty);
    });

    _on('a refused copy says why', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip(refuse: 'The system did not take the text.');
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyC);
      expect(find.text('The system did not take the text.'), findsOneWidget);
    });
  });

  group('select all', () {
    _on('Ctrl+A stays the shell\'s (start of line); Ctrl+Shift+A selects the screen', TargetPlatform.linux, (tester, os) async {
      final rig = await _rig(tester, _Clip());
      rig.shell.print('one\r\ntwo\r\n');
      await _settle(tester);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyA);
      expect(rig.shell.writes, ['\x01']);
      expect(rig.pane.selection, isNull);

      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyA);
      expect(rig.pane.selection, startsWith('one\ntwo'));
    });
  });

  group('the right-click menu', () {
    Future<void> rightClick(WidgetTester tester) async {
      final mouse = await tester.startGesture(const Offset(450, 200), kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
      await mouse.up();
      await tester.pumpAndSettle();
    }

    _on('has Copy, Paste and Select all; Paste sends the clipboard, Copy takes the selection', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip(text: 'ls -l\n');
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);

      await rightClick(tester);
      expect(tester.widget<PopupMenuItem<String>>(_key('terminal context copy')).enabled, isTrue);
      await tester.tap(_key('terminal context copy'));
      await tester.pumpAndSettle();
      expect(clip.written, ['build ok']);

      await rightClick(tester);
      await tester.tap(_key('terminal context paste'));
      await tester.pumpAndSettle();
      await _settle(tester);
      expect(rig.shell.writes, ['build ok'], reason: 'the clipboard now holds what Copy put there');

      await rightClick(tester);
      await tester.tap(_key('terminal context select all'));
      await tester.pumpAndSettle();
      expect(rig.pane.selection, startsWith('build ok'));
    });

    _on('Copy is off with nothing selected', TargetPlatform.linux, (tester, os) async {
      await _rig(tester, _Clip());
      await rightClick(tester);
      expect(tester.widget<PopupMenuItem<String>>(_key('terminal context copy')).enabled, isFalse);
    });

    _on('a refused Paste says why, in words', TargetPlatform.linux, (tester, os) async {
      final rig = await _rig(tester, _Clip(refuse: 'The browser did not let this page read the clipboard. Paste with the keyboard instead.'));
      await rightClick(tester);
      await tester.tap(_key('terminal context paste'));
      await tester.pumpAndSettle();
      expect(find.textContaining('The browser did not let this page read the clipboard'), findsOneWidget);
      expect(rig.shell.writes, isEmpty);
    });

    _on('the toolbar has Copy and Paste too', TargetPlatform.linux, (tester, os) async {
      final clip = _Clip(text: 'pwd');
      final rig = await _rig(tester, clip);
      expect(tester.widget<IconButton>(_key('terminal copy')).onPressed, isNull, reason: 'nothing is selected');
      await tester.tap(_key('terminal paste'));
      await _settle(tester);
      expect(rig.shell.writes, ['pwd']);
      await _select(tester, rig, 'build ok\r\n', 8);
      await tester.tap(_key('terminal copy'));
      await _settle(tester);
      expect(clip.written, ['build ok']);
    });
  });

  group('in a browser, where a keyboard paste arrives as the browser\'s paste event', () {
    _on('the paste key is left to the browser: nothing is read and nothing is sent until the event comes', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip(text: 'never read', byEvent: true);
      final rig = await _rig(tester, clip);
      for (final (mods, key) in [([_ctrl], LogicalKeyboardKey.keyV), ([_ctrl, _shift], LogicalKeyboardKey.keyV), ([_shift], LogicalKeyboardKey.insert)]) {
        await _chord(tester, os, mods, key);
      }
      expect(clip.reads, 0, reason: 'reading the clipboard from a page asks the person\'s permission');
      expect(rig.shell.writes, isEmpty, reason: 'the shell must not get a ^V either');

      expect(clip.deliver('echo hi\n'), isTrue);
      await _settle(tester);
      expect(rig.shell.writes, ['echo hi\r']);
    });

    _on('only the shell that has the keyboard takes the event', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip(byEvent: true);
      final rig = await _rig(tester, clip);
      rig.pane.focusNode.unfocus();
      await tester.pump();
      expect(clip.deliver('nobody listens'), isFalse, reason: 'the page keeps a paste no terminal has the keyboard for');
      await _settle(tester);
      expect(rig.shell.writes, isEmpty);
    });

    _on('the menu\'s Paste reads the clipboard, which is the one place a browser asks', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip(text: 'from the menu', byEvent: true);
      final rig = await _rig(tester, clip);
      final mouse = await tester.startGesture(const Offset(450, 200), kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
      await mouse.up();
      await tester.pumpAndSettle();
      await tester.tap(_key('terminal context paste'));
      await tester.pumpAndSettle();
      await _settle(tester);
      expect(clip.reads, 1);
      expect(rig.shell.writes, ['from the menu']);
    });

    _on('Copy still writes from the key, so the browser takes it as the person\'s own gesture', TargetPlatform.windows, (tester, os) async {
      final clip = _Clip(byEvent: true);
      final rig = await _rig(tester, clip);
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl], LogicalKeyboardKey.keyC);
      expect(clip.written, ['build ok']);
    });
  });

  group('the system clipboard, through the engine\'s channel', () {
    final held = <String, String>{};

    setUp(() {
      held.clear();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
        switch (call.method) {
          case 'Clipboard.setData':
            held['text'] = (call.arguments as Map)['text'] as String;
            return null;
          case 'Clipboard.getData':
            return held.containsKey('text') ? <String, dynamic>{'text': held['text']} : null;
        }
        return null;
      });
    });
    tearDown(() => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));

    _on('a selection copied with Ctrl+Shift+C is what Ctrl+Shift+V then sends', TargetPlatform.linux, (tester, os) async {
      final rig = await _rig(tester, const SystemTerminalClipboard());
      await _select(tester, rig, 'build ok\r\n', 8);
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyC);
      expect(held['text'], 'build ok');
      await _chord(tester, os, [_ctrl, _shift], LogicalKeyboardKey.keyV);
      expect(rig.shell.writes, ['build ok']);
    });
  });
}
