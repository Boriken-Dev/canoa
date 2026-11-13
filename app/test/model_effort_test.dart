// Model, effort and mode beside Send, against a stand-in agent that offers them as config options and refuses one value:
// what shows before the agent starts, what reaches it before the first prompt, a change while it runs, a refusal, and a
// change during a turn.
import 'dart:io' show Directory;

import 'package:canoa/chat/session_options.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

SessionStore _store([SessionLedger? ledger]) => SessionStore(ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));

/// A new session of the stand-in, in its view.
Future<void> _draft(WidgetTester tester) async {
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
}

/// What the one model and effort pill reads.
String _pill(WidgetTester tester) => [
  for (final text in find.descendant(of: find.byKey(const Key('option-model')), matching: find.byType(Text)).evaluate()) (text.widget as Text).data,
].join();

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/model_effort_test.dart')));

  testWidgets('before an unknown source has run, model and effort say they come after it starts', (tester) async {
    final agent = OptionsAgent();
    await pumpApp(tester, _store(), linked: agent.openLink);
    await _draft(tester);
    expect(find.text('Model: after start'), findsOneWidget);
    expect(find.text('Effort: after start'), findsNothing, reason: 'one pill for both');
    expect(agent.starts, isEmpty);
    await finish(tester);
  });

  testWidgets('what the source offered last time shows before it starts, with when it was seen', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final agent = OptionsAgent();
    final store = _store(ledger);
    await pumpApp(tester, store, linked: agent.openLink);
    await _draft(tester);
    await send(tester, 'Say hello');
    expect(ledger.source('Scripted')?.options.map((option) => option.id), ['mode', 'model', 'effort']);

    await newSession(tester);
    expect(store.selected?.isDraft, isTrue);
    expect(await menuOf(tester, 'option-model'), ['Default', 'Default model', 'Opus', 'Refused']);
    expect(await menuOf(tester, 'option-effort'), ['Default', 'Low', 'High']);
    expect(await menuOf(tester, 'option-mode'), ['Default', 'Ask first', 'Accept edits', 'Bypass']);
    final tip = tester.widget<Tooltip>(find.descendant(of: find.byKey(const Key('option-model')), matching: find.byType(Tooltip))).message!;
    expect(tip, startsWith('Model and effort, as its agent offered on '));
    expect(agent.starts, hasLength(1), reason: 'the new session has started nothing');
    await finish(tester);
  });

  testWidgets('the chosen model and effort reach the agent before the first prompt', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true);
    final agent = OptionsAgent();
    final store = _store(ledger);
    await pumpApp(tester, store, linked: agent.openLink);
    await _draft(tester);
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-effort', 'High');
    expect(agent.starts, isEmpty);
    await send(tester, 'Fix the tests');

    expect(agent.methods, ['initialize', 'session/new', 'session/set_config_option', 'session/set_config_option', 'session/prompt']);
    final set = [for (final request in agent.requests) if (request['method'] == 'session/set_config_option') request['params']];
    expect(set.map((params) => (params['configId'], params['value'])), [('model', 'opus'), ('effort', 'high')]);
    expect(_pill(tester), 'Opus · High');
    expect(ledger.source('Scripted')?.chosen, {'model': 'opus', 'effort': 'high'}, reason: 'choices the agent took are the source\'s last');
    await finish(tester);
  });

  testWidgets('a change after it starts goes to the agent, shows, and is the source\'s default next time', (tester) async {
    final agent = OptionsAgent();
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = _store(ledger);
    await pumpApp(tester, store, linked: agent.openLink);
    await _draft(tester);
    await send(tester, 'Say hello');
    await pick(tester, 'option-effort', 'High');
    expect(agent.effort, 'high');
    expect(agent.last('session/set_config_option')['params'], {'sessionId': 'o1', 'configId': 'effort', 'value': 'high'});
    expect(_pill(tester), endsWith('High'));
    expect(ledger.source('Scripted')?.chosen, {'effort': 'high'});

    await newSession(tester);
    expect(store.selected!.pending.values.map((choice) => (choice.role, choice.value)), [(OptionRole.effort, 'high')]);
    expect(_pill(tester), endsWith('High'));
    await finish(tester);
  });

  testWidgets('a value the agent refuses goes back, with its reason', (tester) async {
    final agent = OptionsAgent();
    await pumpApp(tester, _store(), linked: agent.openLink);
    await _draft(tester);
    await send(tester, 'Say hello');
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-model', 'Refused');
    expect(agent.model, 'opus');
    expect(_pill(tester), startsWith('Opus'));
    expect(find.text('The agent did not change Model to Refused: That model is not available'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a change during a turn waits for the next one', (tester) async {
    final agent = OptionsAgent()..holdTurns = true;
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink);
    await _draft(tester);
    await send(tester, 'Take your time');
    expect(store.selected?.status, SessionStatus.working);
    await pick(tester, 'option-effort', 'High');
    expect(agent.methods.where((method) => method == 'session/set_config_option'), isEmpty);
    expect(_pill(tester), endsWith('High (next turn)'));
    agent.finishTurn();
    await settle(tester);
    expect(agent.effort, 'low', reason: 'nothing goes until the next message');
    agent.holdTurns = false;
    await send(tester, 'Now quickly');
    final tail = agent.methods.sublist(agent.methods.length - 2);
    expect(tail, ['session/set_config_option', 'session/prompt']);
    expect(agent.effort, 'high');
    await finish(tester);
  });

  group('one pill for model and effort', () {
    Future<OptionsAgent> claudeDraft(WidgetTester tester, {Size? size}) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(), linked: agent.openLink, size: size ?? const Size(1280, 800));
      await newSession(tester);
      await chooseSource(tester, 'Claude');
      await openFolder(tester);
      return agent;
    }

    testWidgets('model and effort read as one pill, and effort is a submenu of the model menu', (tester) async {
      final agent = OptionsAgent();
      await pumpApp(tester, _store(), linked: agent.openLink);
      await _draft(tester);
      await send(tester, 'Say hello');
      await pick(tester, 'option-model', 'Opus');
      await pick(tester, 'option-effort', 'High');
      expect(_pill(tester), 'Opus \u00b7 High');
      expect(find.byKey(const Key('option-model')), findsOneWidget);
      expect(find.text('Effort'), findsNothing, reason: 'no pill of its own');
      await tester.tap(find.byKey(const Key('option-model')));
      await frames(tester);
      expect(find.byKey(const Key('option-effort')), findsOneWidget, reason: 'the submenu is in the model menu');
      expect(find.descendant(of: find.byKey(const Key('option-effort')), matching: find.text('High')), findsOneWidget, reason: 'and says what it is');
      expect(choiceItems('effort-choice'), findsNothing, reason: 'its choices come when it opens');
      await tester.tap(find.byKey(const Key('option-effort')));
      await frames(tester);
      expect(choiceItems('effort-choice'), findsNWidgets(2));
      await tester.tapAt(const Offset(2, 2));
      await frames(tester);
      await finish(tester);
    });

    testWidgets('before the agent starts the pill says what the source defaults to, not Default twice', (tester) async {
      await claudeDraft(tester);
      expect(_pill(tester), 'Opus 5.5');
      expect(_pill(tester), isNot(contains('Default')));
      await finish(tester);
    });

    testWidgets('a source that knows its default effort shows it, and the effort follows the model chosen', (tester) async {
      final agent = OptionsAgent();
      await pumpApp(tester, _store(), linked: agent.openLink);
      await newSession(tester);
      await chooseSource(tester, 'Codex');
      await openFolder(tester);
      expect(_pill(tester), 'GPT-6 Luna \u00b7 Medium');
      await pick(tester, 'option-model', 'GPT-6 Sol');
      expect(_pill(tester), 'GPT-6 Sol \u00b7 High');
      await finish(tester);
    });

    for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
      testWidgets('the pill with its menu and the effort submenu open at $label width', (tester) async {
        await claudeDraft(tester, size: size);
        await tester.tap(find.byKey(const Key('option-model')));
        await frames(tester);
        await tester.tap(find.byKey(const Key('option-effort')));
        await frames(tester);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/model_pill_$label.png'));
        await tester.tapAt(const Offset(2, 2));
        await frames(tester);
        await finish(tester);
      });
    }
  });

  group('the mode menu', () {
    testWidgets('says what it is for, and gives each mode a line on what it does', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(), linked: agent.openLink, size: const Size(1280, 800));
      await newSession(tester);
      await chooseSource(tester, 'Claude');
      await openFolder(tester);
      await tester.tap(find.byKey(const Key('option-mode')));
      await frames(tester);
      expect(find.descendant(of: find.byKey(const Key('mode-header')), matching: find.text('Permissions \u00b7 this session')), findsOneWidget);
      for (final entry in const {
        'Manual': 'Asks before each change or command',
        'Accept edits': 'Changes files without asking; commands still ask',
        'Auto': 'Lets the agent decide what needs asking',
        'Plan': 'Reads and plans; makes no changes',
        'Bypass': 'Asks for nothing',
      }.entries) {
        expect(find.text(entry.key), findsWidgets);
        expect(find.text(entry.value), findsOneWidget, reason: entry.key);
      }
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mode_menu_desktop.png'));
      await tester.tapAt(const Offset(2, 2));
      await frames(tester);
      await finish(tester);
    });
  });
}
