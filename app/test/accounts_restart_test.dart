// Changing which accounts a source offers restarts the host, which ends the sessions open on it: with some open, the person is asked
// first and told which end (and can leave the change for the next start); with none open, nothing is asked.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/fake_host.dart';
import 'fixtures/options_agent.dart';

const _config = CanoaConfig(host: {
  'agents': {
    'Claude-accounts': {'harness': 'claude', 'accounts': ['work']},
  },
});

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

/// Opens Settings at Accounts and offers `metered` from the entry: the change that restarts the host.
Future<void> _offerMetered(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings').evaluate().isNotEmpty ? find.byTooltip('Settings') : find.text('Settings').first);
  await frames(tester);
  await tester.tap(find.byKey(const Key('settings-nav-accounts')));
  await frames(tester);
  await tester.tap(find.byKey(const Key('account-menu metered')));
  await frames(tester);
  await tester.tap(find.byKey(const Key('account-offer metered Claude-accounts')));
  await frames(tester);
}

void main() {
  late int starts;
  late OptionsAgent agent;
  late FakeAccountsHost accounts;

  setUp(() {
    starts = 0;
    agent = OptionsAgent();
    accounts = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);
  });

  Future<List<CanoaConfig>> app(WidgetTester tester, SessionStore store) => pumpApp(
    tester,
    store,
    config: _config,
    accountsHost: accounts,
    host: linkedHost(agent.openLink, sources: () {
      starts++;
      return defaultLocalSources;
    }),
  );

  testWidgets('with a session open, the person is asked, told which session ends, and can leave it for the next start', (tester) async {
    final store = _store();
    final written = await app(tester, store);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    final title = store.sessions.firstWhere((session) => !session.isDraft).title;
    final before = starts;

    await _offerMetered(tester);
    expect(find.byKey(const Key('restart-accounts')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('restart-accounts')), matching: find.textContaining(title)), findsWidgets);
    await tester.tap(find.byKey(const Key('restart-accounts-later')));
    await frames(tester);
    // Saved, not applied: the offer is in canoa.cfg and the host was not started again.
    expect(written.last.hostAccountsOf('Claude-accounts'), ['work', 'metered']);
    expect(starts, before);
    await finish(tester);
  });

  testWidgets('answering Restart now starts the host again', (tester) async {
    final store = _store();
    await app(tester, store);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    final before = starts;
    await _offerMetered(tester);
    await tester.tap(find.byKey(const Key('restart-accounts-now')));
    await settle(tester);
    expect(starts, greaterThan(before));
    await finish(tester);
  });

  testWidgets('with nothing open nothing is asked, and the host restarts', (tester) async {
    final written = await app(tester, _store());
    final before = starts;
    await _offerMetered(tester);
    expect(find.byKey(const Key('restart-accounts')), findsNothing);
    await settle(tester);
    expect(written.last.hostAccountsOf('Claude-accounts'), ['work', 'metered']);
    expect(starts, greaterThan(before));
    await finish(tester);
  });
}
