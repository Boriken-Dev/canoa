// Moving a running conversation to another account: the agent is started again for that account and opens the same conversation,
// with its model, effort and mode, and the transcript says so in one line. A kind of account whose conversations do not move,
// a turn in progress and a start that fails leave it where it was, and the picker says why.
import 'dart:convert';
import 'dart:io' show Directory;


import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['one', 'two', 'login']},
    },
  },
);

/// Claude's catalog with the API key kind marked as one whose conversations move to another account, as the provider marks it.
Map<String, Object?> _catalog() {
  final catalog = jsonDecode(jsonEncode(claudeCatalog)) as Map<String, Object?>;
  for (final kind in catalog['kinds'] as List) {
    if ((kind as Map)['id'] == 'anthropic-api-key') kind['history_portable'] = true;
  }
  return catalog;
}

Object? _asked(Map<String, dynamic> initialize) => ((initialize['params'] as Map)['_meta'] as Map)[accountMeta];

/// A Claude session running on [first] after its first message, with the host's accounts one, two (API keys) and login.
Future<(SessionStore, OptionsAgent, ChatSession)> _running(WidgetTester tester, {String first = 'one', bool hold = false}) async {
  final agent = OptionsAgent(modes: OptionsAgent.claudeModes)
    ..resetsChoicesOnLoad = true
    ..holdTurns = hold;
  final host = FakeAccountsHost(accounts: [account('one'), account('two'), account('login', kind: 'claude-login')])
    ..catalogs['claude'] = _catalog();
  final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
  await pumpApp(tester, store, linked: agent.openLink, config: _config, accountsHost: host);
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  await settle(tester);
  if (first != 'one') {
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    await tester.tap(find.byKey(Key('account-choice $first')));
    await frames(tester);
    await closePicker(tester);
  }
  await send(tester, 'Say hello');
  await settle(tester, 150);
  return (store, agent, store.selected!);
}

Future<void> _openPicker(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('session-account')));
  await frames(tester);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/account_switch_test.dart')));

  testWidgets('a running session goes on under another account: the same conversation, its model and effort, one line', (tester) async {
    final (store, agent, session) = await _running(tester);
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-effort', 'High');
    expect(find.text('Model: Opus'), findsOneWidget, reason: 'a switch in a conversation is one quiet line');
    expect(find.text('Effort: High'), findsOneWidget);
    final first = agent.links.first;
    expect(agent.starts, ['Claude']);

    await _openPicker(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice two'))).onTap, isNotNull);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice login'))).onTap, isNotNull, reason: 'what moves is judged by the account it began on');
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice one'))).onTap, isNull, reason: 'it is on this one');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/session_picker_running_desktop.png'));
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 200);

    expect(agent.starts, ['Claude', 'Claude']);
    final initializes = [for (final request in agent.requests) if (request['method'] == 'initialize') request];
    expect(initializes.map(_asked), ['one', 'two']);
    final load = agent.last('session/load')['params'] as Map;
    expect(load['sessionId'], 'o1', reason: 'the conversation is the same one');
    expect(load['cwd'], '/work/repo');
    expect(session.account, 'two');
    expect(store.ledger!.entries.single.account, 'two');
    expect(first.closed, isTrue, reason: 'the first agent ended once nothing used it');
    expect(agent.model, 'opus');
    expect(agent.effort, 'high', reason: 'what the person had chosen went back to the new agent');
    expect(find.textContaining('Account: one → two'), findsOneWidget);
    expect(session.status, SessionStatus.connected);

    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Go on');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1, reason: 'the conversation works on the new agent');
    await finish(tester);
  });

  testWidgets('what is chosen before the conversation begins is not a line in it', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    await pumpApp(tester, store, linked: agent.openLink, config: _config, accountsHost: FakeAccountsHost(accounts: [account('one')]));
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await pick(tester, 'option-model', 'Opus 5.5');
    await pick(tester, 'option-effort', 'High');
    await send(tester, 'Say hello');
    expect(agent.model, 'opus');
    expect(find.textContaining('Model:'), findsNothing);
    expect(find.textContaining('Effort:'), findsNothing);
    await finish(tester);
  });

  testWidgets('a kind of account whose conversations stay with it says so, and nothing moves', (tester) async {
    final (_, agent, session) = await _running(tester, first: 'login');
    await _openPicker(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice one'))).onTap, isNull);
    expect(find.byKey(const Key('picker-account-note')), findsOneWidget);
    expect(find.textContaining('keeps its conversations to itself'), findsOneWidget);
    await tester.tap(find.byKey(const Key('account-choice one')), warnIfMissed: false);
    await frames(tester);
    expect(agent.starts, ['Claude']);
    expect(session.account, 'login');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await finish(tester);
  });

  testWidgets('while a turn runs the accounts wait, and the picker says to wait', (tester) async {
    final (_, agent, session) = await _running(tester, hold: true);
    expect(session.status, SessionStatus.working);
    await _openPicker(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice two'))).onTap, isNull);
    expect(find.textContaining('Wait for the turn to end'), findsOneWidget);
    expect(agent.starts, ['Claude']);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    agent.finishTurn();
    await settle(tester);
    await finish(tester);
  });

  testWidgets('an agent that will not start on the other account leaves the conversation on the first, and says why', (tester) async {
    final (store, agent, session) = await _running(tester);
    final first = agent.links.first;
    agent.failStart = 'no such account';
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 200);
    expect(session.account, 'one');
    expect(store.ledger!.entries.single.account, 'one');
    expect(first.closed, isFalse, reason: 'the first agent still holds it');
    expect(find.textContaining('Account: not moved to two'), findsOneWidget);
    expect(find.textContaining('stays on one'), findsOneWidget);
    agent.failStart = null;
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Still here');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1);
    await finish(tester);
  });
}
