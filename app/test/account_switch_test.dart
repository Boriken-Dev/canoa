// Moving a running conversation to another account: the agent is started again for that account and opens the conversation, the
// host first carrying it to the account's own folder when the accounts keep theirs apart; the model, effort and mode go back, and
// the transcript says what happened in one line. A start or a move that fails leaves it on its account, with the reason.
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

Object? _asked(Map<String, dynamic> initialize) => ((initialize['params'] as Map)['_meta'] as Map)[accountMeta];

/// A Claude session running on [first] after its first message, with the host's accounts one and two (API keys) and login.
Future<(SessionStore, OptionsAgent, ChatSession, FakeAccountsHost)> _running(WidgetTester tester, {String first = 'one', bool hold = false}) async {
  final agent = OptionsAgent(modes: OptionsAgent.claudeModes)
    ..resetsChoicesOnLoad = true
    ..holdTurns = hold;
  final host = FakeAccountsHost(accounts: [account('one'), account('two'), account('login', kind: 'claude-login')]);
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
  return (store, agent, store.selected!, host);
}

Future<void> _openPicker(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('session-account')));
  await frames(tester);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/account_switch_test.dart')));

  testWidgets('accounts that share a folder: the same conversation is resumed on the new login, with its model and effort', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-effort', 'High');
    expect(find.text('Model: Opus'), findsOneWidget, reason: 'a switch in a conversation is one quiet line');
    expect(find.text('Effort: High'), findsOneWidget);
    final first = agent.links.first;

    await _openPicker(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice two'))).onTap, isNotNull);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice login'))).onTap, isNotNull, reason: 'every account of the entry can be moved to');
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice one'))).onTap, isNull, reason: 'it is on this one');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/session_picker_running_desktop.png'));
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 200);

    expect(host.moves.single.to, 'two');
    expect(host.moves.single.session, 'o1');
    expect(host.moves.single.from, 'one', reason: 'the account it ran on, the first the entry offers');
    expect(agent.starts, ['Claude', 'Claude']);
    final initializes = [for (final request in agent.requests) if (request['method'] == 'initialize') request];
    expect(initializes.map(_asked), ['one', 'two']);
    final load = agent.last('session/load')['params'] as Map;
    expect(load['sessionId'], 'o1', reason: 'nothing moved: it is the same conversation under the same id');
    expect(session.account, 'two');
    expect(store.ledger!.entries.single.account, 'two');
    expect(first.closed, isTrue, reason: 'the first agent ended before the second began');
    expect(agent.model, 'opus');
    expect(agent.effort, 'high', reason: 'what the person had chosen went back to the new agent');
    expect(find.textContaining('Account: one → two'), findsOneWidget);
    expect(find.textContaining('picked up as it was'), findsOneWidget);
    expect(session.status, SessionStatus.connected);

    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Go on');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1, reason: 'the conversation works on the new agent');
    await finish(tester);
  });

  testWidgets('accounts with a folder each: the host carries the conversation over, and it goes on under its new id', (tester) async {
    final (store, agent, session, host) = await _running(tester, first: 'login');
    host.moved = const AccountMove(moved: true, session: '7d7dbf66-0000-4000-8000-0000000000e1', via: 'provider');
    final before = store.ledger!.entries.single;
    store.ledger!.setPlacement(before.peerId, before.sessionId, pinned: true, archived: false);
    await pick(tester, 'option-model', 'Opus');

    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 200);

    expect(host.moves.single.from, 'login');
    expect(host.moves.single.to, 'two');
    expect((agent.last('session/load')['params'] as Map)['sessionId'], '7d7dbf66-0000-4000-8000-0000000000e1', reason: 'resumed in the new account\'s folder');
    expect(session.remoteSessionId, '7d7dbf66-0000-4000-8000-0000000000e1');
    final kept = store.ledger!.entries.single;
    expect(kept.sessionId, '7d7dbf66-0000-4000-8000-0000000000e1', reason: 'the list follows it');
    expect(kept.account, 'two');
    expect(kept.pinned, isTrue, reason: 'with what the person had made of it');
    expect(agent.model, 'opus');
    expect(find.textContaining('moved to that account\'s own folder (by the agent\'s own export) and picked up'), findsOneWidget);
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Go on');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1);
    await finish(tester);
  });

  testWidgets('a provider that can neither export nor import it: the conversation stays, and the exact reason is in it', (tester) async {
    final (store, agent, session, host) = await _running(tester, first: 'login');
    host.moveRefusal = 'Claude cannot export this conversation from account login, and the host has no record of it: no session has this id';
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 200);
    expect(session.account, 'login');
    expect(store.ledger!.entries.single.account, 'login');
    expect(find.textContaining('Account: not moved to two: '), findsOneWidget);
    expect(find.textContaining('and the host has no record of it: no session has this id'), findsOneWidget);
    expect(find.textContaining('stays on login'), findsOneWidget);
    expect(agent.starts, ['Claude', 'Claude'], reason: 'its agent was started again on the account it had');
    expect(session.status, SessionStatus.connected);
    host.moveRefusal = null;
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Still here');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1);
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

  testWidgets('while a turn runs the accounts wait, and the picker says to wait', (tester) async {
    final (_, agent, session, _) = await _running(tester, hold: true);
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

  testWidgets('an agent that will not start on the other account is started again on the first, and says why', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    agent.failNextStarts = 1;
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('account-choice two')));
    await settle(tester, 300);
    expect(host.moves, hasLength(1));
    expect(agent.starts, ['Claude', 'Claude', 'Claude'], reason: 'the first, the one that failed, and the one on the account it had');
    expect(session.account, 'one');
    expect(store.ledger!.entries.single.account, 'one');
    expect(find.textContaining('Account: not moved to two: '), findsOneWidget);
    expect(find.textContaining('no such account'), findsOneWidget);
    expect(find.textContaining('stays on one'), findsOneWidget);
    expect(session.status, SessionStatus.connected);
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Still here');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1);
    await finish(tester);
  });
}
