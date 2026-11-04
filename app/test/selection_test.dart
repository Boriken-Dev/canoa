// The conversation as text a person can select and copy: Ctrl+A and Ctrl+C there, dragging across messages, the context
// menu's Copy, the copy buttons on a message and a code block, and the composer's own Ctrl+A.
import 'package:canoa/chat/transcript.dart' show SelectableTranscript;
import 'package:canoa/session_store.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

/// [text] in the conversation, not in the side list or the header.
Finder _said(String text) => find.descendant(of: find.byType(SelectableTranscript), matching: find.text(text));

/// What the app put on the clipboard last.
String? _copied;

Future<void> _chord(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pumpAndSettle();
}

/// A conversation of two turns, the second answering with a code block.
Future<SessionStore> _conversation(WidgetTester tester) async {
  final store = SessionStore(includePreview: false);
  final agent = OptionsAgent();
  await pumpApp(tester, store, startAgent: agent.start, size: const Size(1280, 1000));
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await send(tester, 'First question');
  store.selected!.messages.add(ChatMessage(role: MessageRole.agent, text: 'Here:\n\n```\nfinal x = 1;\nfinal y = 2;\n```\n', at: DateTime.now()));
  await send(tester, 'Second question');
  return store;
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

  testWidgets('Ctrl+A in the conversation selects all of it, and Ctrl+C copies its text in reading order', (tester) async {
    await _conversation(tester);
    await tester.tap(_said('First question'));
    await tester.pumpAndSettle();
    await _chord(tester, LogicalKeyboardKey.keyA);
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, 'First question\n\nDone.\n\nHere:\nfinal x = 1;\nfinal y = 2;\n\nSecond question\n\nDone.');
    await finish(tester);
  });

  testWidgets('dragging selects across two messages', (tester) async {
    await _conversation(tester);
    final from = tester.getTopLeft(_said('First question')) + const Offset(1, 4);
    final to = tester.getBottomRight(_said('Done.').first) - const Offset(1, 4);
    final gesture = await tester.startGesture(from, kind: PointerDeviceKind.mouse);
    await tester.pump();
    await gesture.moveTo(to);
    await tester.pump();
    await gesture.up();
    await tester.pumpAndSettle();
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, contains('First question'));
    expect(_copied, contains('Done.'));
    await finish(tester);
  });

  testWidgets('the context menu copies what is selected and selects all', (tester) async {
    await _conversation(tester);
    final word = tester.getCenter(_said('Second question'));
    final gesture = await tester.startGesture(word, kind: PointerDeviceKind.mouse);
    await gesture.moveBy(const Offset(40, 0));
    await gesture.up();
    await tester.pumpAndSettle();
    final right = await tester.startGesture(word, kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
    await right.up();
    await tester.pumpAndSettle();
    expect(find.text('Select all'), findsOneWidget);
    await tester.tap(find.text('Copy'));
    await tester.pumpAndSettle();
    expect(_copied, isNotEmpty);
    expect('Second question', contains(_copied!.trim()));

    final again = await tester.startGesture(word, kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
    await again.up();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Select all'));
    await tester.pumpAndSettle();
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, startsWith('First question'));
    await finish(tester);
  });

  testWidgets('a message and a code block each copy with their own button, the code alone', (tester) async {
    await _conversation(tester);
    await tester.tap(find.byTooltip('Copy code'));
    await tester.pumpAndSettle();
    expect(_copied, 'final x = 1;\nfinal y = 2;');
    await tester.tap(find.byTooltip('Copy message').first);
    await tester.pumpAndSettle();
    expect(_copied, 'First question');
    await finish(tester);
  });

  testWidgets('Ctrl+A in the composer selects only what is written there', (tester) async {
    await _conversation(tester);
    final composer = find.byType(TextField).last;
    await tester.enterText(composer, 'draft words');
    await tester.tap(composer);
    await _chord(tester, LogicalKeyboardKey.keyA);
    final value = tester.widget<TextField>(composer).controller!.value;
    expect((value.selection.start, value.selection.end), (0, 'draft words'.length));
    await _chord(tester, LogicalKeyboardKey.keyC);
    expect(_copied, 'draft words');
    await finish(tester);
  });
}
