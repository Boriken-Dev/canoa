// What the host lists is what the app shows: the agents, their names and marks and the accounts' kinds come from the host's answer
// to `providers/list`, so an agent the host did not load has no account type, mark or name anywhere, and one a library adds is
// drawn like any other.
import 'package:canoa/accounts/account_models.dart' show standardTitle;
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/report.dart';
import 'package:canoa/providers/provider_catalog.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/source_options.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/providers_fixture.dart';

final class _Reports implements UsageSource {
  _Reports(this.entries);

  final List<Object?> entries;

  @override
  String get name => 'Laptop';

  @override
  Future<List<Object?>> limits() async => entries;

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};
}

Map<String, Object?> _reading(String id) => {
  'account': id,
  'updated_at': 0,
  'observed_at': 0,
  'buckets': [
    {'id': 'weekly', 'label': 'Weekly', 'unit': 'permille', 'used': 100, 'limit': 1000, 'status': 'ok', 'observed_at': 0, 'source': 'reported'},
  ],
};

Future<UsageController> _usage(FakeAccountsHost host) async {
  final accounts = _controller(host);
  await accounts.refresh();
  final usage = UsageController(sources: [_Reports([_reading('id-mine'), _reading('id-theirs')])], accounts: accounts, now: () => 0);
  await usage.refresh();
  return usage;
}

AccountsController _controller(FakeAccountsHost host) => AccountsController(host: host, config: () => const CanoaConfig(), change: (_) async {});

Future<void> _show(WidgetTester tester, AccountsController controller) async {
  tester.view.physicalSize = const Size(900, 1400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
  await tester.pump();
  await tester.pump();
}

void main() {
  tearDown(listBundledProviders);

  test('the descriptions of an agent entry and the readers of a quota stay when the agent rows are replaced', () {
    expect(providerCatalog.agentEntry?.schema['properties'], containsPair('program', isA<Map>()));
    expect(providerCatalog.localAgent?.schema['required'], ['name', 'program']);
    final readers = providerCatalog.usageReaders.map((each) => each.id).toList();
    expect(readers, isNotEmpty);
    providerCatalog.replace(providersOf(const ['codex']));
    expect(providerCatalog.agentEntry, isNotNull);
    expect(providerCatalog.localAgent, isNotNull);
    expect(providerCatalog.usageReaders.map((each) => each.id), readers);
    expect(providerCatalog.gateways, isNotEmpty);
  });

  test('the answer is read into agents with their names, standards, mark and accounts', () {
    final claude = providerCatalog.byId('claude')!;
    expect((claude.title, claude.program, claude.vendor, claude.aos), ('Claude', 'Claude Code', 'anthropic', 'claude-code'));
    expect(claude.standards['anthropic-api'], 'Anthropic API');
    expect(claude.mark, isNotNull);
    expect(claude.harness, isTrue);
    expect(claude.accounts!.kinds.map((each) => each.$1), contains('claude-login'));
    expect(providerCatalog.byId('grok')!.harness, isTrue);
    expect(providerCatalog.byId('grok')!.accounts, isNotNull);
    expect(providerCatalog.byId('antigravity')!.harness, isTrue);
    expect(providerCatalog.byId('antigravity')!.accounts, isNotNull);
    expect(providerCatalog.byId('gemini')!.harness, isFalse);
    expect(providerCatalog.byId('gemini')!.accounts, isNull);
    expect(providerCatalog.byId('pi')!.accounts, isNotNull);
  });

  test('a name means the agent whose id, title, source, standard, vendor, registry id or reported name it is', () {
    expect(providerCatalog.resolve('Claude')?.id, 'claude');
    expect(providerCatalog.resolve('claude-code')?.id, 'claude');
    expect(providerCatalog.resolve('claude_store')?.id, 'claude');
    expect(providerCatalog.resolve('anthropic-api')?.id, 'claude');
    expect(providerCatalog.resolve('openai-api-key')?.id, 'codex');
    expect(providerCatalog.resolve('@agentclientprotocol/codex-acp')?.id, 'codex');
    expect(providerCatalog.resolve('pipeline'), isNull);
    expect(providerCatalog.resolve(null), isNull);
    expect(providerCatalog.byMarkers(['agentVersion', 'grokShell'])?.id, 'grok');
    expect(providerCatalog.byMarkers(const []), isNull);
  });

  test('a program that reports its own quota is read as the host says, and only while its agent is loaded', () {
    expect(usageFor(parseCommand('agy acp')!), 'antigravity');
    expect(usageFor(parseCommand('/opt/bin/agy.exe acp')!), 'antigravity');
    expect(usageFor(parseCommand('agy chat')!), isNull);
    expect(usageFor(parseCommand('other acp')!), isNull);
    providerCatalog.replace(providersOf(const ['codex']));
    expect(usageFor(parseCommand('agy acp')!), isNull);
  });

  test("the readers of an agent's quota are the ones the host offers, bundled and a library's, and none before it answers", () {
    expect(providerCatalog.usageReaders.map((each) => (each.id, each.origin)), [('antigravity', 'bundled')]);
    providerCatalog.adopt({
      'providers': const <Object?>[],
      'usage_readers': [
        {'id': 'antigravity', 'origin': 'bundled'},
        {'id': 'team_meter', 'origin': 'team_ext'},
        {'origin': 'no id'},
      ],
    });
    expect(providerCatalog.usageReaders.map((each) => (each.id, each.origin)), [('antigravity', 'bundled'), ('team_meter', 'team_ext')]);
    providerCatalog.adopt({'providers': const <Object?>[]});
    expect(providerCatalog.usageReaders, isEmpty, reason: 'a host that offers none');
    providerCatalog.clear();
    expect(providerCatalog.usageReaders, isEmpty);
  });

  test("which sources can bring in earlier sessions is the host's word, not a list of names", () {
    expect(providerCatalog.byId('claude')!.imports, isTrue);
    expect(providerCatalog.byId('codex')!.imports, isTrue);
    expect(providerCatalog.byId('pi')!.imports, isFalse);
    expect(providerCatalog.byId('zed'), isNull);
  });

  group('a host that lists only Codex', () {
    setUp(() => providerCatalog.replace(providersOf(const ['codex'])));

    testWidgets('has no Claude among the providers an account can be added to, in Settings or in the dialog', (tester) async {
      final host = FakeAccountsHost(providers: providersOf(const ['codex']));
      final controller = _controller(host);
      await _show(tester, controller);
      expect(controller.providers.map((each) => each.agent), ['codex']);
      expect(find.byKey(const Key('add-account-codex_accounts')), findsOneWidget);
      expect(find.byKey(const Key('add-account-claude_accounts')), findsNothing);
      expect(find.text('Claude'), findsNothing);
      await tester.tap(find.byKey(const Key('add-account-codex_accounts')));
      await tester.pumpAndSettle();
      expect(find.text('Claude'), findsNothing);
      expect(find.byType(DropdownButton<String>), findsNothing, reason: 'one provider is no choice');
    });

    test('names and draws nothing of Claude', () {
      expect(providerCatalog.resolve('claude'), isNull);
      expect(markIdOf('claude'), isNull);
      expect(markIdOf('anthropic-api-key'), isNull);
      expect(standardTitle('claude-code'), 'claude-code', reason: 'no loaded agent owns the standard, so it is shown as written');
      expect(harnessLabel('claude-code'), 'claude-code');
      expect(knownModes(const AgentSource.hosted('Claude', harness: 'claude', local: true)), isEmpty);
      expect(markIdOf('codex'), 'codex');
    });
  });

  group('an agent a library adds', () {
    setUp(() => providerCatalog.replace([...bundledProviders(), extraProvider()]));

    testWidgets('is offered in Add account with its name, mark and kinds, and its accounts are listed under it', (tester) async {
      final host = FakeAccountsHost(providers: [...providersOf(const ['claude', 'codex']), extraProvider()], accounts: [accountOf('zed', 'mine')]);
      host.catalogs['zed_accounts'] = {
        'harness': 'zed',
        'kinds': [
          {'id': 'zed-key', 'title': 'Zed key'},
        ],
        'logins': [
          {'id': 'key', 'title': 'Paste a key', 'produces': 'zed-key', 'steps': <String>[], 'remote': true},
        ],
      };
      final controller = _controller(host);
      await _show(tester, controller);
      expect(controller.providers.map((each) => each.agent), ['claude', 'codex', 'zed']);
      expect(find.byKey(const Key('add-account-zed_accounts')), findsOneWidget);
      expect(find.text('Zed'), findsWidgets);
      expect(find.text('mine'), findsOneWidget);
      expect(markIdOf('zed'), 'zed');
      expect(markIdOf('zed-api'), 'zed', reason: 'a standard of the new agent is its own');
      await tester.tap(find.byKey(const Key('add-account-zed_accounts')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('login-method key')), findsOneWidget);
      expect(find.text('Paste a key'), findsOneWidget);
    });

    test('has its standards titled and its mark drawn, with no change to the app', () {
      expect(standardTitle('zed-api'), 'Zed API');
      expect(providerCatalog.markOf('zed')?.side, 10);
      expect(providerCatalog.titleOf('zed'), 'Zed');
      expect(providerCatalog.titleOf('other'), 'Other', reason: 'an id no loaded agent names is written with a capital');
    });
  });

  group('Usage', () {
    testWidgets('lists the accounts of the agents the host loaded, each under its own name and mark, and an agent a library adds like the rest', (tester) async {
      providerCatalog.replace([...providersOf(const ['codex']), extraProvider()]);
      final host = FakeAccountsHost(
        providers: [...providersOf(const ['codex']), extraProvider()],
        accounts: [account('mine', kind: 'openai-api-key', harness: 'codex', id: 'id-mine'), accountOf('zed', 'theirs')],
      );
      final usage = await _usage(host);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SizedBox(width: 300, height: 600, child: UsageIndex(controller: usage)))));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('usage-index provider codex')), findsOneWidget);
      expect(find.byKey(const Key('usage-index provider zed')), findsOneWidget);
      expect(find.byKey(const Key('usage-index provider claude')), findsNothing);
      expect(find.descendant(of: find.byKey(const Key('usage-index-mark zed')), matching: find.byType(CustomPaint)), findsWidgets, reason: "the new agent's own mark");
      expect(find.descendant(of: find.byKey(const Key('usage-index-mark codex')), matching: find.byType(Icon)), findsNothing);
    });
  });
}
