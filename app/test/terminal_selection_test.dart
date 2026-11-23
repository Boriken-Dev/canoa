// The terminal inside a screen that is one selection area: selecting and copying inside the terminal works with its own cells and keys,
// Select all of the screen does not take the shell's text, and a drag that starts in the screen's text does not run into the terminal.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/selectable.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/surfaces/terminal_clipboard.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/gestures.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Shell implements RemoteTerminal {
  @override
  String get folder => '/work';
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();

  void print(String text) => _out.add(utf8.encode(text));

  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => _exit.future;
  @override
  Future<void> write(String text) async {}
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

final class _Clip implements TerminalClipboard {
  final written = <String>[];

  @override
  Future<void> write(String value) async => written.add(value);
  @override
  Future<String?> read() async => null;
  @override
  VoidCallback onPaste(bool Function(String text) handler) => () {};
  @override
  bool get pastesByEvent => false;
}

/// What the screen's own selection area last put on the system clipboard.
String? _copied;

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _chord(WidgetTester tester, List<LogicalKeyboardKey> mods, LogicalKeyboardKey key) async {
  for (final mod in mods) {
    await tester.sendKeyDownEvent(mod);
  }
  await tester.sendKeyEvent(key);
  for (final mod in mods.reversed) {
    await tester.sendKeyUpEvent(mod);
  }
  await tester.pumpAndSettle();
}

void main() {
  setUp(() {
    _copied = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') _copied = (call.arguments as Map)['text'] as String?;
      if (call.method == 'Clipboard.getData') return {'text': _copied};
      return null;
    });
  });
  tearDown(() => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));

  testWidgets('the terminal selects and copies its own cells, and the screen around it keeps its own text apart', (tester) async {
    final host = _Host();
    final clip = _Clip();
    final workspace = TerminalWorkspace(host: host, cwd: '/work', clipboard: clip)..newTab();
    addTearDown(() => tester.runAsync(workspace.release));
    await tester.binding.setSurfaceSize(const Size(900, 420));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: Selectable(
          child: Column(children: [
            const Padding(padding: EdgeInsets.all(8), child: Text('text beside the shell')),
            Expanded(child: TerminalPanel(workspace: workspace, focusOnMount: true)),
          ]),
        ),
      ),
    ));
    await tester.pump(const Duration(milliseconds: 50));
    await _settle(tester);
    host.opened.single.print('shell words here');
    await _settle(tester);

    // Inside the terminal: the cells the person selected are what its copy key puts on the clipboard.
    final pane = workspace.focused!;
    final buffer = pane.terminal.buffer;
    pane.view.setSelection(buffer.createAnchor(0, 0), buffer.createAnchor(11, 0));
    await tester.pump();
    await _chord(tester, [LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.shiftLeft], LogicalKeyboardKey.keyC);
    expect(clip.written, ['shell words']);

    // In the screen: a drag over its text and Select all copy that text and none of the shell's.
    final beside = find.text('text beside the shell');
    final gesture = await tester.startGesture(tester.getTopLeft(beside) + const Offset(1, 4), kind: PointerDeviceKind.mouse);
    await tester.pump();
    await gesture.moveTo(tester.getBottomRight(beside) - const Offset(1, 4));
    await tester.pump();
    await gesture.up();
    await tester.pumpAndSettle();
    await _chord(tester, [LogicalKeyboardKey.controlLeft], LogicalKeyboardKey.keyA);
    await _chord(tester, [LogicalKeyboardKey.controlLeft], LogicalKeyboardKey.keyC);
    expect(_copied, contains('text beside the shell'));
    expect(_copied, isNot(contains('shell words')), reason: 'the shell\'s output is not part of the screen\'s selection');
  });
}
