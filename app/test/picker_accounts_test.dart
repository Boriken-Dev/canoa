// The picker always offers an account for an agent that takes accounts: the login the agent has where it runs first, then every
// account of its kind whether or not canoa.cfg's entry lists it, and Add account. The choice is what `initialize` names.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/add_account_dialog.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

/// Claude lists nothing; the scripted agent runs a program of its own.
const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude'},
    },
  },
);

/// Claude lists one account.
const _listing = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['work']},
    },
  },
);

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

Map<Object?, Object?> _meta(OptionsAgent agent) {
  final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
  return (initialize['params'] as Map)['_meta'] as Map<Object?, Object?>;
}

FakeAccountsHost _host() => FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);

Future<void> _open(WidgetTester tester, String source, {OptionsAgent? agent, SessionStore? store, CanoaConfig config = _config}) async {
  await pumpApp(tester, store ?? _store(), linked: (agent ?? OptionsAgent()).openLink, config: config, accountsHost: _host());
  await newSession(tester);
  await chooseSource(tester, source);
  await openFolder(tester);
  await settle(tester);
}

void main() {
  testWidgets('an entry that lists no account still offers the machine login, every account of its kind and Add account', (tester) async {
    await _open(tester, 'Claude');
    expect(find.text(machineLoginText), findsOneWidget, reason: 'the chip says what the session runs on');
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(find.byKey(const Key('account-machine')), findsOneWidget);
    expect(find.byKey(const Key('account-choice work')), findsOneWidget);
    expect(find.byKey(const Key('account-choice metered')), findsOneWidget);
    expect(find.byKey(const Key('account-add')), findsOneWidget);
    // The machine's login comes first, and is the one in use until another is chosen.
    final machine = tester.getTopLeft(find.byKey(const Key('account-machine')));
    final work = tester.getTopLeft(find.byKey(const Key('account-choice work')));
    expect(machine.dy < work.dy || (machine.dy == work.dy && machine.dx < work.dx), isTrue);
    expect(find.descendant(of: find.byKey(const Key('account-machine')), matching: find.byIcon(Icons.check)), findsOneWidget);
    await finish(tester);
  });

  testWidgets('choosing an account the entry does not list starts the session under it', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    await _open(tester, 'Claude', agent: agent, store: store);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('account-choice metered')));
    await frames(tester);
    await closePicker(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(_meta(agent)[accountMeta], 'metered');
    expect(store.ledger!.entries.single.account, 'metered');
    await finish(tester);
  });

  testWidgets('an entry that lists no account and is left alone names none', (tester) async {
    final agent = OptionsAgent();
    await _open(tester, 'Claude', agent: agent);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(_meta(agent).containsKey(accountMeta), isFalse);
    await finish(tester);
  });

  testWidgets('an entry that lists accounts can still be started on the machine login, said as an explicit null', (tester) async {
    final agent = OptionsAgent();
    await _open(tester, 'Claude', agent: agent, config: _listing);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(find.descendant(of: find.byKey(const Key('account-choice work')), matching: find.byIcon(Icons.check)), findsOneWidget, reason: 'its first is the default');
    await tester.tap(find.byKey(const Key('account-machine')));
    await frames(tester);
    await closePicker(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(_meta(agent).containsKey(accountMeta), isTrue);
    expect(_meta(agent)[accountMeta], isNull);
    await finish(tester);
  });

  testWidgets('Add account opens the sign-in for the agent\'s kind', (tester) async {
    await _open(tester, 'Claude');
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('account-add')));
    await frames(tester);
    expect(find.byType(AddAccountDialog), findsOneWidget);
    await finish(tester);
  });

  testWidgets('an agent that runs a program of its own has no account choice', (tester) async {
    await _open(tester, 'Scripted');
    expect(find.byKey(const Key('option-account')), findsNothing);
    await finish(tester);
  });
}

const machineLoginText = "This machine's login";
