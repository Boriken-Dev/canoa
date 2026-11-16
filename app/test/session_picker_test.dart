// The one picker for agent, account, model, effort and mode: opened from the composer's chips, agents down the side, models with
// search, favourites and Ctrl+1..9, the efforts beside them, the mode at the bottom; a running session keeps its agent and account.
import 'dart:io' show Directory;

import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/source_choices.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _picker = Key('session-picker');

/// What the one model and effort pill reads.
String _pill(WidgetTester tester) => [
  for (final text in find.descendant(of: find.byKey(const Key('option-model')), matching: find.byType(Text)).evaluate()) (text.widget as Text).data,
].join();

Finder _inPicker(Finder finder) => find.descendant(of: find.byKey(_picker), matching: finder);

Future<void> _open(WidgetTester tester, [String chip = 'option-model']) async {
  await tester.tap(find.byKey(Key(chip)));
  await frames(tester);
}

/// A new Claude draft in its folder, with the host answering [host] for each source's models.
Future<(SessionStore, SessionLedger, FakeHost)> _draft(WidgetTester tester, {Size size = const Size(1280, 800), OptionsAgent? agent}) async {
  final ledger = SessionLedger(MemoryLedgerBackend());
  final store = SessionStore(ledger: ledger);
  final host = FakeHost();
  await pumpApp(tester, store, linked: (agent ?? OptionsAgent(modes: OptionsAgent.claudeModes)).openLink, askChoices: host.call, size: size);
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  return (store, ledger, host);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/session_picker_test.dart')));

  testWidgets('the agents are down the side, and choosing one lists its models', (tester) async {
    final (store, _, _) = await _draft(tester);
    await _open(tester);
    expect(_inPicker(find.byKey(const Key('source Claude'))), findsOneWidget);
    expect(_inPicker(find.byKey(const Key('source Codex'))), findsOneWidget);
    expect(_inPicker(find.byKey(const Key('source Scripted'))), findsOneWidget);
    expect(choiceItems('model-choice'), findsNWidgets(5), reason: 'Default and four models');
    await tester.tap(find.byKey(const Key('source Codex')));
    await frames(tester);
    expect(store.selected?.source?.label, 'Codex');
    expect(choiceItems('model-choice'), findsNWidgets(3), reason: 'Default and its two');
    expect(find.text('GPT-6 Sol'), findsOneWidget);
    expect(find.byKey(const Key('effort-choice=xhigh')), findsNothing, reason: 'the default model\'s own efforts');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    expect(find.byKey(_picker), findsNothing);
    await finish(tester);
  });

  testWidgets('a model is chosen and the picker stays for its effort, which closes it', (tester) async {
    final (store, _, _) = await _draft(tester);
    await _open(tester);
    await tester.tap(find.byKey(const ValueKey('model-choice=fable')));
    await frames(tester);
    expect(find.byKey(_picker), findsOneWidget);
    expect(_inPicker(find.descendant(of: find.byKey(const ValueKey('model-choice=fable')), matching: find.byIcon(Icons.check))), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('effort-choice=xhigh')));
    await frames(tester);
    expect(find.byKey(_picker), findsNothing);
    expect(_pill(tester), 'Fable 5.1 · Extra high');
    expect(store.selected!.pending.values.map((choice) => (choice.role.name, choice.value)), [('model', 'fable'), ('effort', 'xhigh')]);
    await finish(tester);
  });

  testWidgets('the search narrows the models from what is held, and asks the host nothing', (tester) async {
    final (_, _, host) = await _draft(tester);
    final asked = host.asked.length;
    await _open(tester);
    await tester.enterText(find.byKey(const Key('picker-search')), 'son');
    await frames(tester);
    expect(choiceItems('model-choice'), findsOneWidget);
    expect(find.byKey(const ValueKey('model-choice=sonnet')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('picker-search')), 'sonnet 5');
    await frames(tester);
    expect(choiceItems('model-choice'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('picker-search')), 'zzz');
    await frames(tester);
    expect(choiceItems('model-choice'), findsNothing);
    expect(find.byKey(const Key('picker-no-models')), findsOneWidget);
    expect(host.asked.length, asked, reason: 'typing asks nobody');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await finish(tester);
  });

  testWidgets('Enter takes the highlighted model, the arrow moves it, and Ctrl+2 takes the second at once', (tester) async {
    final (store, _, _) = await _draft(tester);
    await _open(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await frames(tester);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await frames(tester);
    expect(find.byKey(_picker), findsNothing);
    expect(store.selected!.pending.values.firstWhere((choice) => choice.role.name == 'model').value, 'opus', reason: 'Default is the first row, opus the next');

    await _open(tester);
    expect(find.byKey(const Key('shortcut 2')), findsOneWidget);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit2);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await frames(tester);
    expect(find.byKey(_picker), findsNothing);
    expect(store.selected!.pending.values.firstWhere((choice) => choice.role.name == 'model').value, 'fable');
    await finish(tester);
  });

  testWidgets('a star keeps a model under Favourites, across agents and across restarts of the ledger', (tester) async {
    final (store, ledger, _) = await _draft(tester);
    await _open(tester);
    await tester.tap(find.byKey(const Key('star sonnet')));
    await frames(tester);
    expect(ledger.isFavourite('Claude', 'sonnet'), isTrue);
    expect(ledger.encode(), contains('"favourites"'));
    await tester.tap(find.byKey(const Key('source Codex')));
    await frames(tester);
    expect(find.byKey(const ValueKey('model-choice=sonnet')), findsNothing);
    await tester.tap(find.byKey(const Key('picker-favourites')));
    await frames(tester);
    expect(choiceItems('model-choice'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('model-choice=sonnet')));
    await frames(tester);
    expect(store.selected?.source?.label, 'Claude', reason: 'a favourite of another agent brings its agent with it');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);

    final reread = SessionLedger((ledger.backend as MemoryLedgerBackend)..document = ledger.encode());
    await tester.runAsync(reread.load);
    expect(reread.favourites, [('Claude', 'sonnet')]);
    reread.toggleFavourite('Claude', 'sonnet');
    expect(reread.favourites, isEmpty);
    await finish(tester);
  });

  testWidgets('the mode is chosen at the bottom of the same picker', (tester) async {
    final (store, _, _) = await _draft(tester);
    await _open(tester);
    await tester.tap(find.byKey(const Key('picker-mode plan')));
    await frames(tester);
    expect(store.modeOf(store.selected!), 'plan');
    await tester.tap(find.byKey(const Key('picker-mode-default')));
    await frames(tester);
    expect(store.modeOf(store.selected!), isNull);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await finish(tester);
  });

  testWidgets('the source chip opens the same picker', (tester) async {
    await _draft(tester);
    await _open(tester, 'new-session-source');
    expect(find.byKey(_picker), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await finish(tester);
  });

  testWidgets('a running session keeps its agent, says so, and still takes a new effort', (tester) async {
    final agent = OptionsAgent();
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    await pumpApp(tester, store, linked: agent.openLink, askChoices: FakeHost().call);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await _open(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('source Claude'))).onTap, isNull);
    expect(tester.widget<InkWell>(find.byKey(const Key('source Scripted'))).onTap, isNotNull);
    expect(find.byKey(const Key('picker-note')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('effort-choice=high')));
    await settle(tester);
    expect(find.byKey(_picker), findsNothing);
    expect(agent.effort, 'high');
    await finish(tester);
  });

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    testWidgets('at $label width the picker draws with nothing overflowing, and closes', (tester) async {
      await _draft(tester, size: size);
      await _open(tester);
      expect(tester.takeException(), isNull);
      expect(find.byKey(const Key('picker-close')), label == 'phone' ? findsOneWidget : findsNothing);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/session_picker_$label.png'));
      if (label == 'phone') {
        await tester.tap(find.byKey(const Key('picker-close')));
        await frames(tester);
        expect(find.byKey(_picker), findsNothing);
      } else {
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      }
      await finish(tester);
    });
  }
}
