// Settings' Providers page over a scripted host: every provider the libraries register, by kind, each with the same switch the Catalog
// sources page uses; turning one off is one `providers/switch` call and says the host restarts to apply it; the accounts of an account
// provider that is off are counted, and what `[host.disabled]` names that no library offers is said. Goldens are drawn in the test
// font; with CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/off_switch.dart';
import 'package:canoa/settings/provider_switches.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

/// A host that keeps `[host.disabled]` and answers `providers/registered`, `providers/switch` and `accounts/list` from it.
final class ScriptedProviders {
  ScriptedProviders({Set<String> off = const {}, this.unknown = const [], this.deny = false}) : off = {...off};

  /// The providers that are off, as `kind/id`.
  final Set<String> off;
  final List<String> unknown;

  /// Whether the key may not change the host's own settings.
  final bool deny;

  final calls = <(String, Map<String, Object?>)>[];

  static const _providers = [
    ('account_provider', 'claude_accounts', 'Claude accounts', 'canoa-claude', ['claude-login']),
    ('account_provider', 'litellm_accounts', 'LiteLLM', 'canoa-litellm', ['litellm']),
    ('acp_harness', 'claude', 'Claude', 'canoa-claude', <String>[]),
    ('store_extension', 'whisper_extension', 'whisper.cpp', 'canoa-whispercpp', <String>[]),
    ('speech_output', 'speech_say_piper', 'Piper', 'canoa-piper', <String>[]),
  ];

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    if (deny) throw ControlFailed(-32001, 'this key does not hold hosting');
    switch (method) {
      case 'providers/registered':
        return {
          'providers': [
            for (final (kind, id, title, library, kinds) in _providers)
              {'kind': kind, 'id': id, 'title': title, 'library': library, 'origin': 'bundled', 'off': off.contains('$kind/$id'), if (kinds.isNotEmpty) 'kinds': kinds},
          ],
          'unknown': unknown,
          'disabled': const {},
        };
      case 'accounts/list':
        return {
          'accounts': [
            for (final name in ['work', 'old']) {'name': name, 'kind': 'claude-login', 'disabled': off.contains('account_provider/claude_accounts') ? 'claude_accounts' : null},
            {'name': 'proxy', 'kind': 'litellm', 'disabled': null},
          ],
        };
      case 'providers/switch':
        final key = '${params['kind']}/${params['id']}';
        if (params['off'] == true) {
          off.add(key);
        } else {
          off.remove(key);
        }
        return {'ok': true, 'restart': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/providers_switch_test.dart')));

  group('the controller', () {
    test('lists the providers by kind and counts the accounts a provider that is off puts out of reach', () async {
      final host = ScriptedProviders(off: {'account_provider/claude_accounts'});
      final switches = ProviderSwitches(host.call);
      await switches.load();
      expect(switches.byKind.map((each) => each.$1), ['account_provider', 'acp_harness', 'speech_output', 'store_extension']);
      expect(switches.byKind.first.$2.map((provider) => provider.id), ['claude_accounts', 'litellm_accounts']);
      expect(switches.byKind.first.$2.first.accountKinds, ['claude-login']);
      expect(switches.accountsOff('claude_accounts'), 2, reason: 'two accounts of the kind stay in canoa.cfg, shown as off');
      expect(switches.accountsOff('litellm_accounts'), 0);
      switches.dispose();
    });

    test('a key that may not change the host says so and lists nothing', () async {
      final switches = ProviderSwitches(ScriptedProviders(deny: true).call);
      await switches.load();
      expect(switches.providers, isEmpty);
      expect(switches.problem, contains('may not change the host'));
      switches.dispose();
    });
  });

  group('the page', () {
    Future<(ScriptedProviders, ProviderSwitches)> open(WidgetTester tester, {ScriptedProviders? host, Size size = const Size(1100, 1800), Future<bool> Function()? restart}) async {
      final scripted = host ?? ScriptedProviders();
      final switches = ProviderSwitches(scripted.call, restart: restart);
      await openSettings(tester, section: 'providers', size: size, providers: switches);
      await tester.pumpAndSettle();
      return (scripted, switches);
    }

    testWidgets('lists every provider under its kind with a switch that is on', (tester) async {
      await open(tester);
      for (final kind in ['account_provider', 'acp_harness', 'store_extension', 'speech_output']) {
        expect(find.byKey(Key('providers-kind-$kind')), findsOneWidget, reason: kind);
      }
      final claude = find.byKey(const Key('provider-switch-account_provider-claude_accounts'));
      expect(tester.widget<OffSwitch>(claude).on, isTrue);
      expect(find.byType(OffBadge), findsNothing);
      expect(find.byKey(const Key('providers-restart-note')), findsNothing);
      expect(find.descendant(of: find.byKey(const Key('provider-row-account_provider-claude_accounts')), matching: find.textContaining('claude-login')), findsOneWidget);
    });

    testWidgets('turning one off is one providers/switch call, marks it off and says the host restarts', (tester) async {
      var restarts = 0;
      final (host, _) = await open(tester, restart: () async => ++restarts > 0);
      await tester.tap(find.descendant(of: find.byKey(const Key('provider-switch-account_provider-claude_accounts')), matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'providers/switch').single.$2, {'kind': 'account_provider', 'id': 'claude_accounts', 'off': true});
      final row = find.byKey(const Key('provider-row-account_provider-claude_accounts'));
      expect(find.descendant(of: row, matching: find.byType(OffBadge)), findsOneWidget);
      expect(find.byKey(const Key('providers-restart-note')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('provider-row-acp_harness-claude')), matching: find.byType(OffBadge)), findsNothing, reason: 'the harness stays');

      await tester.tap(find.byKey(const Key('providers-restart')));
      await tester.pumpAndSettle();
      expect(restarts, 1);
      expect(find.byKey(const Key('providers-restart-note')), findsNothing);
      // The accounts it put out of reach are counted on the row once the page reads again.
      expect(find.descendant(of: row, matching: find.textContaining('2 accounts of this provider stay in canoa.cfg')), findsOneWidget);
    });

    testWidgets('a provider that is off is turned on with the same switch', (tester) async {
      final (host, _) = await open(tester, host: ScriptedProviders(off: {'store_extension/whisper_extension'}));
      final key = find.byKey(const Key('provider-switch-store_extension-whisper_extension'));
      expect(tester.widget<OffSwitch>(key).on, isFalse);
      await tester.tap(find.descendant(of: key, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'providers/switch').single.$2['off'], false);
      expect(find.byType(OffBadge), findsNothing);
    });

    testWidgets('what [host.disabled] names that no library offers is listed', (tester) async {
      await open(tester, host: ScriptedProviders(unknown: ['[host.disabled] account_provider names "gone_accounts", which no library offers']));
      expect(find.textContaining('gone_accounts'), findsOneWidget);
    });
  });

  testWidgets('both pages draw one switch component', (tester) async {
    await openSettings(tester, section: 'providers', providers: ProviderSwitches(ScriptedProviders().call));
    await tester.pumpAndSettle();
    expect(find.byType(OffSwitch), findsWidgets);
  });

  for (final (label, size) in [('desktop', const Size(1280, 1000)), ('phone', const Size(390, 1000))]) {
    testWidgets('golden at $label width', (tester) async {
      final host = ScriptedProviders(off: {'account_provider/claude_accounts', 'speech_output/speech_say_piper'}, unknown: ['[host.disabled] account_provider names "gone_accounts", which no library offers; the providers of that kind are claude_accounts, litellm_accounts']);
      final switches = ProviderSwitches(host.call);
      await openSettings(tester, section: 'providers', size: size, providers: switches);
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/providers_$label.png'));
    });
  }
}
