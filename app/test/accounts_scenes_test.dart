// Accounts as Canoa draws them: Settings › Accounts with an account's usage, Add account with the provider's methods, the page and
// code a sign-in shows, the sealed form a key is typed into, and the composer's chip. Goldens are drawn in the test font; with
// CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart' as app;
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/secret_host.dart' as secret;
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

const _config = CanoaConfig(host: {
  'agents': {
    'Claude': {'harness': 'claude', 'accounts': ['work', 'metered', 'old']},
  },
});

FakeAccountsHost _host() => FakeAccountsHost(accounts: [
  account('work', kind: 'claude-login', id: 'id-work', provides: ['claude-code']),
  account('metered', provides: ['anthropic-api']),
  account('old', kind: 'claude-login', state: 'needs sign-in', provides: ['claude-code']),
]);

AccountsController _controller(FakeAccountsHost host, {UsageReport? usage, String? Function(String)? holder}) => AccountsController(
  host: host,
  config: () => _config,
  change: (_) async {},
  usage: (id) => id == 'id-work' ? usage : null,
  holder: holder,
);

UsageReport _usage() {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  return UsageReport(supported: true, account: 'id-work', buckets: [
    UsageBucket(
      id: 'session', label: 'Session (5 h)', used: 120, limit: 1000,
      refill: UsageRefill('fixed_window', length: const Duration(hours: 5), resetsAt: now + 3 * 3600 + 20 * 60),
    ),
    UsageBucket(
      id: 'weekly', label: 'Weekly', used: 640, limit: 1000,
      refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: now + 4 * 86400),
    ),
  ]);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/accounts_scenes_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/accounts_${name}_$label.png'));
      }

      Future<FakeAccountsHost> open(WidgetTester tester, {String? Function(String)? holder}) async {
        final host = _host();
        final controller = _controller(host, usage: _usage(), holder: holder);
        await controller.refresh();
        await openSettings(tester, config: _config, size: size, section: 'accounts', accounts: controller, version: '0.0.1');
        return host;
      }

      Future<void> startAddAccount(WidgetTester tester, String method) async {
        await tester.tap(find.byKey(const Key('add-account-claude_accounts')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(Key('login-method $method')));
        await tester.pump();
      }

      testWidgets('Settings, Accounts: each account with its kind, state and, opened, its usage', (tester) async {
        await open(tester, holder: (name) => name == 'metered' ? 'Fix the build' : null);
        await tester.tap(find.byKey(const Key('account-usage-toggle work')));
        await tester.pumpAndSettle();
        await golden(tester, 'settings');
      });

      testWidgets('Add account: the provider\'s own ways to sign in', (tester) async {
        await open(tester);
        await startAddAccount(tester, 'subscription');
        await golden(tester, 'add');
      });

      testWidgets('a sign-in that shows a page and a code to confirm', (tester) async {
        final host = await open(tester);
        await startAddAccount(tester, 'subscription');
        await tester.tap(find.byKey(const Key('add-account-start')));
        await tester.pump();
        host.signIns.single
          ..say('Starting the sign-in')
          ..showPage(const SignInPage(url: 'https://claude.ai/oauth/authorize?code=true', userCode: 'WDJB-MJHT'));
        await tester.pump();
        await golden(tester, 'sign_in_page');
      });

      testWidgets('the sealed form a key is typed into, which only the host reads', (tester) async {
        final host = await open(tester);
        await startAddAccount(tester, 'api-key');
        await tester.tap(find.byKey(const Key('add-account-start')));
        await tester.pump();
        final (request, environment, _) = (await tester.runAsync(keyRequest))!;
        final asked = host.signIns.single.ask(request, environment);
        await tester.pump();
        await secret.until(tester, find.byKey(const ValueKey('secret-api_key')));
        await tester.enterText(find.byKey(const ValueKey('secret-api_key')), 'sk-ant-api03-xxxxxxxxxxxxxxxx');
        FocusManager.instance.primaryFocus?.unfocus();
        await tester.pump();
        await golden(tester, 'sealed_key');
        host.signIns.single.cancel();
        expect(await asked, isNotNull);
      });
    });
  }

  testWidgets('the composer\'s chip lists what the source offers, a held account disabled, desktop', (tester) async {
    final agent = OptionsAgent();
    final host = _host()..hostEntries.add(const AccountEntry(name: 'Claude', harness: 'claude', accepts: ['claude-code', 'anthropic-api'], usable: ['work', 'metered', 'old']));
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    await app.pumpApp(tester, store, linked: agent.openLink, config: _config, accountsHost: host);
    await app.newSession(tester);
    await app.chooseSource(tester, 'Claude');
    await app.openFolder(tester);
    await app.settle(tester);
    await tester.tap(find.byKey(const Key('option-account')));
    await app.frames(tester);
    expect(tester.takeException(), isNull);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/accounts_chip_desktop.png'));
  });
}
