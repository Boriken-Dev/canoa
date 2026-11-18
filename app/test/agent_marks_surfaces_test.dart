// Each agent's own mark wherever an agent or an account shows: the Store's agents, Settings > Accounts, Add account, the picker's
// account tiles, the composer's account chip and Usage. One function maps a harness, a Store program or a standard to its mark.
// With CANOA_SAMPLES_DIR set the scenes are written there with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/chat/session_picker.dart' show machineLoginLabel;
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/agent_marks.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show MemoryHome, MemoryOverlays, PlatformTolerant;
import 'fixtures/store_link.dart';

/// The mark the SourceMark under [key] draws, or null when it draws the generic icon.
String? _markAt(WidgetTester tester, Finder finder) {
  final mark = tester.widget<SourceMark>(finder);
  return markIdOf(mark.provider);
}

AccountsController _controller(FakeAccountsHost host) =>
    AccountsController(host: host, harnesses: const ['claude', 'codex', 'foundry'], config: () => const CanoaConfig(), change: (_) async {});

final _accounts = [
  account('work', kind: 'claude-login'),
  account('metered'),
  account('team', kind: 'openai-api-key', harness: 'codex'),
  account('chatgpt', kind: 'codex-login', harness: 'codex'),
  account('local', kind: 'foundry-local', harness: 'foundry'),
];

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/agent_marks_surfaces_test.dart')));

  test('a harness, a Store program and a standard each map to their agent\'s mark', () {
    expect(markIdOf('claude'), 'claude');
    expect(markIdOf('claude-code'), 'claude');
    expect(markIdOf('claude_store'), 'claude');
    expect(markIdOf('anthropic-api-key'), 'claude');
    expect(markIdOf('codex'), 'codex');
    expect(markIdOf('openai-api-key'), 'codex');
    expect(markIdOf('grok_store'), 'grok');
    expect(markIdOf('xai-api'), 'grok');
    expect(markIdOf('pi_store'), 'pi');
    expect(markIdOf('opencode_store'), 'opencode');
    expect(markIdOf('OpenCode'), 'opencode');
    expect(markIdOf('antigravity'), 'antigravity');
    expect(markIdOf('Claude'), 'claude');
    expect(markIdOf('foundry-local'), isNull);
    expect(markIdOf('pipeline'), isNull, reason: 'only a whole first word names an agent');
    expect(markIdOf(null), isNull);
    expect(accountMarkId('foundry', 'foundry-local'), isNull);
    expect(accountMarkId('custom', 'anthropic-api-key'), 'claude', reason: 'the kind is the standard when the harness has no mark');
    for (final id in agentMarks.keys) {
      expect(markIdOf(id), id);
    }
  });

  group('the Store\'s agents', () {
    FakeStore store(String id, String title, String serves) =>
        FakeStore(id, title: title, serves: [{'provider': serves, 'when': <String, Object?>{}}], copies: [copyOf('1.0.0')], latest: '1.0.0');

    for (final (label, size) in [('phone', const Size(390, 1500)), ('desktop', const Size(1280, 800))]) {
      testWidgets('each agent tile shows its own mark at $label width', (tester) async {
        final host = FakeStoreHost([
          store('claude_store', 'Claude Code', 'claude'),
          store('codex_store', 'Codex', 'codex'),
          store('grok_store', 'Grok', 'grok'),
          store('pi_store', 'Pi', 'pi'),
          store('antigravity_store', 'Antigravity', 'antigravity'),
          store('other_store', 'Another program', 'stdio'),
        ]);
        tester.view
          ..physicalSize = size
          ..devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        await tester.pumpWidget(MaterialApp(
          theme: themeFor(LookId.canoa, Brightness.dark),
          home: StoreSection(config: const CanoaConfig(), home: MemoryHome(MemoryOverlays()), onSave: (changed) async => changed, programs: host.programs()),
        ));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('store-tab-agents')));
        await tester.pumpAndSettle();
        for (final (id, mark) in [('claude', 'claude'), ('codex', 'codex'), ('grok', 'grok'), ('pi', 'pi'), ('antigravity', 'antigravity')]) {
          expect(_markAt(tester, find.byKey(Key('program-mark-${id}_store'))), mark, reason: id);
        }
        expect(_markAt(tester, find.byKey(const Key('program-mark-other_store'))), isNull, reason: 'a program with no mark here keeps the generic icon');
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_store_$label.png'));
      });
    }
  });

  group('Settings > Accounts and Add account', () {
    testWidgets('each provider and each account shows its agent\'s mark, and a local model the generic icon', (tester) async {
      final host = FakeAccountsHost(accounts: _accounts);
      host.catalogs['claude'] = claudeCatalog;
      tester.view
        ..physicalSize = const Size(900, 1400)
        ..devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: _controller(host)))),
      ));
      await tester.pump();
      await tester.pump();
      expect(_markAt(tester, find.byKey(const Key('account-mark work'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('account-mark metered'))), 'claude', reason: 'an Anthropic key is Claude\'s');
      expect(_markAt(tester, find.byKey(const Key('account-mark team'))), 'codex', reason: 'an OpenAI key is Codex\'s');
      expect(_markAt(tester, find.byKey(const Key('account-mark chatgpt'))), 'codex');
      expect(_markAt(tester, find.byKey(const Key('account-mark local'))), isNull);
      expect(find.descendant(of: find.byKey(const Key('accounts-claude')), matching: find.byType(SourceMark)), findsNWidgets(3), reason: 'the provider and its two accounts');
      expect(find.descendant(of: find.byKey(const Key('accounts-codex')), matching: find.byType(SourceMark)), findsNWidgets(3));
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_accounts.png'));

      await tester.tap(find.byKey(const Key('add-account-claude')));
      await tester.pumpAndSettle();
      expect(_markAt(tester, find.byKey(const Key('method-mark subscription'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('method-mark api-key'))), 'claude');
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_add_account.png'));
    });
  });

  group('the picker and the composer\'s chip', () {
    const config = CanoaConfig(
      agents: [scripted],
      host: {
        'agents': {
          'Claude': {'harness': 'claude'},
          'Codex': {'harness': 'codex'},
        },
      },
    );

    Future<void> open(WidgetTester tester, String source) async {
      await pumpApp(tester, SessionStore(ledger: SessionLedger(MemoryLedgerBackend())), linked: OptionsAgent().openLink, config: config, accountsHost: FakeAccountsHost(accounts: _accounts));
      await newSession(tester);
      await chooseSource(tester, source);
      await openFolder(tester);
      await settle(tester);
    }

    testWidgets('Claude: the chip and every account tile carry Claude\'s mark', (tester) async {
      await open(tester, 'Claude');
      expect(_markAt(tester, find.byKey(const Key('account-chip-mark'))), 'claude', reason: 'the machine login of a Claude entry');
      await tester.tap(find.byKey(const Key('option-account')));
      await frames(tester);
      expect(_markAt(tester, find.byKey(const Key('account-mark $machineLoginLabel'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('account-mark work'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('account-mark metered'))), 'claude');
      expect(find.byKey(const Key('account-mark team')), findsNothing, reason: 'a Codex account is not offered to Claude');
      await finish(tester);
    });

    testWidgets('Codex: its accounts carry Codex\'s mark, in the tiles and after one is chosen on the chip', (tester) async {
      await open(tester, 'Codex');
      await tester.tap(find.byKey(const Key('option-account')));
      await frames(tester);
      expect(_markAt(tester, find.byKey(const Key('account-mark team'))), 'codex');
      expect(_markAt(tester, find.byKey(const Key('account-mark chatgpt'))), 'codex');
      await tester.tap(find.byKey(const Key('account-choice team')));
      await frames(tester);
      expect(_markAt(tester, find.byKey(const Key('account-chip-mark'))), 'codex');
      await finish(tester);
    });
  });

  group('Usage', () {
    testWidgets('the index and the limits carry each account\'s agent mark', (tester) async {
      final accounts = _controller(FakeAccountsHost(accounts: _accounts));
      await accounts.refresh();
      final usage = UsageController(sources: [_Reported()], accounts: accounts, now: () => 0);
      await usage.refresh();
      tester.view
        ..physicalSize = const Size(1280, 800)
        ..devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: Row(children: [
            SizedBox(width: 260, child: UsageIndex(controller: usage)),
            Expanded(child: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
          ]),
        ),
      ));
      await tester.pumpAndSettle();
      expect(_markAt(tester, find.byKey(const Key('usage-index-mark id-work'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('usage-index-mark id-team'))), 'codex');
      expect(_markAt(tester, find.byKey(const Key('usage-account-mark id-work'))), 'claude');
      expect(_markAt(tester, find.byKey(const Key('usage-account-mark id-team'))), 'codex');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_usage.png'));
    });
  });
}


final class _Reported implements UsageSource {
  @override
  String get name => 'Laptop';

  @override
  Future<List<Object?>> limits() async => [
    for (final id in ['id-work', 'id-team'])
      {
        'account': id,
        'updated_at': 0,
        'observed_at': 0,
        'buckets': [
          {
            'id': 'weekly',
            'label': 'Weekly',
            'unit': 'permille',
            'used': 300,
            'limit': 1000,
            'refill': {'kind': 'fixed_window', 'length': [604800, 0], 'resets_at': 86400},
            'status': 'ok',
            'observed_at': 0,
            'source': 'reported',
          },
        ],
      },
  ];

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};
}
