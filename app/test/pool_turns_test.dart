// The turns of a session started on a pool: before each message the host says which account of the pool takes it, and a session whose
// prompt cache has lapsed, or whose account has run out, moves to that account and says so in the conversation; a refusal (no account
// has usage left) stops the message and keeps it with the person. A session on one account is never moved: when it runs out the
// conversation offers the move, and nothing happens until it is taken.
import 'dart:async';

import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['keys', 'login']},
    },
  },
);

/// An entry that lists accounts only, as one that offers no pool does.
const _plain = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['one', 'two', 'login']},
    },
  },
);

const _far = 4102444800;

Map<String, Object?> _usage({required bool exhausted, String account = 'id-one', int revision = 1}) => {
  'supported': true,
  'sources': ['reported'],
  'account': account,
  'revision': revision,
  'observed_at': 1,
  'buckets': [
    {
      'id': 'five_hour',
      'label': 'Session (5 hours)',
      'models': {'kind': 'all'},
      'unit': 'permille',
      'used': exhausted ? 1000 : 100,
      'limit': 1000,
      'refill': {'kind': 'fixed_window', 'resets_at': _far},
      'status': exhausted ? 'exhausted' : 'ok',
      'observed_at': 1,
      'source': 'reported',
    },
  ],
};

FakeAccountsHost _host() => FakeAccountsHost(accounts: [account('one', provides: ['anthropic-api']), account('two', provides: ['anthropic-api']), account('login', kind: 'claude-login')])
  ..hostEntries.add(const AccountEntry(
    name: 'Claude',
    harness: 'claude',
    accounts: ['keys', 'login'],
    accepts: ['claude-code', 'anthropic-api'],
    usable: ['login', 'one', 'two'],
    pools: [
      EntryPool(name: 'keys', standard: 'anthropic-api', listed: true, members: [PoolMember(name: 'one', listed: true), PoolMember(name: 'two', listed: true)]),
    ],
  ));

/// A Claude session after its first message, started on [on] (a pool, or an account), whose agent says it ran on one.
Future<(SessionStore, OptionsAgent, ChatSession, FakeAccountsHost)> _running(WidgetTester tester, {String on = 'keys', Map<String, Object?>? usage}) async {
  final agent = OptionsAgent(modes: OptionsAgent.claudeModes)
    ..reportsAccount = true
    ..poolTakes['keys'] = 'one'
    ..usage = usage;
  final host = _host();
  final now = DateTime.utc(2026, 11, 20, 9);
  if (on != 'keys') host.hostEntries[0] = const AccountEntry(name: 'Claude', harness: 'claude', accounts: ['one', 'two', 'login'], accepts: ['claude-code', 'anthropic-api'], usable: ['login', 'one', 'two']);
  final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()))..clock = () => now;
  await pumpApp(tester, store, linked: agent.openLink, config: on == 'keys' ? _config : _plain, accountsHost: host);
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  await settle(tester);
  if (on == 'keys') {
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    final choice = find.byKey(const Key('pool-choice keys'));
    await tester.ensureVisible(choice);
    await tester.tap(choice);
    await frames(tester);
    await closePicker(tester);
  }
  await send(tester, 'Say hello');
  await settle(tester, 150);
  final session = store.selected!;
  expect(session.account, on);
  expect(session.ranOnAccount, on == 'keys' ? 'one' : on);
  return (store, agent, session, host);
}

int _prompts(OptionsAgent agent) => agent.methods.where((method) => method == 'session/prompt').length;

void main() {
  testWidgets('a warm cache keeps the account: the host is asked how long ago the last turn ended and the message goes as it is', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    host.turnAdvice['keys'] = const TurnAdvice(account: 'one', pool: 'keys', why: 'kept');
    await send(tester, 'Go on');
    final ask = host.turnAsks.single;
    expect((ask.entry, ask.account, ask.current), ('Claude', 'keys', 'one'));
    expect(ask.idleSeconds, 0, reason: 'the clock has not moved since the last turn ended');
    expect(ask.lifetime, isNotNull);
    expect(ask.limited, isEmpty);
    expect(host.moves, isEmpty);
    expect(agent.starts, ['Claude'], reason: 'no new agent');
    expect(_prompts(agent), 2);
    expect(session.account, 'keys');
    expect(store.selected, same(session));
    await finish(tester);
  });

  testWidgets('a lapsed cache moves the conversation to the account the host names, which the pool keeps naming, and a line says so', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    store.clock = () => DateTime.utc(2026, 11, 20, 10, 30);
    host.turnAdvice['keys'] = const TurnAdvice(account: 'two', pool: 'keys', moved: true, why: 'in_turn');
    await send(tester, 'Go on');
    expect(host.turnAsks.single.idleSeconds, 5400, reason: 'an hour and a half after the turn ended at nine');
    expect(host.moves.single.from, 'one', reason: 'the account it ran on, not the pool');
    expect(host.moves.single.to, 'two');
    final initializes = [for (final request in agent.requests) if (request['method'] == 'initialize') request];
    expect([for (final each in initializes) (((each['params'] as Map)['_meta'] as Map)[accountMeta])], ['keys', 'two'], reason: 'the new agent is started on the member');
    expect(session.account, 'keys', reason: 'the session still names the pool');
    expect(session.ranOnAccount, 'two');
    expect(session.ranOnPool, 'keys');
    expect(find.textContaining('Pool keys: moved from one to two: the prompt cache had lapsed'), findsOneWidget);
    expect(_prompts(agent), 2, reason: 'the message went on the new agent');
    expect(find.text('keys · two'), findsOneWidget, reason: 'the chip says which account it runs on now');
    await finish(tester);
  });

  testWidgets('an account that ran out is left even with a warm cache, and the line says what ran out', (tester) async {
    final (_, _, session, host) = await _running(tester);
    host.turnAdvice['keys'] = const TurnAdvice(account: 'two', pool: 'keys', moved: true, why: 'exhausted', left: 'Session (5 hours) is used up');
    await send(tester, 'Go on');
    expect(find.textContaining('Pool keys: moved from one to two: one had run out (Session (5 hours) is used up)'), findsOneWidget);
    expect(session.ranOnAccount, 'two');
    await finish(tester);
  });

  testWidgets('with no account left the message is not sent, stays with the person and the refusal naming the refill time is in the conversation', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    const why = 'no account of pool keys can take a turn: one: Session (5 hours) is used up; two: Session (5 hours) is used up; the soonest allowance refills in 2 h 10 min (at 2027-01-15 10:10 UTC)';
    host.turnRefusal = why;
    await send(tester, 'Go on');
    expect(_prompts(agent), 1, reason: 'nothing was sent');
    expect(host.moves, isEmpty);
    expect(find.textContaining('refills in 2 h 10 min'), findsWidgets);
    expect(session.messages.any((message) => message.text == 'Pool keys: $why'), isTrue, reason: 'the conversation says why');
    expect(find.widgetWithText(TextField, 'Go on'), findsOneWidget, reason: 'what was written is there to send again');
    host.turnRefusal = null;
    unawaited(store.sendText('Go on'));
    await settle(tester);
    expect(_prompts(agent), 2, reason: 'sent when the host allows it');
    await finish(tester);
  });

  testWidgets('a host that cannot be asked never holds a message back', (tester) async {
    final (_, agent, _, host) = await _running(tester);
    host.turnFails = true;
    await send(tester, 'Go on');
    expect(host.turnAsks, hasLength(1));
    expect(_prompts(agent), 2);
    expect(host.moves, isEmpty);
    await finish(tester);
  });

  testWidgets('a session on one account is asked about only when it has run out, offered the move, and moved only when the person takes it', (tester) async {
    final (store, agent, session, host) = await _running(tester, on: 'one', usage: _usage(exhausted: false));
    host.turnAdvice['one'] = const TurnAdvice(account: 'one', exhausted: true, left: 'Session (5 hours) is used up', alternatives: ['two', 'login']);
    await send(tester, 'Go on');
    expect(host.turnAsks, isEmpty, reason: 'a session on one account is not asked about before a message, and has room');
    expect(session.accountOffer, isNull);

    agent.pushUsage(_usage(exhausted: true, revision: 2));
    await settle(tester);
    expect(host.turnAsks.single.account, 'one');
    expect(host.turnAsks.single.current, 'one');
    final offer = session.accountOffer!;
    expect((offer.account, offer.left), ('one', 'Session (5 hours) is used up'));
    expect(offer.alternatives, ['two', 'login']);
    expect(find.byKey(const Key('account-offer')), findsOneWidget);
    expect(find.textContaining('Account one has run out (Session (5 hours) is used up)'), findsOneWidget);
    expect(host.moves, isEmpty, reason: 'nothing moves by itself');
    expect(session.ranOnAccount, 'one');
    // It is not asked again for the same running out.
    agent.pushUsage(_usage(exhausted: true, revision: 3));
    await settle(tester);
    expect(host.turnAsks, hasLength(1));

    await tester.tap(find.byKey(const Key('account-offer-move two')));
    await settle(tester, 200);
    expect(host.moves.single.to, 'two');
    expect(session.account, 'two');
    expect(session.accountOffer, isNull);
    expect(find.byKey(const Key('account-offer')), findsNothing);
    expect(store.selected, same(session));
    await finish(tester);
  });

  testWidgets('a dismissed offer does not come back until the account has had room', (tester) async {
    final (_, agent, session, host) = await _running(tester, on: 'one', usage: _usage(exhausted: false));
    host.turnAdvice['one'] = const TurnAdvice(account: 'one', exhausted: true, left: 'Session (5 hours) is used up', alternatives: ['two']);
    agent.pushUsage(_usage(exhausted: true, revision: 2));
    await settle(tester);
    expect(session.accountOffer, isNotNull);
    await tester.tap(find.byKey(const Key('account-offer-dismiss')));
    await frames(tester);
    expect(session.accountOffer, isNull);
    agent.pushUsage(_usage(exhausted: true, revision: 3));
    await settle(tester);
    expect(session.accountOffer, isNull, reason: 'dismissed');
    expect(host.turnAsks, hasLength(1));
    // Room again, then out again: offered again.
    agent.pushUsage(_usage(exhausted: false, revision: 4));
    await settle(tester);
    agent.pushUsage(_usage(exhausted: true, revision: 5));
    await settle(tester);
    expect(host.turnAsks, hasLength(2));
    expect(session.accountOffer, isNotNull);
    expect(host.moves, isEmpty);
    await finish(tester);
  });

  testWidgets('an account the host says nothing else could replace is said once in the conversation, with no offer', (tester) async {
    final (_, agent, session, host) = await _running(tester, on: 'one', usage: _usage(exhausted: false));
    host.turnAdvice['one'] = const TurnAdvice(account: 'one', exhausted: true, left: 'Session (5 hours) is used up');
    agent.pushUsage(_usage(exhausted: true, revision: 2));
    await settle(tester);
    expect(session.accountOffer, isNull);
    expect(find.textContaining('no other account of this agent could take the conversation'), findsOneWidget);
    await finish(tester);
  });
}
