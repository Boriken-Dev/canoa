// Settings as a page: a list of sections, rows that apply as they change, and every section writing canoa.cfg without losing
// the tables it does not edit. A canoa.cfg holding every table is read, changed one value at a time through the page, written
// back and compared table by table.
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/popout/open_in.dart';
import 'package:canoa/providers/provider_catalog.dart';
import 'dart:io' show FileSystemException;

import 'package:canoa/session_store.dart';
import 'package:canoa/settings/settings_page.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/settings/settings_row.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:toml/toml.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/providers_fixture.dart';
import 'fixtures/settings_fixture.dart';

final digest = 'sha256:${'b' * 64}';

/// Every table canoa.cfg takes, each with something in it.
final everything = '''
[desktop]
start_hidden = false
close_to_tray = true
minimize_to_tray = true
local_host = true

[network]
prefer_webtransport = true
connect_timeout = 15

[assets]
cdn = "local"

[secrets]
default = "form"

[secrets.providers.work]
kind = "env"

[settings.claude]
effort = "high"

[say.reader]
program = "piper"

[appearance]
look = "terminal"
brightness = "system"
density = "verbose"
open_in = "new_windows"

[sessions]
keep = 200
keep_text = true
text_messages = 7
workspace = "\${documents}/mine/\${session}"

[harnesses]
"my-agent" = "claude"

[notifications]
permission = true
question = false
finished = true

[voice]
backend = "local"
model = "small"
language = "es-PR"
device = "usb"
mode = "hold"

[attachments]
paste_as_file = 100
embed_limit = 2048

[usage]
warn_permille = 700
limit_phrases = ["rate limit"]

[permissions]
default_mode = "plan"
allow_once_only = true
protected = ["secrets", "!secrets/ok"]

[sources]
hidden = ["Pi"]

[agents.Mine]
program = "my-agent"
args = ["--acp"]

[agents.Mine.profile]
model = "m"

[agents.Mine.mcp]
own = false

[agents.Mine.hooks]
team = "$digest"

[host.agents.old]
program = "agent"
listen = "127.0.0.1:7801"

[overlays]
team = "https://example.invalid/team.git@main"

[profile]
effort = "high"

[profile.mcp.own]
type = "stdio"
command = "own-mcp"
args = ["--serve"]

[store.overlays.catalogs]
boriken = false
main = "https://example.invalid/catalog.git@main#catalog.json"

[store]
catalog_max_bytes = 100000

[skill_pins]
helper = "$digest"

[loadouts.main]
overlays = ["team"]

[defaults]
loadout = "main"

[profiles."Deep review"]
source = "my-agent"
model = "opus"
context = "Read the tests first."
loadout = "main"
''';

Map<String, dynamic> tomlOf(CanoaConfig config) => TomlDocument.parse(config.encode()).toMap();

/// The tables of [after] that differ from [before], by name.
Set<String> changedTables(CanoaConfig before, CanoaConfig after) {
  final a = tomlOf(before), b = tomlOf(after);
  return {for (final name in {...a.keys, ...b.keys}) if (jsonEncode(a[name]) != jsonEncode(b[name])) name};
}

final every = CanoaConfig.parse(everything);

void main() {
  test('the canoa.cfg used here holds every table, and writes back whole', () {
    final names = tomlOf(every).keys.toSet();
    expect(names, containsAll(['desktop', 'network', 'assets', 'secrets', 'settings', 'say', 'appearance', 'sessions', 'harnesses', 'notifications', 'attachments',
      'usage', 'voice', 'permissions', 'sources', 'agents', 'host', 'overlays', 'profile', 'store', 'skill_pins', 'loadouts', 'defaults', 'profiles']));
    expect(changedTables(every, CanoaConfig.parse(every.encode())), isEmpty);
  });

  test('the comparison sees a table lost: [host] dropped, and [permissions] dropped by a stale copy', () {
    expect(changedTables(every, every.withHost(null)), {'host'});
    expect(changedTables(every, every.copyWith(clearProtectedPaths: true, allowOnceOnly: false, clearDefaultMode: true)), {'permissions'});
  });

  group('the page', () {
    testWidgets('lists the sections in groups, opens the first, and has no Save, Cancel or Quit', (tester) async {
      await openSettings(tester);
      for (final group in const ['App', 'Agents', 'Store', 'About']) {
        expect(find.text(group), findsWidgets);
      }
      for (final id in const ['general', 'appearance', 'notifications', 'shortcuts', 'sources', 'profiles', 'mcp', 'permissions', 'usage', 'store', 'about']) {
        expect(find.byKey(Key('settings-nav-$id')), findsOneWidget, reason: id);
      }
      expect(find.byKey(const Key('settings-nav-hosting')), findsNothing, reason: 'this app runs no host here');
      expect(find.byKey(const Key('settings-section-general')), findsOneWidget);
      for (final id in const ['general', 'appearance', 'notifications', 'shortcuts', 'sources', 'profiles', 'mcp', 'permissions', 'usage', 'store', 'about']) {
        await goTo(tester, id);
        expect(find.byKey(Key('settings-section-$id')), findsOneWidget, reason: id);
        for (final word in const ['Save', 'Cancel', 'Quit Canoa']) {
          expect(find.text(word), findsNothing, reason: '$word in $id');
        }
      }
    });

    testWidgets('a search over the sections\' keywords narrows the list, and says when nothing matches', (tester) async {
      await openSettings(tester);
      await tester.enterText(find.byKey(const Key('settings-search')), 'tray');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
      expect(find.byKey(const Key('settings-nav-sources')), findsNothing);
      await tester.enterText(find.byKey(const Key('settings-search')), 'bypass');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-permissions')), findsOneWidget);
      expect(find.byKey(const Key('settings-section-permissions')), findsOneWidget, reason: 'the first match is shown');
      await tester.enterText(find.byKey(const Key('settings-search')), 'zzz');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-no-match')), findsOneWidget);
      await tester.enterText(find.byKey(const Key('settings-search')), '');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
    });

    testWidgets('a search finds the sections about agents by the names of the agents the host loaded, and by no other', (tester) async {
      await openSettings(tester);
      await tester.enterText(find.byKey(const Key('settings-search')), 'claude');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-sources')), findsOneWidget);
      providerCatalog.replace(providersOf(const ['codex']));
      addTearDown(listBundledProviders);
      await tester.enterText(find.byKey(const Key('settings-search')), 'claude ');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-sources')), findsNothing, reason: 'no loaded agent is called Claude');
      expect(find.byKey(const Key('settings-no-match')), findsOneWidget);
      await tester.enterText(find.byKey(const Key('settings-search')), 'codex');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-sources')), findsOneWidget);
    });

    testWidgets('at phone width the list is a page, a section another, and back goes to the list', (tester) async {
      await openSettings(tester, size: const Size(390, 844));
      expect(find.byKey(const Key('settings-nav-appearance')), findsOneWidget);
      expect(find.byKey(const Key('settings-section-general')), findsNothing);
      await goTo(tester, 'appearance');
      expect(find.byKey(const Key('settings-section-appearance')), findsOneWidget);
      expect(find.byKey(const Key('settings-nav-appearance')), findsNothing);
      await tester.tap(find.byKey(const Key('settings-back')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-appearance')), findsOneWidget);
      await goTo(tester, 'general');
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('settings-nav-general')), findsOneWidget, reason: 'the system back goes to the list too');
      expect(tester.takeException(), isNull);
    });

    testWidgets('a write that fails is said in the page, and the value stays shown', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1100, 1800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        home: SettingsPage(config: const CanoaConfig(), section: 'usage', apply: (changed) async => throw const FileSystemException('disk full')),
      ));
      await tester.pumpAndSettle();
      await choose(tester, 'settings-warn', '90%');
      expect(find.byKey(const Key('settings-write-problem')), findsOneWidget);
      expect(find.textContaining('disk full'), findsOneWidget);
      expect(tester.widget<DropdownButton<int>>(find.byKey(const Key('settings-warn'))).value, 900, reason: 'what was chosen stays shown');
    });
  });

  group('General', () {
    testWidgets('where chats keep their files is shown as the host works it out, for the template as it stands', (tester) async {
      final asked = <String>[];
      await openSettings(tester, section: 'general', hostWorkspacePreview: (template) async {
        asked.add(template);
        return '/host/docs/${template.contains('day') ? 'day' : 'chats'}/example';
      });
      expect(find.textContaining('/host/docs/chats/example'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('settings-workspace')), r'${documents}/day/${session}');
      await tester.pumpAndSettle();
      expect(find.textContaining('/host/docs/day/example'), findsOneWidget);
      expect(asked.length, 2, reason: 'asked once for each text, not on every build');
    });

    testWidgets('round-trips through canoa.cfg, marks what stays on this computer, and disables a sub-option with its parent', (tester) async {
      final h = await openSettings(tester, config: every, section: 'general');
      expect(find.byKey(const Key('machine-badge')), findsWidgets);
      await tester.tap(find.descendant(of: find.widgetWithText(SwitchRow, 'Close the window to the tray'), matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(h.last.closeToTray, isFalse);
      expect(changedTables(every, h.last), {'desktop'});
      expect(CanoaConfig.parse(h.last.encode()).closeToTray, isFalse);

      expect(tester.widget<SwitchRow>(find.byKey(const Key('settings-keep-text'))).enabled, isTrue);
      await choose(tester, 'settings-keep', 'None');
      expect(h.last.sessions.keep, 0);
      expect(tester.widget<SwitchRow>(find.byKey(const Key('settings-keep-text'))).enabled, isFalse, reason: 'a sub-option goes with its parent');
      expect(changedTables(every, h.last), {'desktop', 'sessions'});
      expect(h.last.sessions.harnesses, every.sessions.harnesses);

      // Any number above 0 is taken, typed; the presets are only suggestions.
      await tester.enterText(find.byKey(const Key('settings-timeout')), '300');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(h.last.connectTimeout, 300);
      await tester.tap(find.byKey(const ValueKey('settings-timeout-presets')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('30 s').last);
      await tester.pumpAndSettle();
      expect(h.last.connectTimeout, 30);
      expect(tester.widget<TextField>(find.byKey(const Key('settings-timeout'))).controller!.text, '30');
      await tester.enterText(find.byKey(const Key('settings-timeout')), '0');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(h.last.connectTimeout, 30, reason: 'a number that is not above 0 changes nothing');
      expect(find.text('Above 0'), findsOneWidget);
      expect(changedTables(every, h.last), {'desktop', 'sessions', 'network'});
    });

    testWidgets('Enter while the agent works and how queued messages go are rows, and change only sessions', (tester) async {
      final h = await openSettings(tester, config: every);
      await choose(tester, 'settings-while-working', 'Steer the turn');
      await choose(tester, 'settings-queue-delivery', 'One at a time');
      expect(h.last.sessions.whileWorking, WhileWorking.steer);
      expect(h.last.sessions.queueDelivery, QueueDelivery.each);
      expect(changedTables(every, h.last), {'sessions'});
    });

    testWidgets('clears what was said with the session list', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), settings: const LedgerSettings(keepText: true));
      await openSettings(tester, ledger: ledger, config: const CanoaConfig(sessions: LedgerSettings(keepText: true)));
      expect(find.text('None is kept'), findsOneWidget);
      expect(tester.widget<TextButton>(find.byKey(const Key('settings-clear-text'))).onPressed, isNull);
    });
  });

  group('Appearance', () {
    testWidgets('round-trips, and previews the choice before the app shows it', (tester) async {
      final h = await openSettings(tester, config: every, section: 'appearance');
      final preview = find.byKey(const Key('appearance-preview'));
      expect(Theme.of(tester.element(preview)).extension<CanoaLook>()!.id, LookId.terminal, reason: 'the preview is in the look chosen');
      await choose(tester, 'settings-look', 'Canoa');
      expect(h.last.look.name, 'canoa');
      expect(Theme.of(tester.element(preview)).extension<CanoaLook>()!.id, LookId.canoa, reason: 'and follows the choice');
      await choose(tester, 'settings-brightness', 'Light');
      await choose(tester, 'settings-density', 'Normal');
      expect(h.last.brightness, 'light');
      expect(h.last.density.name, 'normal');
      expect(h.last.openIn, OpenIn.newWindows, reason: 'the cfg chose new windows and the page shows it');
      await choose(tester, 'settings-open-in', 'This window');
      expect(h.last.openIn, OpenIn.thisWindow);
      expect(h.last.encode().contains('open_in'), isFalse, reason: 'this window is the default, so it is not written');
      await choose(tester, 'settings-open-in', 'New windows');
      expect(h.last.openIn, OpenIn.newWindows);
      expect(CanoaConfig.parse(h.last.encode()).openIn, OpenIn.newWindows, reason: 'and it is written back, so a restart finds it');
      expect(changedTables(every, h.last), {'appearance'});
      expect(find.textContaining('return a + b;'), findsOneWidget);
    });
  });

  group('Notifications', () {
    for (final (label, read) in <(String, bool Function(NotificationSettings))>[
      ('It asks leave to use a tool', (n) => n.permission),
      ('It has a question for you', (n) => n.question),
      ('It finished its turn', (n) => n.finished),
    ]) {
      testWidgets('"$label" is a row of its own, and changes only itself', (tester) async {
        final h = await openSettings(tester, config: every, section: 'notifications');
        final row = find.widgetWithText(SwitchRow, label);
        expect(row, findsOneWidget);
        expect(find.descendant(of: row, matching: find.byType(Switch)), findsOneWidget);
        final before = read(every.notifications);
        await tester.tap(find.descendant(of: row, matching: find.byType(Switch)));
        await tester.pumpAndSettle();
        expect(read(h.last.notifications), !before);
        expect(changedTables(every, h.last), {'notifications'});
        final others = [every.notifications.permission, every.notifications.question, every.notifications.finished];
        final now = [h.last.notifications.permission, h.last.notifications.question, h.last.notifications.finished];
        expect([for (var i = 0; i < 3; i++) others[i] != now[i]].where((changed) => changed), hasLength(1));
      });
    }

    testWidgets('a browser is asked for leave only where that is needed', (tester) async {
      await openSettings(tester, section: 'notifications');
      expect(find.byKey(const Key('settings-allow-notifications')), findsNothing);
      var asked = 0;
      await openSettings(tester, section: 'notifications', onAllowNotifications: () async => ++asked > 0);
      await tester.tap(find.byKey(const Key('settings-allow-notifications')));
      await tester.pumpAndSettle();
      expect(asked, 1);
      expect(find.text('This browser shows notifications from Canoa.'), findsOneWidget);
    });
  });

  testWidgets('Shortcuts lists each shortcut, Settings\' own among them', (tester) async {
    await openSettings(tester, section: 'shortcuts');
    expect(find.text('Open Settings'), findsOneWidget);
    expect(find.textContaining('Ctrl+,').evaluate().isNotEmpty || find.textContaining('+,').evaluate().isNotEmpty, isTrue);
  });

  group('Sources', () {
    final claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
    final codex = AgentSource.hosted('Codex', harness: 'codex', local: true);

    Future<SourceCheck> check(AgentSource source) async => source.label == 'Claude'
        ? SourceCheck.fromStore(store: 'claude_store', title: 'Claude Code', installed: true, path: '/bin/claude', version: '2.1.3')
        : source.label == 'Codex'
        ? const SourceCheck(missing: ['codex'])
        : const SourceCheck();

    testWidgets('each is a row with its name, version, readiness and a switch; the command is in its detail', (tester) async {
      final h = await openSettings(tester, config: every, section: 'sources', presets: [claude, codex], checkSource: check);
      expect(find.byKey(const Key('source-version Claude')), findsOneWidget);
      expect(find.text('v2.1.3'), findsOneWidget);
      expect(find.text('Ready'), findsWidgets);
      expect(find.text('Needs codex'), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('source-row Mine')), matching: find.textContaining('my-agent')), findsNothing, reason: 'a program and its arguments are in the detail, not the row');
      await tester.tap(find.byKey(const Key('source-row Mine')));
      await tester.pumpAndSettle();
      expect(find.text('my-agent --acp'), findsOneWidget);

      await tester.tap(find.byKey(const Key('source-offered Codex')));
      await tester.pumpAndSettle();
      expect(h.last.hiddenSources, {'Pi', 'Codex'});
      expect(changedTables(every, h.last), {'sources'});
    });

    testWidgets('adding and removing a command keeps every other table', (tester) async {
      final h = await openSettings(tester, config: every, section: 'sources', presets: [claude], checkSource: check);
      await tester.enterText(find.byKey(const Key('source-field-name')), 'Other');
      await tester.enterText(find.byKey(const Key('source-field-program')), 'other');
      await tester.enterText(find.byKey(const Key('source-field-args')), '--acp');
      await tester.tap(find.byKey(const Key('source-add')));
      await tester.pumpAndSettle();
      expect(h.last.agents.map((agent) => agent.label), ['Mine', 'Other']);
      expect(changedTables(every, h.last), {'agents'});
      expect(tomlOf(h.last)['agents'], containsPair('Mine', isA<Map>().having((m) => m.keys, 'its profile, mcp and hooks', containsAll(['profile', 'mcp', 'hooks']))));

      await tester.tap(find.byKey(const Key('source-row Other')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('source-remove Other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('confirm-remove')), findsOneWidget, reason: 'removing asks first');
      await tester.tap(find.byKey(const Key('confirm-remove-no')));
      await tester.pumpAndSettle();
      expect(h.last.agents.map((agent) => agent.label), ['Mine', 'Other'], reason: 'keeping it changes nothing');
      await tester.tap(find.byKey(const Key('source-remove Other')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-yes')));
      await tester.pumpAndSettle();
      expect(h.last.agents.map((agent) => agent.label), ['Mine']);
      expect(changedTables(every, h.last), isEmpty);
    });

    testWidgets('refuses a name taken and a missing program, and applies nothing', (tester) async {
      final h = await openSettings(tester, section: 'sources', presets: [claude], checkSource: check);
      await tester.enterText(find.byKey(const Key('source-field-name')), 'Claude');
      await tester.enterText(find.byKey(const Key('source-field-program')), 'x');
      await tester.tap(find.byKey(const Key('source-add')));
      await tester.pumpAndSettle();
      expect(find.text('Another source is called Claude.'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('source-field-name')), 'New');
      await tester.enterText(find.byKey(const Key('source-field-program')), '');
      await tester.tap(find.byKey(const Key('source-add')));
      await tester.pumpAndSettle();
      expect(find.text('Program is needed'), findsOneWidget);
      expect(h.applied, isEmpty);
    });
  });

  group('Profiles', () {
    testWidgets('are shown for the agent chosen in the scope bar, beside the layer each value comes from', (tester) async {
      await openSettings(tester, config: every, section: 'profiles', agents: const ['Mine', 'Codex']);
      expect(find.text('Applying settings for agent '), findsOneWidget);
      expect(find.text('Effective profile'), findsOneWidget);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('profile-layer-model')), findsOneWidget, reason: 'Mine has its own model');
      await choose(tester, 'profile-agent', 'Codex');
      expect(find.byKey(const Key('profile-layer-model')), findsNothing, reason: 'Codex has none');
    });
  });

  group('MCP servers', () {
    testWidgets('switch on or off for the agent chosen, and a choice for one is not a choice for the other', (tester) async {
      final h = await openSettings(tester, config: every, section: 'mcp', agents: const ['Claude', 'Mine']);
      expect(find.text('Program: own-mcp --serve'), findsOneWidget);
      final own = find.byKey(const Key('mcp-own'));
      expect(tester.widget<SwitchRow>(own).value, isTrue, reason: 'on for Claude: nothing turned it off');
      await tester.tap(find.descendant(of: own, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(h.last.agentMcp['Claude'], {'own': false});
      expect(h.last.agentMcp['Mine'], {'own': false}, reason: 'the file had it off for Mine and still does');
      expect(changedTables(every, h.last), {'agents'});

      await choose(tester, 'scope-bar', 'Mine');
      expect(tester.widget<SwitchRow>(own).value, isFalse);
      await tester.tap(find.descendant(of: own, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(h.last.agentMcp['Mine'], {'own': true});
      expect(h.last.agentMcp['Claude'], {'own': false});
      expect(CanoaConfig.parse(h.last.encode()).agentMcp, h.last.agentMcp);
      expect(tomlOf(h.last)['profile'], tomlOf(every)['profile']);
    });
  });

  group('Permissions', () {
    testWidgets('holds the mode, Bypass and its log, and round-trips them', (tester) async {
      final rules = PermissionRules(MemoryLedgerBackend(), log: MemoryRuleLog());
      final h = await openSettings(tester, config: every, section: 'permissions', rules: rules);
      expect(find.byKey(const Key('rules-log-location')), findsOneWidget);
      expect(find.text('memory only'), findsOneWidget);
      await choose(tester, 'settings-default-mode', 'Bypass');
      expect(h.last.defaultMode, 'bypass');
      await tester.tap(find.descendant(of: find.byKey(const Key('settings-allow-once-only')), matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(h.last.allowOnceOnly, isFalse);
      expect(changedTables(every, h.last), {'permissions'});
      expect(h.last.protectedPaths, ['secrets', '!secrets/ok'], reason: 'the protected list is kept as it was');
      expect(CanoaConfig.parse(h.last.encode()).defaultMode, 'bypass');
    });

    testWidgets('lists the remembered rules of the source chosen in the scope bar', (tester) async {
      final rules = PermissionRules(MemoryLedgerBackend());
      final one = rules.add(source: 'Claude', folder: '/a', kind: 'execute', type: RuleType.command, value: 'npm test')!;
      final two = rules.add(source: 'Codex', folder: '/b', kind: 'execute', type: RuleType.command, value: 'ls')!;
      await openSettings(tester, section: 'permissions', rules: rules);
      expect(find.byKey(Key('rule-${one.id}')), findsOneWidget);
      expect(find.byKey(Key('rule-${two.id}')), findsOneWidget);
      await choose(tester, 'scope-bar', 'Codex');
      expect(find.byKey(Key('rule-${one.id}')), findsNothing);
      expect(find.byKey(Key('rule-${two.id}')), findsOneWidget);
    });
  });

  group('Usage', () {
    testWidgets('sets when the usage control warns, and keeps usage.limit_phrases', (tester) async {
      final h = await openSettings(tester, config: every, section: 'usage');
      await choose(tester, 'settings-warn', '90%');
      expect(h.last.warnPermille, 900);
      expect(h.last.limitPhrases, ['rate limit']);
      expect(changedTables(every, h.last), {'usage'});
      expect(tomlOf(h.last)['usage'], {'warn_permille': 900, 'limit_phrases': ['rate limit']});
    });
  });

  group('Store', () {
    testWidgets('Settings lists no catalogs: they are each Store type\'s, and the section opens the Store', (tester) async {
      var opened = 0;
      await openSettings(tester, config: every, section: 'store', onLoadout: () => opened++);
      expect(find.text('https://example.invalid/catalog.git@main#catalog.json'), findsNothing);
      for (final key in const ['catalog-main', 'catalog-label', 'catalog-spec', 'catalog-add', 'limit-catalog_max_bytes']) {
        expect(find.byKey(Key(key)), findsNothing, reason: key);
      }
      expect(find.byKey(const Key('settings-nav-catalogs')), findsNothing);
      await tester.tap(find.byKey(const Key('settings-open-store')));
      expect(opened, 1);
    });

    testWidgets('searching for catalogs, sources or registries finds the Store section', (tester) async {
      await openSettings(tester);
      for (final word in const ['catalog', 'registries', 'skills']) {
        await tester.enterText(find.byKey(const Key('settings-search')), word);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('settings-nav-store')), findsOneWidget, reason: word);
      }
    });
  });

  testWidgets('About names the build', (tester) async {
    await openSettings(tester, section: 'about');
    expect(find.byKey(const Key('about-version')), findsOneWidget);
    expect(find.textContaining('Boriken Dev'), findsOneWidget);
  });

  group('in the app', () {
    testWidgets('Ctrl+, opens Settings, and a change there reaches canoa.cfg at once', (tester) async {
      final written = await pumpApp(tester, SessionStore(), config: every);
      expect(find.byKey(const Key('settings-nav-general')), findsNothing);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.comma);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await frames(tester);
      expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
      await tester.tap(find.byKey(const Key('settings-nav-usage')));
      await frames(tester);
      await choose(tester, 'settings-warn', '90%');
      await frames(tester);
      expect(written.last.warnPermille, 900);
      expect(changedTables(every, written.last), {'usage'});
      await finish(tester);
    });
  });
}
