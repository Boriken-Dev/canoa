// The account a session runs on: the composer's choice is what `initialize` carries to the agent, as a name and never a
// credential, the first usable account is the default, and the ledger keeps the name so a reopened session asks for the same.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageBucket, UsageRefill, UsageReport;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['work', 'metered']},
    },
  },
);

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

Object? _asked(OptionsAgent agent) {
  final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
  return ((initialize['params'] as Map)['_meta'] as Map)[accountMeta];
}

void main() {
  testWidgets('the account chosen in the composer is what initialize names, and the ledger keeps it', (tester) async {
    final agent = OptionsAgent();
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, config: _config, accountsHost: host);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    expect(find.byKey(const Key('option-account')), findsOneWidget);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('account-choice metered')));
    await frames(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(_asked(agent), 'metered');
    final kept = store.ledger!.entries.single;
    expect(kept.account, 'metered');
    // Only the name travels and is kept.
    expect(agent.requests.map((request) => '$request').join(), isNot(contains('sk-ant')));
    await finish(tester);
  });

  testWidgets('the account list says what is left of the short window and the week for an account heard from', (tester) async {
    final agent = OptionsAgent();
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);
    final store = _store();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    store.accountUsage['id-work'] = UsageReport(supported: true, account: 'id-work', buckets: [
      UsageBucket(id: 'session', used: 100, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(hours: 5), resetsAt: now + 3600)),
      UsageBucket(id: 'weekly', used: 380, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: now + 86400)),
    ]);
    await pumpApp(tester, store, linked: agent.openLink, config: _config, accountsHost: host);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(find.text('5 h 90% left · week 62% left'), findsOneWidget);
    // An account nothing was heard from says nothing of limits.
    expect(find.byKey(const Key('account-left-summary metered')), findsNothing);
    await finish(tester);
  });

  testWidgets('no choice names no account, and the host runs the entry\'s first', (tester) async {
    final agent = OptionsAgent();
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);
    await pumpApp(tester, _store(), linked: agent.openLink, config: _config, accountsHost: host);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
    expect(((initialize['params'] as Map)['_meta'] as Map).containsKey(accountMeta), isFalse);
    await finish(tester);
  });
}
