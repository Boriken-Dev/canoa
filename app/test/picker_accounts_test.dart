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

FakeAccountsHost _host() => FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('other', kind: 'claude-login'), account('metered')]);

Future<void> _open(WidgetTester tester, String source, {OptionsAgent? agent, SessionStore? store, CanoaConfig config = _config, FakeAccountsHost? host}) async {
  await pumpApp(tester, store ?? _store(), linked: (agent ?? OptionsAgent()).openLink, config: config, accountsHost: host ?? _host());
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
    await tester.tap(find.byKey(const Key('account-choice other')));
    await frames(tester);
    await closePicker(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(_meta(agent)[accountMeta], 'other');
    expect(store.ledger!.entries.single.account, 'other');
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

  testWidgets('an unlisted account whose key reaches the agent says so, cannot be chosen, and can be offered to the agent', (tester) async {
    final host = _host();
    final store = _store();
    await _open(tester, 'Claude', host: host, store: store);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-choice metered'))).onTap, isNull);
    expect(find.text('Its key reaches the agent; offer it to this agent first'), findsOneWidget);
    expect(find.byKey(const Key('account-offer metered')), findsOneWidget);
    expect(find.byKey(const Key('account-offer other')), findsNothing, reason: 'a login kept in a folder of its own needs no offer');
    await tester.tap(find.byKey(const Key('account-offer metered')));
    await settle(tester, 100);
    // An agent open to every paired key says so first.
    if (find.byKey(const Key('offer-here-confirm')).evaluate().isNotEmpty) {
      await tester.tap(find.byKey(const Key('offer-here-confirm')));
      await settle(tester, 100);
    }
    expect(host.offers.last.$1, 'Claude');
    expect(host.offers.last.$2, ['metered']);
    expect(store.selected!.account, 'metered', reason: 'and it is chosen once offered');
    await finish(tester);
  });

  testWidgets('a running conversation moves to the machine login, and from it to an unlisted account', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..resetsChoicesOnLoad = true;
    final host = _host();
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, config: _listing, accountsHost: host);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    final session = store.selected!;
    expect(session.account, 'work');
    await tester.tap(find.byKey(const Key('session-account')));
    await frames(tester);
    expect(tester.widget<InkWell>(find.byKey(const Key('account-machine'))).onTap, isNotNull, reason: 'the machine login can be moved to');
    await tester.tap(find.byKey(const Key('account-machine')));
    await settle(tester, 200);
    expect(host.moves.last.from, 'work');
    expect(host.moves.last.to, machineLogin);
    expect(session.account, machineLogin);
    expect(find.textContaining("Account: work → this machine's login"), findsOneWidget);
    final initializes = [for (final request in agent.requests) if (request['method'] == 'initialize') request];
    expect((initializes.last['params'] as Map)['_meta'], containsPair(accountMeta, null));

    await tester.tap(find.byKey(const Key('session-account')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('account-choice other')));
    await settle(tester, 200);
    expect(host.moves.last.from, machineLogin);
    expect(host.moves.last.to, 'other');
    expect(session.account, 'other');
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

  testWidgets('an account no agent made is offered to the agents the host says it fits, and to no other', (tester) async {
    final host = FakeAccountsHost(accounts: [
      account('proxy', kind: 'litellm', provider: 'litellm_accounts', fits: ['claude', 'codex'], provides: ['anthropic-api', 'openai-api']),
      account('only-codex', kind: 'litellm', provider: 'litellm_accounts', fits: ['codex'], provides: ['openai-api']),
    ]);
    await _open(tester, 'Claude', host: host);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(find.byKey(const Key('account-choice proxy')), findsOneWidget);
    expect(find.byKey(const Key('account-choice only-codex')), findsNothing);
  });
}

const machineLoginText = "This machine's login";
