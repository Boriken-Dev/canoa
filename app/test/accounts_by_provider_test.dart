// Accounts are grouped by the account provider that made them, not by an agent: Add account starts from the provider and shows that
// provider's own ways to sign in, the index has a group for each provider, an account says which agents can run it, and the session
// picker offers an agent the accounts that fit it.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

const _litellm = AccountProviderInfo(id: 'litellm_accounts', title: 'LiteLLM', description: 'Accounts on a LiteLLM proxy');

const _litellmCatalog = {
  'kinds': [
    {'id': 'litellm', 'title': 'LiteLLM key', 'provides': ['anthropic-api', 'openai-api']},
  ],
  'logins': [
    {'id': 'litellm-sso', 'title': 'Sign in through the proxy', 'produces': 'litellm', 'steps': ['open_url']},
    {'id': 'litellm-key', 'title': 'A key you were given', 'produces': 'litellm', 'steps': ['ask']},
  ],
};

FakeAccountsHost _host() {
  final host = FakeAccountsHost(accounts: [
    account('work', kind: 'claude-login'),
    account('proxy', kind: 'litellm', provider: 'litellm_accounts', fits: ['claude', 'codex'], provides: ['anthropic-api', 'openai-api']),
  ]);
  host.makers.add(_litellm);
  host.catalogs['litellm_accounts'] = _litellmCatalog;
  return host;
}

AccountsController _controller(FakeAccountsHost host, {CanoaConfig config = const CanoaConfig()}) =>
    AccountsController(host: host, config: () => config, change: (edit) async {}, hostIsHere: true);

Future<void> _show(WidgetTester tester, AccountsController controller) async {
  tester.view.physicalSize = const Size(900, 1600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
  await tester.pump();
  await tester.pump();
}

void main() {
  testWidgets('the section has a group for each account provider, an agent\'s own and a maker, and an account sits under its provider', (tester) async {
    await _show(tester, _controller(_host()));
    expect(find.byKey(const Key('accounts-claude_accounts')), findsOneWidget);
    expect(find.byKey(const Key('accounts-litellm_accounts')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('accounts-litellm_accounts')), matching: find.byKey(const Key('account proxy'))), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('accounts-claude_accounts')), matching: find.byKey(const Key('account proxy'))), findsNothing);
    expect(find.descendant(of: find.byKey(const Key('accounts-claude_accounts')), matching: find.byKey(const Key('account work'))), findsOneWidget);
  });

  testWidgets('an account says which agents can run it', (tester) async {
    await _show(tester, _controller(_host()));
    expect(find.byKey(const Key('account-agents proxy')), findsOneWidget);
    expect(find.text('Agents: Claude, Codex'), findsOneWidget);
    expect(find.text('Agents: Claude'), findsOneWidget);
  });

  testWidgets('Add account lists the providers, and LiteLLM\'s ways to sign in are under LiteLLM and not under Claude', (tester) async {
    await _show(tester, _controller(_host()));
    await tester.tap(find.byKey(const Key('add-account-claude_accounts')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('login-method subscription')), findsOneWidget);
    expect(find.byKey(const Key('login-method litellm-sso')), findsNothing, reason: 'Claude signs in its own way only');
    expect(find.byKey(const Key('login-method litellm-key')), findsNothing);
    // The provider choice lists every provider; choosing LiteLLM shows its ways.
    await tester.tap(find.byKey(const Key('add-account-harness')));
    await tester.pumpAndSettle();
    expect(find.text('LiteLLM'), findsWidgets);
    await tester.tap(find.text('LiteLLM').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('login-method litellm-sso')), findsOneWidget);
    expect(find.byKey(const Key('login-method litellm-key')), findsOneWidget);
    expect(find.byKey(const Key('login-method subscription')), findsNothing);
  });

  testWidgets('a maker\'s group starts Add account at its own provider', (tester) async {
    final host = _host();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-litellm_accounts')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('login-method litellm-key')), findsOneWidget);
    await tester.tap(find.byKey(const Key('login-method litellm-key')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    expect(host.signIns.single.provider, 'litellm_accounts');
  });

  test('the accounts that fit an agent are the ones the host says fit it, whichever provider made them', () async {
    final controller = _controller(_host());
    await controller.refresh();
    expect([for (final each in controller.accounts) '${each.name} ${each.provider} ${each.fits.join(',')}'], [
      'work claude_accounts claude',
      'proxy litellm_accounts claude,codex',
    ]);
    expect(controller.accounts.last.agent, isNull);
    expect(controller.accounts.last.groupId, 'litellm_accounts');
    expect(controller.entriesFor(controller.accounts.last), isEmpty, reason: 'no entry is configured here');
  });
}
