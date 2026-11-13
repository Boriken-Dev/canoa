// Any text in the app can be selected with the mouse and copied: in a message, in a tool call, in the side list, in a
// dialog, in a page over the screen, and the controls beside that text still work.
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';

/// [text] in the conversation, not in the side list, which shows a session by its first message.
Finder _said(String text) => find.descendant(of: find.byType(SelectableTranscript), matching: find.text(text));

/// What the app put on the clipboard last.
String? _copied;

Future<void> _chord(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pumpAndSettle();
}

/// Drags the mouse over all of [text]'s first match, then copies with Ctrl+C.
Future<String?> _dragAndCopy(WidgetTester tester, Finder text) async {
  _copied = null;
  final box = tester.getRect(text.first);
  final gesture = await tester.startGesture(box.centerLeft - const Offset(2, 0), kind: PointerDeviceKind.mouse);
  await tester.pump();
  // Many small moves, to a point past the text: one jump selects only as far as the first update reaches.
  for (var step = 1; step <= 12; step++) {
    await gesture.moveTo(box.centerLeft + Offset(box.width * step / 10 + 4, 0));
    await tester.pump();
  }
  await gesture.up();
  await tester.pumpAndSettle();
  await _chord(tester, LogicalKeyboardKey.keyC);
  return _copied;
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

  Future<SessionStore> conversation(WidgetTester tester) async {
    final store = SessionStore();
    final agent = OptionsAgent();
    await pumpApp(tester, store, linked: agent.openLink, size: const Size(1280, 1000));
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await send(tester, 'What does the build say');
    final tool = ToolCall('t1')
      ..apply({
        'sessionUpdate': 'tool_call',
        'title': 'Run the build',
        'content': [
          {'type': 'content', 'content': {'type': 'text', 'text': 'compiled 12 files'}},
        ],
      }, chatLimits);
    store.selected!.messages.add(ChatMessage(role: MessageRole.tool, text: '', at: DateTime.now(), tool: tool));
    await send(tester, 'Next question');
    return store;
  }

  testWidgets('a message and a tool call select and copy', (tester) async {
    await conversation(tester);
    expect(await _dragAndCopy(tester, _said('What does the build say')), 'What does the build say');
    expect(await _dragAndCopy(tester, _said('Run the build')), 'Run the build');
    await finish(tester);
  });

  testWidgets('the side list and the header select and copy, beside their buttons', (tester) async {
    final store = await conversation(tester);
    final titled = find.text(store.selected!.title);
    expect(titled, findsWidgets);
    expect(await _dragAndCopy(tester, titled.first), isNotEmpty);
    // The button next to selectable text still works.
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a dialog selects and copies, and Select all in it stays in the dialog', (tester) async {
    await conversation(tester);
    await _chord(tester, LogicalKeyboardKey.slash);
    await frames(tester);
    expect(await _dragAndCopy(tester, find.text('Keyboard shortcuts')), 'Keyboard shortcuts');

    await _chord(tester, LogicalKeyboardKey.keyA);
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, contains('Keyboard shortcuts'));
    expect(_copied, isNot(contains('What does the build say')), reason: 'the screen under a dialog is not part of it');

    // Its controls work: the dropdown opens and closing the dialog works.
    await tester.tap(find.text('Close'));
    await frames(tester);
    expect(find.text('Keyboard shortcuts'), findsNothing);
    await finish(tester);
  });

  testWidgets('a tap outside a dialog closes it once, a tap on its text does not, a drag does not', (tester) async {
    await conversation(tester);
    await _chord(tester, LogicalKeyboardKey.slash);
    await frames(tester);
    await tester.tap(find.text('Open Settings'));
    await frames(tester);
    expect(find.text('Keyboard shortcuts'), findsOneWidget);
    await tester.dragFrom(const Offset(4, 4), const Offset(200, 200), kind: PointerDeviceKind.mouse);
    await frames(tester);
    expect(find.text('Keyboard shortcuts'), findsOneWidget, reason: 'a drag is not a tap');
    await tester.tapAt(const Offset(4, 4));
    await frames(tester);
    expect(find.text('Keyboard shortcuts'), findsNothing);
    expect(find.byTooltip('Canoa settings'), findsOneWidget, reason: 'the screen under it stays');
    await finish(tester);
  });

  testWidgets('a page over the screen selects and copies', (tester) async {
    await tester.pumpWidget(MaterialApp(
      home: Builder(builder: (context) => TextButton(
        onPressed: () => showLoadout(context, config: const CanoaConfig(), home: MemoryHome(MemoryOverlays()), onSave: (changed) async => changed),
        child: const Text('open Loadout'),
      )),
    ));
    await tester.tap(find.text('open Loadout'));
    await tester.pumpAndSettle();
    expect(await _dragAndCopy(tester, find.text('Loadout')), 'Loadout');
    await tester.tap(find.byKey(const Key('overlays-discover')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('catalogs-empty')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('store-catalog-source')), 'lint');
    expect(find.text('lint'), findsOneWidget, reason: 'the field still takes text');
  });

  testWidgets('the composer takes text and sends after a selection, and a copy button still copies', (tester) async {
    await conversation(tester);
    await _dragAndCopy(tester, _said('What does the build say'));
    await send(tester, 'And now the tests');
    expect(find.text('And now the tests'), findsWidgets);
    await tester.tap(find.byTooltip('Copy message').first);
    await tester.pumpAndSettle();
    expect(_copied, 'What does the build say');
    await finish(tester);
  });
}
