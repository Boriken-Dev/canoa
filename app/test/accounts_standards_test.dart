// What an account supports and what an agent accepts: Settings says each account's standards and each source's, Add account says
// what each way to sign in makes, and the picker lists the accounts the agent's entry can run on by the host's match, each with its
// standard, whatever provider made it.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/agent_sections.dart';
import 'package:canoa/settings/settings_controller.dart';
import 'package:canoa/sources/sources.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

AccountsController _controller(FakeAccountsHost host, {CanoaConfig config = const CanoaConfig()}) =>
    AccountsController(host: host, config: () => config, change: (edit) async {});

Future<void> _show(WidgetTester tester, Widget child) async {
  tester.view.physicalSize = const Size(900, 1400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: child))));
  await tester.pump();
  await tester.pump();
}

const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['work']},
    },
  },
);

/// Claude's entry takes a subscription login and an Anthropic key, and the Foundry gateway of Codex's provider speaks both APIs.
FakeAccountsHost _host() => FakeAccountsHost(accounts: [
  account('work', kind: 'claude-login', provides: ['claude-code']),
  account('metered', provides: ['anthropic-api']),
  account('gateway', kind: 'foundry-gateway', harness: 'codex', provides: ['anthropic-api', 'openai-api']),
  account('chatgpt', kind: 'codex-chatgpt', harness: 'codex', provides: ['codex']),
])..hostEntries.add(const AccountEntry(
  name: 'Claude',
  harness: 'claude',
  accounts: ['work'],
  accepts: ['claude-code', 'anthropic-api'],
  usable: ['work', 'metered', 'gateway'],
));

void main() {
  test('the host\'s lists are read: an account\'s provides, an entry\'s accepts and usable, a kind\'s provides', () {
    final entry = AccountEntry.parse({'name': 'Pi', 'harness': 'pi', 'accounts': <String>[], 'accepts': ['openai-api', 'anthropic-api'], 'usable': ['a']})!;
    expect(entry.accepts, ['openai-api', 'anthropic-api']);
    expect(entry.usable, ['a']);
    expect(AccountEntry.parse({'name': 'Pi', 'harness': 'pi'})!.usable, isNull, reason: 'a host that does not say leaves it to the provider');
    expect(account('k', provides: ['codex']).types, ['codex']);
    expect(AccountKindInfo.parse({'id': 'k', 'provides': ['xai-api']}).provides, ['xai-api']);
    expect(standardsText(['claude-code', 'anthropic-api', 'a-new-one']), 'Claude Code, Anthropic API, a-new-one');
  });

  testWidgets('Settings says what each account supports', (tester) async {
    final host = _host();
    await _show(tester, AccountsSection(controller: _controller(host)));
    expect(find.descendant(of: find.byKey(const Key('account work')), matching: find.text('Supports Claude Code')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('account metered')), matching: find.text('Supports Anthropic API')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('account gateway')), matching: find.text('Supports Anthropic API, OpenAI API')), findsOneWidget);
  });

  testWidgets('Settings shows the warning the host gives for an account address, and nothing for one with none', (tester) async {
    const note = 'account lan: base_url is plain http to 192.0.2.11: what is sent there, a key or a password included, crosses the network unprotected';
    final host = FakeAccountsHost(accounts: [
      account('lan', kind: 'litellm', provides: ['openai-api'], warnings: [note]),
      account('metered', provides: ['anthropic-api']),
    ]);
    await _show(tester, AccountsSection(controller: _controller(host)));
    expect(find.descendant(of: find.byKey(const Key('account lan')), matching: find.text(note)), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('account metered')), matching: find.byIcon(Icons.warning_amber_rounded)), findsNothing);
  });

  testWidgets('Add account says what each way to sign in makes, by its standard', (tester) async {
    await _show(tester, AccountsSection(controller: _controller(FakeAccountsHost())));
    await tester.tap(find.byKey(const Key('add-account-claude_accounts')));
    await tester.pumpAndSettle();
    expect(find.descendant(of: find.byKey(const Key('login-method subscription')), matching: find.text('Supports Claude Code')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('login-method api-key')), matching: find.text('Supports Anthropic API')), findsOneWidget);
  });

  testWidgets('a source says the standards its agent accepts', (tester) async {
    final controller = _controller(_host());
    await controller.refresh();
    final settings = SettingsController(config: const CanoaConfig(), apply: (config) async => config);
    await _show(
      tester,
      SourcesSection(
        controller: settings,
        presets: const [AgentSource.hosted('Claude', harness: 'claude', local: true)],
        accounts: controller,
      ),
    );
    await tester.tap(find.byKey(const Key('source-row Claude')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('source-accepts Claude')), findsOneWidget);
    expect(find.text('Accepts accounts that support Claude Code, Anthropic API'), findsOneWidget);
  });

  group('the picker', () {
    Future<void> open(WidgetTester tester) async {
      await pumpApp(
        tester,
        SessionStore(ledger: SessionLedger(MemoryLedgerBackend())),
        linked: OptionsAgent().openLink,
        config: _config,
        accountsHost: _host(),
      );
      await newSession(tester);
      await chooseSource(tester, 'Claude');
      await openFolder(tester);
      await settle(tester);
      await tester.tap(find.byKey(const Key('option-account')));
      await frames(tester);
    }

    testWidgets('lists the accounts the host says the entry can run on, whichever provider made them, and no other', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('account-choice work')), findsOneWidget);
      expect(find.byKey(const Key('account-choice metered')), findsOneWidget);
      expect(find.byKey(const Key('account-choice gateway')), findsOneWidget, reason: 'its standards meet what Claude Code accepts');
      expect(find.byKey(const Key('account-choice chatgpt')), findsNothing, reason: 'Codex\'s own login is not a standard Claude Code accepts');
      await finish(tester);
    });

    testWidgets('shows each account\'s standard on its tile', (tester) async {
      await open(tester);
      expect(find.descendant(of: find.byKey(const Key('account-choice work')), matching: find.text('Claude Code · Signed in')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('account-choice gateway')), matching: find.text('Anthropic API, OpenAI API · Signed in')), findsOneWidget);
      await finish(tester);
    });

    testWidgets('says what the agent accepts', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('picker-accepts')), findsOneWidget);
      expect(find.text('Claude accepts Claude Code, Anthropic API'), findsOneWidget);
      await finish(tester);
    });
  });
}
