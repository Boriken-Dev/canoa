// `[permissions.approvers]`: how canoa.cfg holds them, and the Approvers group of Settings > Permissions, in both languages.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/base_text.dart';
import 'package:canoa/permissions/approvers.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';

const _file = '''
[permissions.approvers.rules]
provider = "policy"
settings = { rules = [ { kind = "read", verdict = "allow" }, { command = "git status*", verdict = "allow" } ] }

[permissions.approvers.phone]
provider = "notified"
notifier = "ntfy"
detail = "command"
timeout_seconds = 300
only_when_away = true

[permissions.approvers.window]
timeout_seconds = 600
''';

void main() {
  group('canoa.cfg', () {
    test('the approvers are read in the file\'s order, with the window\'s timeout', () {
      final approvers = CanoaConfig.parse(_file).approvers;
      expect([for (final entry in approvers.entries) entry.name], ['rules', 'phone']);
      final rules = approvers.entries.first;
      expect(rules.provider, 'policy');
      expect(rules.detail, 'tool');
      expect((rules.settings['rules'] as List).length, 2);
      final phone = approvers.entries.last;
      expect((phone.notifier, phone.detail, phone.timeoutSeconds, phone.onlyWhenAway), ('ntfy', 'command', 300, true));
      expect(approvers.windowTimeoutSeconds, 600);
    });

    test('they are written back as they were read, and a file with none writes none', () {
      final config = CanoaConfig.parse(_file);
      expect(CanoaConfig.parse(config.encode()).approvers, config.approvers);
      expect(const CanoaConfig().encode(), isNot(contains('approvers')));
      expect(CanoaConfig.parse(const CanoaConfig().encode()).approvers.isEmpty, isTrue);
    });

    test('they sit beside the other permissions without disturbing them', () {
      final config = CanoaConfig.parse('[permissions]\ndefault_mode = "plan"\nprotected = ["secrets"]\n$_file');
      final again = CanoaConfig.parse(config.encode());
      expect(again.defaultMode, 'plan');
      expect(again.protectedPaths, ['secrets']);
      expect(again.approvers, config.approvers);
    });

    test('a name, a provider, a detail or a timeout that is wrong is refused', () {
      for (final text in [
        '[permissions.approvers.Bad]\nprovider = "policy"\n',
        '[permissions.approvers.a]\n',
        '[permissions.approvers.a]\nprovider = "policy"\ndetail = "all"\n',
        '[permissions.approvers.a]\nprovider = "policy"\ntimeout_seconds = 0\n',
        '[permissions.approvers.window]\ntimeout_seconds = 0\n',
        '[permissions]\napprovers = 3\n',
      ]) {
        expect(() => CanoaConfig.parse(text), throwsFormatException, reason: text);
      }
    });

    test('inline settings keep their numbers, flags, text and nesting', () {
      const settings = {'a': 1, 'b': true, 'c': 'x"y', 'd': [1, 'z', {'e': 2}], 'f': <String, Object?>{}};
      const entry = ApproverEntry(name: 'x', provider: 'p', settings: settings);
      final read = CanoaConfig.parse(CanoaConfig(approvers: Approvers(entries: [entry])).encode()).approvers;
      expect(read.entries.single.settings, settings);
    });
  });

  group('Settings > Permissions', () {
    Future<SettingsHarness> open(WidgetTester tester, {CanoaConfig config = const CanoaConfig()}) => openSettings(
      tester,
      config: config,
      rules: PermissionRules(MemoryLedgerBackend(), real: (path) => path),
      section: 'permissions',
      size: const Size(1100, 3200),
    );

    testWidgets('says that nothing answers for the person unless an approver is added', (tester) async {
      await open(tester);
      expect(find.textContaining('Nothing answers a request for you'), findsOneWidget);
      expect(find.textContaining('refused'), findsWidgets);
      expect(find.byKey(const Key('approver-rules')), findsNothing);
    });

    testWidgets('lists the approvers and changes what each is shown, how long it is waited for and when it is asked', (tester) async {
      final settings = await open(tester, config: CanoaConfig.parse(_file));
      expect(find.byKey(const Key('approver-rules')), findsOneWidget);
      expect(find.byKey(const Key('approver-phone')), findsOneWidget);

      await tester.tap(find.byKey(const Key('approver-detail-rules')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Also the command and files').last);
      await tester.pumpAndSettle();
      expect(settings.last.approvers.entries.first.detail, 'command');

      await tester.tap(find.descendant(of: find.byKey(const Key('approver-away-rules')), matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(settings.last.approvers.entries.first.onlyWhenAway, isTrue);

      await tester.tap(find.byKey(const Key('approver-window-timeout')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Until the turn ends').last);
      await tester.pumpAndSettle();
      expect(settings.last.approvers.windowTimeoutSeconds, isNull);
      expect(settings.last.approvers.entries.map((entry) => entry.name), ['rules', 'phone'], reason: 'the others are as they were');
    });

    testWidgets('adds an approver from its name, its provider and its settings, and refuses a bad name or settings', (tester) async {
      final settings = await open(tester);
      await tester.tap(find.byKey(const Key('approver-add')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('approver-name')), 'Bad Name');
      await tester.tap(find.byKey(const Key('approver-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('approver-problem')), findsOneWidget);
      await tester.enterText(find.byKey(const Key('approver-name')), 'rules');
      await tester.enterText(find.byKey(const Key('approver-settings')), '[1]');
      await tester.tap(find.byKey(const Key('approver-save')));
      await tester.pumpAndSettle();
      expect(find.text('The settings are a JSON object.'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('approver-settings')), '{"rules": [{"kind": "read", "verdict": "allow"}]}');
      await tester.tap(find.byKey(const Key('approver-save')));
      await tester.pumpAndSettle();
      final entry = settings.last.approvers.entries.single;
      expect((entry.name, entry.provider), ('rules', 'policy'));
      expect(entry.settings, {'rules': [{'kind': 'read', 'verdict': 'allow'}]});
    });

    testWidgets('removes an approver only when the person confirms', (tester) async {
      final settings = await open(tester, config: CanoaConfig.parse(_file));
      await tester.tap(find.byKey(const Key('approver-remove-phone')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-no')));
      await tester.pumpAndSettle();
      expect(settings.applied, isEmpty);
      await tester.tap(find.byKey(const Key('approver-remove-phone')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-yes')));
      await tester.pumpAndSettle();
      expect(settings.last.approvers.entries.map((entry) => entry.name), ['rules']);
    });
  });

  test('every word of the group is in English and in Spanish', () {
    final english = baseStrings['en']!;
    final spanish = baseStrings['es']!;
    final keys = english.keys.where((key) => key.startsWith('approvers.')).toList();
    expect(keys, isNotEmpty);
    for (final key in keys) {
      expect(spanish[key], isNotNull, reason: 'es lacks $key');
    }
    expect(spanish.keys.where((key) => key.startsWith('approvers.')).toSet(), keys.toSet());
    expect(const AppText(language: 'es')('approvers.title'), 'Aprobadores');
    expect(const AppText()('approvers.title'), 'Approvers');
    expect(const AppText(language: 'es')('approvers.removing', {'name': 'phone'}), '¿Quitar phone?');
  });
}
