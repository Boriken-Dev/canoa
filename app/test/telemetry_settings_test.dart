// Settings' Telemetry page over a scripted host: the list of what the host records and what it never does comes first, before any
// switch; the switch cannot be turned on with no exporter; accepting writes the list's digest and the switch in one change; a list
// that changed pauses export and asks again; exporters are added with the form their provider describes and removed after asking.
// Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/telemetry_settings.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

const _digest = '3f2a9c41d07be5a6';

/// A host that keeps `[host.telemetry]` and answers `telemetry/catalog`, `telemetry/status`, `telemetry/test` and `config/change`.
final class ScriptedTelemetry {
  ScriptedTelemetry({
    this.acknowledged = '',
    this.enabled = false,
    List<String> exporters = const [],
    this.deny = false,
    this.testFails,
  }) : exporters = {for (final name in exporters) name: 'file'};

  String acknowledged;
  bool enabled;
  final Map<String, String> exporters;
  List<String> record = ['spans', 'metrics', 'logs'];
  final bool deny;

  /// What a test record's failure says, when it fails.
  final String? testFails;

  final calls = <(String, Map<String, Object?>)>[];

  List<Map<String, Object?>> get writes => [
    for (final call in calls)
      if (call.$1 == 'config/change') for (final op in call.$2['ops']! as List) (op as Map).cast<String, Object?>(),
  ];

  String get _state => !enabled
      ? 'off'
      : acknowledged != _digest
      ? 'paused'
      : exporters.isEmpty
      ? 'paused'
      : 'running';

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    if (deny) throw ControlFailed(-32001, 'only the owner');
    switch (method) {
      case 'telemetry/catalog':
        return {
          'digest': _digest,
          'signals': [
            {
              'name': 'canoa.turn',
              'group': 'spans',
              'kind': 'span',
              'unit': '',
              'description': 'One turn of a session: how it ended, the model, and the tokens and cost the agent reported.',
              'attributes': [
                {'name': 'canoa.stop_reason', 'type': 'choice', 'values': ['end_turn', 'cancelled', 'error'], 'optional': false},
                {'name': 'canoa.model', 'type': 'text', 'optional': true},
                {'name': 'canoa.tokens.input', 'type': 'int', 'optional': true},
              ],
            },
            {'name': 'canoa.turns', 'group': 'metrics', 'kind': 'counter', 'unit': '{turn}', 'description': 'Turns, by agent and how they ended.', 'attributes': []},
            {'name': 'host.started', 'group': 'logs', 'kind': 'event', 'unit': '', 'description': 'The host started, and its version.', 'attributes': []},
          ],
          'never': ['The words of the person or of the agent.', 'File names, paths and contents.', 'Keys, tokens, passwords and any other secret.'],
          'resource': ['service.name'],
          'text_rule': 'Text from an agent is cut to 64 characters.',
        };
      case 'telemetry/status':
        return {
          'enabled': enabled,
          'state': _state,
          'paused': _state == 'paused' ? (acknowledged == _digest ? 'no exporter is named' : 'the list of what is recorded changed since it was accepted') : null,
          'digest': _digest,
          'acknowledged': acknowledged,
          'record': record,
          'exporters': [
            for (final entry in exporters.entries) {'name': entry.key, 'provider': entry.value, 'state': _state == 'running' ? 'ok' : 'stopped', 'queued': 3, 'dropped': 1},
          ],
          'providers': [
            {
              'id': 'file',
              'title': 'Files',
              'schema': {
                'type': 'object',
                'properties': {
                  'folder': {'type': ['string', 'null'], 'title': 'Folder'},
                  'keep_days': {'type': ['integer', 'null'], 'title': 'Keep days'},
                },
              },
            },
          ],
        };
      case 'telemetry/test':
        return testFails == null ? {'ok': true} : {'ok': false, 'why': testFails};
      case 'config/change':
        for (final op in params['ops']! as List) {
          final path = ((op as Map)['path'] as List).cast<String>();
          final value = op['value'];
          if (path.length == 3 && path[2] == 'enabled') enabled = value as bool;
          if (path.length == 3 && path[2] == 'acknowledged') acknowledged = value as String;
          if (path.length == 3 && path[2] == 'record') record = (value as List).cast<String>();
          if (path.length == 4) {
            if (value == null) {
              exporters.remove(path[3]);
            } else {
              exporters[path[3]] = (value as Map)['provider'] as String;
            }
          }
        }
        return {'revision': 'r', 'restart': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/telemetry_settings_test.dart')));

  group('the controller', () {
    test('reads the list, the state and the exporters', () async {
      final host = ScriptedTelemetry(acknowledged: _digest, enabled: true, exporters: ['local']);
      final settings = TelemetrySettings(host.call);
      await settings.load();
      expect(settings.digest, _digest);
      expect(settings.accepted, isTrue);
      expect(settings.state, 'running');
      expect(settings.signalsOf('spans').single.attributes.map((each) => each.name), ['canoa.stop_reason', 'canoa.model', 'canoa.tokens.input']);
      expect(settings.signalsOf('spans').single.attributes.last.optional, isTrue);
      expect(settings.exporters.single.name, 'local');
      expect(settings.providers.single.id, 'file');
      settings.dispose();
    });

    test('a key that may not read it says so and lists nothing', () async {
      final settings = TelemetrySettings(ScriptedTelemetry(deny: true).call);
      await settings.load();
      expect(settings.signals, isEmpty);
      expect(settings.problem, contains('only the person at this computer'));
      settings.dispose();
    });
  });

  group('the page', () {
    Future<(ScriptedTelemetry, TelemetrySettings)> open(WidgetTester tester, {ScriptedTelemetry? host, Size size = const Size(1100, 3200), Future<bool> Function()? restart}) async {
      final scripted = host ?? ScriptedTelemetry();
      final settings = TelemetrySettings(scripted.call, restart: restart);
      await openSettings(tester, section: 'telemetry', size: size, telemetry: settings);
      await tester.pumpAndSettle();
      return (scripted, settings);
    }

    Switch enabledSwitch(WidgetTester tester) => tester.widget<Switch>(find.byKey(const Key('telemetry-enabled')));

    testWidgets('lists what is recorded and what is never recorded before any switch', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('telemetry-signal-canoa.turn')), findsOneWidget);
      expect(find.text('canoa.tokens.input?: whole number'), findsNothing, reason: 'the attributes are one text under the signal');
      expect(find.textContaining('canoa.tokens.input?: whole number'), findsOneWidget);
      expect(find.textContaining('canoa.stop_reason: end_turn | cancelled | error'), findsOneWidget);
      expect(find.text('The words of the person or of the agent.'), findsOneWidget);
      final recorded = tester.getTopLeft(find.byKey(const Key('telemetry-recorded'))).dy;
      final never = tester.getTopLeft(find.byKey(const Key('telemetry-never'))).dy;
      final switchAt = tester.getTopLeft(find.byKey(const Key('telemetry-enabled'))).dy;
      expect(recorded, lessThan(never));
      expect(never, lessThan(switchAt), reason: 'both lists are above the switch');
    });

    testWidgets('the switch is off, and cannot be turned on while there is no exporter', (tester) async {
      final (host, _) = await open(tester);
      expect(enabledSwitch(tester).value, isFalse);
      expect(enabledSwitch(tester).onChanged, isNull);
      expect(find.byKey(const Key('telemetry-needs-exporter')), findsOneWidget);
      expect(host.writes, isEmpty);
    });

    testWidgets('turning it on asks for the read, and accepting writes the digest and the switch in one change', (tester) async {
      final (host, _) = await open(tester, host: ScriptedTelemetry(exporters: ['local']));
      await tester.tap(find.byKey(const Key('telemetry-enabled')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('telemetry-digest')), findsOneWidget);
      expect(find.textContaining(_digest), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('telemetry-accept'))).onPressed, isNull, reason: 'not before it is read');
      expect(host.writes, isEmpty);
      await tester.tap(find.byKey(const Key('telemetry-read')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-accept')));
      await tester.pumpAndSettle();
      final changes = host.calls.where((call) => call.$1 == 'config/change').toList();
      expect(changes, hasLength(1), reason: 'one write, so the two never disagree');
      expect(host.writes, [
        {'path': ['host', 'telemetry', 'acknowledged'], 'value': _digest},
        {'path': ['host', 'telemetry', 'enabled'], 'value': true},
      ]);
      expect(enabledSwitch(tester).value, isTrue);
      expect(find.byKey(const Key('telemetry-restart-note')), findsOneWidget);
    });

    testWidgets('a list that changed since it was accepted pauses export, and Review asks again', (tester) async {
      final (host, _) = await open(tester, host: ScriptedTelemetry(acknowledged: 'ffffffffffffffff', enabled: true, exporters: ['local']));
      expect(find.byKey(const Key('telemetry-paused')), findsOneWidget);
      expect(find.text('Paused: the list of what is recorded changed'), findsOneWidget);
      expect(enabledSwitch(tester).value, isFalse, reason: 'it is not running');
      await tester.tap(find.byKey(const Key('telemetry-review')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-read')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-accept')));
      await tester.pumpAndSettle();
      expect(host.acknowledged, _digest);
      expect(find.byKey(const Key('telemetry-paused')), findsNothing);
      expect(enabledSwitch(tester).value, isTrue);
    });

    testWidgets('turning it off writes the switch alone', (tester) async {
      final (host, _) = await open(tester, host: ScriptedTelemetry(acknowledged: _digest, enabled: true, exporters: ['local']));
      expect(enabledSwitch(tester).value, isTrue);
      await tester.tap(find.byKey(const Key('telemetry-enabled')));
      await tester.pumpAndSettle();
      expect(host.writes, [
        {'path': ['host', 'telemetry', 'enabled'], 'value': false},
      ]);
      expect(enabledSwitch(tester).value, isFalse);
    });

    testWidgets('the groups are three checkboxes, each written as the list that is left', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('telemetry-record-metrics')));
      await tester.pumpAndSettle();
      expect(host.writes.single, {'path': ['host', 'telemetry', 'record'], 'value': ['spans', 'logs']});
    });

    testWidgets('an exporter is added with the form its provider describes and removed after asking', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('telemetry-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-new-add')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('telemetry-new-problem')), findsOneWidget, reason: 'a name is needed');
      await tester.enterText(find.byKey(const Key('telemetry-new-name')), 'local');
      await tester.enterText(find.byKey(const Key('telemetry-new-field-folder')), 'C:/telemetry');
      await tester.tap(find.byKey(const Key('telemetry-new-add')));
      await tester.pumpAndSettle();
      expect(host.writes.single, {
        'path': ['host', 'telemetry', 'exporters', 'local'],
        'value': {'provider': 'file', 'settings': {'folder': 'C:/telemetry'}},
      });
      expect(find.byKey(const Key('telemetry-exporter-local')), findsOneWidget);

      await tester.tap(find.byKey(const Key('telemetry-remove-local')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-remove-cancel')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('telemetry-exporter-local')), findsOneWidget, reason: 'asked, and said no');
      await tester.tap(find.byKey(const Key('telemetry-remove-local')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-remove-confirm')));
      await tester.pumpAndSettle();
      expect(host.writes.last, {'path': ['host', 'telemetry', 'exporters', 'local']});
      expect(find.byKey(const Key('telemetry-exporter-local')), findsNothing);
    });

    testWidgets('an exporter that was named is tested and says how it went', (tester) async {
      final (host, _) = await open(tester, host: ScriptedTelemetry(exporters: ['local'], testFails: 'the folder could not be made'));
      expect(find.textContaining('3 waiting, 1 let go'), findsOneWidget);
      await tester.tap(find.byKey(const Key('telemetry-test-local')));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'telemetry/test').single.$2, {'name': 'local'});
      expect(find.textContaining('did not reach local: the folder could not be made'), findsOneWidget);
    });

    testWidgets('a restart is offered once something was written', (tester) async {
      var restarts = 0;
      await open(tester, host: ScriptedTelemetry(exporters: ['local']), restart: () async => ++restarts > 0);
      await tester.tap(find.byKey(const Key('telemetry-record-logs')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('telemetry-restart')));
      await tester.pumpAndSettle();
      expect(restarts, 1);
      expect(find.byKey(const Key('telemetry-restart-note')), findsNothing);
    });

    testWidgets('a key that may not read it is told so and sees no list', (tester) async {
      await open(tester, host: ScriptedTelemetry(deny: true));
      expect(find.byKey(const Key('telemetry-problem')), findsOneWidget);
      expect(find.byKey(const Key('telemetry-recorded')), findsNothing);
    });
  });

  for (final (label, size) in [('desktop', const Size(1280, 3000)), ('phone', const Size(390, 3600))]) {
    testWidgets('golden at $label width', (tester) async {
      final host = ScriptedTelemetry(acknowledged: _digest, enabled: true, exporters: ['local']);
      final settings = TelemetrySettings(host.call);
      await openSettings(tester, section: 'telemetry', size: size, telemetry: settings);
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/telemetry_$label.png'));
    });
  }
}
