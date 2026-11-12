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

SessionStore _store([SessionLedger? ledger]) => SessionStore(includePreview: false, ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));

/// A new session of the stand-in, in its view.
Future<void> _draft(WidgetTester tester) async {
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/model_effort_test.dart')));

  testWidgets('before an unknown source has run, model and effort say they come after it starts', (tester) async {
    final agent = OptionsAgent();
    await pumpApp(tester, _store(), linked: agent.openLink);
    await _draft(tester);
    expect(find.text('Model: after start'), findsOneWidget);
    expect(find.text('Effort: after start'), findsOneWidget);
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
    final tip = tester.widget<PopupMenuButton<String?>>(find.byKey(const Key('option-model'))).tooltip!;
    expect(tip, startsWith('Model, as its agent offered on '));
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
    expect(find.descendant(of: find.byKey(const Key('option-model')), matching: find.text('Opus')), findsOneWidget);
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
    expect(find.descendant(of: find.byKey(const Key('option-effort')), matching: find.text('High')), findsOneWidget);
    expect(ledger.source('Scripted')?.chosen, {'effort': 'high'});

    await newSession(tester);
    expect(store.selected!.pending.values.map((choice) => (choice.role, choice.value)), [(OptionRole.effort, 'high')]);
    expect(find.descendant(of: find.byKey(const Key('option-effort')), matching: find.text('High')), findsOneWidget);
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
    expect(find.descendant(of: find.byKey(const Key('option-model')), matching: find.text('Opus')), findsOneWidget);
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
    expect(find.text('High (next turn)'), findsOneWidget);
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

}
