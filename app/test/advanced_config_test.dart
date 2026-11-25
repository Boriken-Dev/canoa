// Settings' Advanced page over a scripted host: the tables of canoa.cfg no other page edits, each a form made from its schema, with an
// entry added, changed and removed as one `config/advanced_set` call, a free-form member shown as JSON, an entry a lower layer gives
// shown with where it comes from and no Remove, and the host's refusal shown in its own words. Goldens are drawn in the test font.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/advanced_config.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

/// A host that keeps the tables the person's own file holds and one entry a lower layer gives, and answers the two methods.
final class ScriptedAdvanced {
  ScriptedAdvanced({this.refuse});

  String? refuse;
  final calls = <(String, Map<String, Object?>)>[];

  /// The person's own tables by path.
  final own = <String, Object?>{};

  static Map<String, Object?> _prop(String type, String title, {Map<String, Object?> extra = const {}}) => {'type': type, 'title': title, ...extra};

  static final _web = {
    'type': 'object',
    'properties': {
      'listen': _prop('string', 'Listen'),
      'office': _prop('boolean', 'Office'),
      'hosts': {'type': 'array', 'title': 'Hosts', 'items': {'type': 'string'}},
      'max_connections': _prop('integer', 'Max connections'),
    },
  };

  static final _tools = {
    'type': 'object',
    'properties': {
      'program': _prop('string', 'Program'),
      'runs': _prop('string', 'Runs'),
      'prompt': _prop('string', 'Prompt', extra: {'x-json': true}),
    },
  };

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'config/advanced':
        return {
          'revision': 'r${calls.length}',
          'tables': [
            {'path': ['host', 'web'], 'title': '[host.web]', 'shape': 'flat', 'schema': _web, 'own': own['host.web'], 'inherited': {'mode': {'layer': 'the system file', 'value': 'local'}}},
            {
              'path': ['host', 'tools'],
              'title': '[host.tools]',
              'shape': 'entries',
              'schema': _tools,
              'own': own['host.tools'],
              'inherited': {'shared': {'layer': 'the system file', 'value': {'program': 'shared'}}},
            },
          ],
        };
      case 'config/advanced_set':
        if (refuse != null) throw ControlFailed(-32602, refuse!);
        final path = [for (final key in params['path']! as List) '$key'].join('.');
        if (params['value'] == null) {
          own.remove(path);
        } else if (path.startsWith('host.tools.')) {
          final tools = {...(own['host.tools'] as Map<String, Object?>? ?? {})};
          tools[path.substring('host.tools.'.length)] = params['value'];
          own['host.tools'] = tools;
        } else {
          own[path] = params['value'];
        }
        if (params['value'] == null && path.startsWith('host.tools.')) {
          final tools = {...(own['host.tools'] as Map<String, Object?>? ?? {})}..remove(path.substring('host.tools.'.length));
          if (tools.isEmpty) {
            own.remove('host.tools');
          } else {
            own['host.tools'] = tools;
          }
        }
        return {'path': params['path'], 'revision': 'x', 'restart': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }

  List<Map<String, Object?>> sets() => [for (final (method, params) in calls) if (method == 'config/advanced_set') params];
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/advanced_config_test.dart')));

  Future<ScriptedAdvanced> open(WidgetTester tester, {ScriptedAdvanced? scripted, Size size = const Size(1100, 2000)}) async {
    final host = scripted ?? ScriptedAdvanced();
    await openSettings(tester, section: 'advanced', size: size, advancedConfig: AdvancedConfig(host.call));
    await tester.pumpAndSettle();
    return host;
  }

  testWidgets('lists each table by its schema, and what a lower layer gives with where it comes from', (tester) async {
    await open(tester);
    expect(find.byKey(const Key('advanced-table-host.web')), findsOneWidget);
    expect(find.byKey(const Key('advanced-table-host.tools')), findsOneWidget);
    expect(find.text('mode = "local"'), findsOneWidget);
    expect(find.text('From the system file'), findsWidgets);
    // An entry a lower layer gives: no Edit, no Remove, and where it is defined.
    expect(find.byKey(const Key('advanced-host.tools-shared')), findsOneWidget);
    expect(find.byKey(const Key('advanced-host.tools-shared-remove')), findsNothing);
    expect(find.textContaining('Defined in the system file'), findsOneWidget);
  });

  testWidgets('a table of settings is changed with its form as one call, and taken out after asking', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('advanced-host.web-table-edit')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('advanced-host.web-field-listen')), '127.0.0.1:7800');
    await tester.tap(find.byKey(const Key('advanced-host.web-save')));
    await tester.pumpAndSettle();
    expect(host.sets().single, {'path': ['host', 'web'], 'value': {'listen': '127.0.0.1:7800'}, 'revision': 'r1'});
    expect(find.byKey(const Key('advanced-host.web-table-note')), findsOneWidget);

    await tester.tap(find.byKey(const Key('advanced-host.web-table-remove')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('confirm-remove')), findsOneWidget);
    await tester.tap(find.byKey(const Key('confirm-remove-no')));
    await tester.pumpAndSettle();
    expect(host.sets(), hasLength(1), reason: 'keeping it changes nothing');
    await tester.tap(find.byKey(const Key('advanced-host.web-table-remove')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('confirm-remove-yes')));
    await tester.pumpAndSettle();
    expect(host.sets().last['value'], isNull);
    expect(host.own.containsKey('host.web'), isFalse);
  });

  testWidgets('an entry is added by name, changed and removed; free-form members are JSON', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('advanced-host.tools-add')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('advanced-host.tools-name')), 'pi');
    await tester.enterText(find.byKey(const Key('advanced-host.tools-field-program')), 'pi');
    await tester.enterText(find.byKey(const Key('advanced-host.tools-field-prompt')), '{"type": "prompt"}');
    await tester.tap(find.byKey(const Key('advanced-host.tools-save')));
    await tester.pumpAndSettle();
    expect(host.sets().single['path'], ['host', 'tools', 'pi']);
    expect(host.sets().single['value'], {'program': 'pi', 'prompt': {'type': 'prompt'}});
    expect(find.byKey(const Key('advanced-host.tools-pi')), findsOneWidget);

    await tester.tap(find.byKey(const Key('advanced-host.tools-pi-edit')));
    await tester.pumpAndSettle();
    expect(find.text('{"type":"prompt"}'), findsOneWidget, reason: 'the free-form member is shown as JSON text');
    await tester.enterText(find.byKey(const Key('advanced-host.tools-field-program')), 'pi2');
    await tester.tap(find.byKey(const Key('advanced-host.tools-save')));
    await tester.pumpAndSettle();
    expect((host.sets().last['value']! as Map)['program'], 'pi2');

    await tester.tap(find.byKey(const Key('advanced-host.tools-pi-remove')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('confirm-remove-yes')));
    await tester.pumpAndSettle();
    expect(host.sets().last, containsPair('value', isNull));
    expect(find.byKey(const Key('advanced-host.tools-pi')), findsNothing);
  });

  testWidgets('text that is not JSON, a name taken and the host\'s refusal are said and write nothing wrong', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('advanced-host.tools-add')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('advanced-host.tools-name')), 'shared');
    await tester.tap(find.byKey(const Key('advanced-host.tools-save')));
    await tester.pumpAndSettle();
    expect(find.text('There is an entry called shared already.'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('advanced-host.tools-name')), 'x');
    await tester.enterText(find.byKey(const Key('advanced-host.tools-field-prompt')), '{not json');
    await tester.tap(find.byKey(const Key('advanced-host.tools-save')));
    await tester.pumpAndSettle();
    expect(find.text('prompt is not valid JSON.'), findsOneWidget);
    expect(host.sets(), isEmpty);
    await tester.enterText(find.byKey(const Key('advanced-host.tools-field-prompt')), '{}');
    host.refuse = 'host.tools.x: unasked is needed';
    await tester.tap(find.byKey(const Key('advanced-host.tools-save')));
    await tester.pumpAndSettle();
    expect(find.text('host.tools.x: unasked is needed'), findsOneWidget);
  });

  for (final (label, size) in [('desktop', const Size(1280, 1200)), ('phone', const Size(390, 1400))]) {
    testWidgets('golden at $label width', (tester) async {
      final host = ScriptedAdvanced()
        ..own['host.web'] = {'listen': '127.0.0.1:7800', 'office': true, 'hosts': ['canoa.example']}
        ..own['host.tools'] = {'pi': {'program': 'pi', 'runs': 'session'}};
      await open(tester, scripted: host, size: size);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/advanced_config_$label.png'));
    });
  }
}
