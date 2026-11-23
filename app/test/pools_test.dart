// Pools: sets of accounts that provide one standard. The host's words are read (pools, an entry's pools, the accounts that list a
// model), the Accounts section makes, edits and removes a pool with the host's refusal shown as it wrote it, the picker offers the
// pools an entry offers beside its accounts, and a start on a pool names it in `initialize`.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

AccountsController _controller(FakeAccountsHost host, {CanoaConfig config = const CanoaConfig(), Future<void> Function()? restart}) =>
    AccountsController(host: host, config: () => config, change: (edit) async {}, restart: restart);

Future<void> _show(WidgetTester tester, Widget child) async {
  tester.view.physicalSize = const Size(900, 1600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: child))));
  await tester.pump();
  await tester.pump();
}

/// Two Anthropic keys, a Claude login and an OpenAI key; the host's own pools of each standard, and one the person made.
/// The pools the host was asked to make, as text: a record of lists does not compare by value.
List<String> _set(FakeAccountsHost host) => [for (final (name, standard, accounts) in host.poolsSet) '$name $standard $accounts'];

FakeAccountsHost _host() => FakeAccountsHost(accounts: [
  account('one', provides: ['anthropic-api']),
  account('two', provides: ['anthropic-api']),
  account('login', kind: 'claude-login', provides: ['claude-code']),
  account('chat', harness: 'codex', kind: 'codex-api-key', provides: ['openai-api']),
])
  ..hostPools.addAll(const [
    PoolInfo(name: 'keys', standard: 'anthropic-api', accounts: ['two', 'one']),
    PoolInfo(name: 'any-anthropic-api', standard: 'anthropic-api', automatic: true, accounts: ['one', 'two']),
    PoolInfo(name: 'any-claude-code', standard: 'claude-code', automatic: true, accounts: ['local', 'login']),
    PoolInfo(name: 'any-openai-api', standard: 'openai-api', automatic: true, accounts: ['chat']),
  ]);

void main() {
  test('the host\'s pools, an entry\'s pools and the accounts that list a model are read', () {
    final pool = PoolInfo.parse({'name': 'keys', 'standard': 'anthropic-api', 'automatic': false, 'accounts': ['two', 'one']})!;
    expect([pool.name, pool.standard, pool.automatic, ...pool.accounts], ['keys', 'anthropic-api', false, 'two', 'one']);
    expect(PoolInfo.parse({'name': 'x'}), isNull);
    final entry = AccountEntry.parse({
      'name': 'Claude',
      'harness': 'claude',
      'accounts': ['keys', 'login'],
      'pools': [
        {
          'name': 'keys',
          'standard': 'anthropic-api',
          'automatic': false,
          'listed': true,
          'accounts': [
            {'name': 'two', 'listed': true},
            {'name': 'one', 'listed': false},
          ],
        },
        {'name': 'any-claude-code', 'standard': 'claude-code', 'automatic': true, 'listed': false, 'accounts': <Object?>[]},
      ],
    })!;
    expect(entry.pools.map((each) => each.name), ['keys', 'any-claude-code']);
    expect(entry.pools.first.listed, isTrue);
    expect(entry.pools.first.members.map((each) => (each.name, each.listed)), [('two', true), ('one', false)]);
    expect(entry.pools.last.automatic, isTrue);
    expect(AccountEntry.parse({'name': 'Pi', 'harness': 'pi'})!.pools, isEmpty);
    final choices = SourceChoices.parse(jsonEncode({
      'known': true,
      'models': [
        {'id': 'shared', 'efforts': <String>[], 'accounts': ['a', 'b']},
        {'id': 'solo', 'efforts': <String>[], 'accounts': ['a']},
        {'id': 'plain', 'efforts': <String>[]},
      ],
      'accounts': [
        {'name': 'a', 'listed': true},
        {'name': 'b', 'listed': false, 'why': 'not signed in'},
      ],
    }))!;
    expect(choices.models.map((each) => each.accounts), [['a', 'b'], ['a'], <String>[]]);
    expect(choices.members.map((each) => (each.name, each.listed, each.why)), [('a', true, null), ('b', false, 'not signed in')]);
    expect(SourceChoices.parse(jsonEncode({'known': true}))!.members, isEmpty);
  });

  test('the controller asks for the pools with the accounts, and makes and removes them through the host', () async {
    final host = _host();
    var restarts = 0;
    final controller = _controller(host, restart: () async => restarts++);
    await controller.refresh();
    expect(controller.pools.map((each) => each.name), ['keys', 'any-anthropic-api', 'any-claude-code']);
    // A new pool may be of any standard an account provides, and holds exactly those accounts, `local` among them when the host's
    // pool of the standard holds it.
    expect(controller.standards, containsAll(['anthropic-api', 'claude-code', 'openai-api']));
    expect(controller.membersFor('anthropic-api'), ['one', 'two']);
    expect(controller.membersFor('claude-code'), ['local', 'login']);
    expect(controller.membersFor('openai-api'), ['chat']);
    await controller.setPool('more', 'anthropic-api', ['two', 'one']);
    expect(_set(host), ['more anthropic-api [two, one]']);
    expect(controller.pools.map((each) => each.name), contains('more'));
    expect(restarts, 1, reason: 'the host serves a pool from the moment it starts');
    await controller.removePool('more');
    expect(host.poolsRemoved, ['more']);
    expect(controller.pools.map((each) => each.name), isNot(contains('more')));
    expect(restarts, 2);
    // The host's refusal reaches the caller in its own words, and nothing restarts.
    host.poolRefusal = 'account login (kind claude-login) provides claude-code, and the pool\'s standard is anthropic-api';
    await expectLater(controller.setPool('bad', 'anthropic-api', ['login']), throwsA(predicate((error) => '$error'.contains('claude-login'))));
    expect(restarts, 2);
  });

  test('an entry offers the pools its harness accepts, and one it lists first is what a session starts on', () async {
    final host = _host()
      ..hostEntries.add(const AccountEntry(
        name: 'Claude',
        harness: 'claude',
        accounts: ['keys', 'login'],
        accepts: ['claude-code', 'anthropic-api'],
        pools: [
          EntryPool(name: 'keys', standard: 'anthropic-api', listed: true, members: [PoolMember(name: 'two', listed: true)]),
          EntryPool(name: 'any-claude-code', standard: 'claude-code', automatic: true, members: [PoolMember(name: 'local', listed: true), PoolMember(name: 'login', listed: true)]),
          EntryPool(name: 'any-anthropic-api', standard: 'anthropic-api', automatic: true, members: [PoolMember(name: 'one', listed: true)]),
        ],
      ));
    const config = CanoaConfig(host: {
      'agents': {
        'Claude': {'harness': 'claude', 'accounts': ['keys', 'login']},
      },
    });
    final controller = _controller(host, config: config);
    await controller.refresh();
    expect(controller.poolsOf('Claude').map((each) => each.name), ['keys', 'any-claude-code'], reason: 'a pool of one account cannot rotate, so no picker shows it');
    expect(controller.poolOf('Claude', 'any-anthropic-api'), isNull);
    expect(controller.poolOf('Claude', 'keys')?.listed, isTrue);
    expect(controller.poolOf('Claude', 'login'), isNull, reason: 'an account is no pool');
    expect(controller.defaultOf('Claude'), 'keys', reason: 'the entry lists a pool first');
    expect(controller.offeredBy('Claude').map((each) => each.name), ['login'], reason: 'a pool is not an account');
    // A pool the entry lists after an account is not the default: the first listed is.
    final after = _host()
      ..hostEntries.add(const AccountEntry(
        name: 'Claude',
        harness: 'claude',
        accounts: ['login', 'keys'],
        accepts: ['claude-code', 'anthropic-api'],
        pools: [EntryPool(name: 'keys', standard: 'anthropic-api', listed: true)],
      ));
    final second = _controller(after);
    await second.refresh();
    expect(second.defaultOf('Claude'), 'login');
  });

  testWidgets('Settings lists the person\'s pools editable and the host\'s own read-only, marked as made by Canoa', (tester) async {
    final controller = _controller(_host());
    await _show(tester, AccountsSection(controller: controller));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pools')), findsOneWidget);
    expect(find.byKey(const Key('pool-edit keys')), findsOneWidget);
    expect(find.byKey(const Key('pool-remove keys')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('pool keys')), matching: find.text('Anthropic API · two, one')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('pool any-claude-code')), matching: find.textContaining('Made by Canoa')), findsOneWidget);
    expect(find.byKey(const Key('pool-automatic any-claude-code')), findsOneWidget);
    expect(find.byKey(const Key('pool-edit any-claude-code')), findsNothing, reason: 'the host\'s own pool is not edited');
    expect(find.byKey(const Key('pool-remove any-claude-code')), findsNothing);
    // The host's pool of one account (the OpenAI key) cannot rotate, so Settings does not list it; the person's own pool of one would be.
    expect(find.byKey(const Key('pool any-openai-api')), findsNothing);
    expect(controller.pools.map((each) => each.name), ['keys', 'any-anthropic-api', 'any-claude-code']);
  });

  testWidgets('a pool is made of the accounts that provide the standard chosen, in the order given, and saved through the host', (tester) async {
    final host = _host();
    final controller = _controller(host);
    await _show(tester, AccountsSection(controller: controller));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-pool')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('pool-name')), 'mine');
    // The standards are the ones some account provides, and the accounts offered are exactly those that provide the chosen one.
    await tester.tap(find.byKey(const Key('pool-standard')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Anthropic API').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pool-member one')), findsOneWidget);
    expect(find.byKey(const Key('pool-member two')), findsOneWidget);
    expect(find.byKey(const Key('pool-member login')), findsNothing);
    expect(find.byKey(const Key('pool-member chat')), findsNothing);
    await tester.tap(find.byKey(const Key('pool-check two')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('pool-check one')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('pool-up one')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('pool-save')));
    await tester.pumpAndSettle();
    expect(_set(host), ['mine anthropic-api [one, two]']);
    expect(find.byKey(const Key('pool-dialog')), findsNothing);
    expect(find.byKey(const Key('pool mine')), findsOneWidget);
  });

  testWidgets('a pool the host refuses stays open with the host\'s words, and a pool of another standard offers its own accounts', (tester) async {
    final host = _host()..poolRefusal = 'account login (kind claude-login) provides claude-code, and the pool\'s standard is anthropic-api';
    await _show(tester, AccountsSection(controller: _controller(host)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('pool-edit keys')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pool-name')), findsOneWidget);
    expect(tester.widget<TextField>(find.byKey(const Key('pool-name'))).enabled, isFalse, reason: 'a pool keeps its name');
    expect(tester.widget<Checkbox>(find.byKey(const Key('pool-check two'))).value, isTrue);
    expect(tester.widget<Checkbox>(find.byKey(const Key('pool-check one'))).value, isTrue);
    await tester.tap(find.byKey(const Key('pool-save')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pool-dialog')), findsOneWidget);
    expect(find.text(host.poolRefusal!), findsOneWidget, reason: 'the host\'s refusal as it wrote it');
    expect(host.poolsSet, isEmpty);
    await tester.tap(find.byKey(const Key('pool-cancel')));
    await tester.pumpAndSettle();
    // The login of a harness is offered to a pool of the standard its pool holds it in.
    await tester.tap(find.byKey(const Key('add-pool')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('pool-standard')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Claude Code').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pool-member local')), findsOneWidget);
    expect(find.byKey(const Key('pool-member login')), findsOneWidget);
    expect(find.byKey(const Key('pool-member one')), findsNothing);
  });

  testWidgets('removing a pool asks, goes through the host, and shows the host\'s words when it refuses', (tester) async {
    final host = _host();
    await _show(tester, AccountsSection(controller: _controller(host)));
    await tester.pumpAndSettle();
    host.poolRefusal = 'entry Claude offers keys: take it out of that entry\'s accounts first';
    await tester.tap(find.byKey(const Key('pool-remove keys')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('remove-pool')), findsOneWidget);
    await tester.tap(find.byKey(const Key('remove-pool-confirm')));
    await tester.pumpAndSettle();
    expect(find.text(host.poolRefusal!), findsOneWidget);
    expect(find.byKey(const Key('pool keys')), findsOneWidget);
    host.poolRefusal = null;
    await tester.pumpAndSettle(const Duration(seconds: 5));
    await tester.tap(find.byKey(const Key('pool-remove keys')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('remove-pool-confirm')));
    await tester.pumpAndSettle();
    expect(host.poolsRemoved, ['keys']);
    expect(find.byKey(const Key('pool keys')), findsNothing);
  });

  test('what the host said of the account a connection ran on is read from initialize\'s answer', () async {
    final answers = StreamController<String>();
    addTearDown(answers.close);
    final client = VerifiedAgent.fromChild(label: 'a', messages: answers.stream, send: (_) async {}, close: () async {});
    expect(client.identity.ranOnAccount, isNull);
    answers.add(jsonEncode({
      'jsonrpc': '2.0',
      'id': 1,
      'result': {
        'protocolVersion': 1,
        '_meta': {
          accountMeta: {'name': 'one', 'kind': 'anthropic-api-key', 'pool': 'keys'},
        },
      },
    }));
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(client.identity.ranOnAccount, 'one');
    expect(client.identity.ranOnPool, 'keys');
  });

  group('the picker', () {
    const config = CanoaConfig(
      agents: [scripted],
      host: {
        'agents': {
          'Claude': {'harness': 'claude', 'accounts': ['keys', 'login']},
        },
      },
    );

    FakeAccountsHost host() => _host()
      ..hostEntries.add(const AccountEntry(
        name: 'Claude',
        harness: 'claude',
        accounts: ['keys', 'login'],
        accepts: ['claude-code', 'anthropic-api'],
        usable: ['login', 'one', 'two'],
        pools: [
          EntryPool(name: 'keys', standard: 'anthropic-api', listed: true, members: [PoolMember(name: 'two', listed: true), PoolMember(name: 'one', listed: true)]),
          EntryPool(name: 'any-claude-code', standard: 'claude-code', automatic: true, members: [PoolMember(name: 'local', listed: true), PoolMember(name: 'login', listed: true)]),
        ],
      ));

    SessionStore newStore() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

    Future<void> open(WidgetTester tester, OptionsAgent agent, SessionStore store, {ChoicesLoader? ask}) async {
      await pumpApp(tester, store, linked: agent.openLink, config: config, accountsHost: host(), askChoices: ask);
      await newSession(tester);
      await chooseSource(tester, 'Claude');
      await openFolder(tester);
      await settle(tester);
    }

    testWidgets('lists the pools an entry offers beside its accounts, the automatic one marked, and a choice names the pool in initialize', (tester) async {
      final agent = OptionsAgent();
      final store = newStore();
      await open(tester, agent, store);
      await tester.tap(find.byKey(const Key('option-account')));
      await frames(tester);
      expect(find.byKey(const Key('pool-choice keys')), findsOneWidget);
      expect(find.byKey(const Key('pool-choice any-claude-code')), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('pool-state any-claude-code'))).data, contains('made by Canoa'));
      expect(tester.widget<Text>(find.byKey(const Key('pool-state keys'))).data, contains('two, one'));
      await tester.ensureVisible(find.byKey(const Key('pool-choice any-claude-code')));
      await tester.tap(find.byKey(const Key('pool-choice any-claude-code')));
      await frames(tester);
      expect(store.sessions.where((each) => each.isDraft).single.account, 'any-claude-code');
      expect(find.descendant(of: find.byKey(const Key('pool-choice any-claude-code')), matching: find.byIcon(Icons.check)), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('account-machine')), matching: find.byIcon(Icons.check)), findsNothing, reason: 'a pool is not the machine\'s login');
      await closePicker(tester);
      expect(find.byKey(const Key('account-chip-pool')), findsOneWidget);
      expect(find.text('any-claude-code'), findsWidgets);
      await send(tester, 'Say hello');
      await settle(tester, 150);
      final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
      final meta = (initialize['params'] as Map)['_meta'] as Map<Object?, Object?>;
      expect(meta[accountMeta], 'any-claude-code');
      expect(store.ledger!.entries.single.account, 'any-claude-code');
      await finish(tester);
    });

    testWidgets('a model of a pool says which of its accounts list it', (tester) async {
      final listing = SourceChoices.parse(jsonEncode({
        'known': true,
        'models': [
          {'id': 'shared', 'name': 'Shared model', 'efforts': <String>[], 'accounts': ['one', 'two']},
          {'id': 'solo', 'name': 'Solo model', 'description': 'Only one', 'efforts': <String>[], 'accounts': ['two']},
        ],
        'accounts': [
          {'name': 'one', 'listed': true},
          {'name': 'two', 'listed': true},
        ],
      }));
      await open(tester, OptionsAgent(), newStore(), ask: (source) async => listing);
      await tester.tap(find.byKey(const Key('option-model')));
      await frames(tester);
      await tester.scrollUntilVisible(find.byKey(const ValueKey('model-choice=solo')), 40, scrollable: find.byType(Scrollable).last);
      expect(find.descendant(of: find.byKey(const ValueKey('model-choice=shared')), matching: find.text('On one, two')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const ValueKey('model-choice=solo')), matching: find.text('Only one · On two')), findsOneWidget);
      await closePicker(tester);
      await finish(tester);
    });

    testWidgets('a pool the entry lists first is what a new session is on, and the chip says so', (tester) async {
      final agent = OptionsAgent();
      await open(tester, agent, newStore());
      expect(find.byKey(const Key('account-chip-pool')), findsOneWidget);
      expect(find.text('keys'), findsWidgets);
      await tester.tap(find.byKey(const Key('option-account')));
      await frames(tester);
      expect(find.descendant(of: find.byKey(const Key('pool-choice keys')), matching: find.byIcon(Icons.check)), findsOneWidget);
      await closePicker(tester);
      await send(tester, 'Say hello');
      await settle(tester, 150);
      final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
      expect(((initialize['params'] as Map)['_meta'] as Map)[accountMeta], 'keys');
      await finish(tester);
    });
  });
}
