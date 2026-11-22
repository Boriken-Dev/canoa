// The transcripts of a subagent's thread and of a pop-out window are selection areas of their own: their text can be selected with
// Ctrl+A and copied with Ctrl+C, and what is copied is the transcript's text alone.
import 'dart:async';

import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SelectableTranscript;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegation_scene.dart';
import 'fixtures/popout_windows.dart';
import 'fixtures/scripted_agent.dart';

String? _copied;

Future<void> _chord(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pump(const Duration(milliseconds: 100));
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

  testWidgets('a subagent\'s thread selects all of its transcript and copies it', (tester) async {
    final scene = await openScene(tester);
    await scene.fourChildren();
    await scene.say(childUpdate('c0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k1', 'title': 'Edit share.dart', 'kind': 'edit', 'status': 'completed'}));
    await scene.settle();
    await tester.tap(find.byKey(const Key('delegation-toggle')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('child-c0')));
    await tester.pump();
    expect(scene.session.openChild, 'c0');

    final transcript = find.byType(SelectableTranscript);
    expect(transcript, findsOneWidget);
    await tester.tap(find.descendant(of: transcript, matching: find.text('Edit share.dart')));
    await tester.pump(const Duration(milliseconds: 100));
    await _chord(tester, LogicalKeyboardKey.keyA);
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, contains('Edit share.dart'));
    expect(_copied, isNot(contains('read only')), reason: 'the rest of the screen is not part of the thread\'s selection');
  });

  testWidgets('a pop-out window selects all of its transcript and copies it', (tester) async {
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    final windows = FakeWindows();
    final hub = PopoutHub(store, windows, shareInterval: const Duration(milliseconds: 10));
    addTearDown(hub.dispose);
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 150)));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);

    final transcript = find.byType(SelectableTranscript);
    expect(transcript, findsOneWidget);
    await tester.tap(find.descendant(of: transcript, matching: find.text('Fix addNumbers')));
    await tester.pump(const Duration(milliseconds: 100));
    await _chord(tester, LogicalKeyboardKey.keyA);
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, contains('Fix addNumbers'));
    expect(_copied, contains('Reading calc.js.'));
  });
}
