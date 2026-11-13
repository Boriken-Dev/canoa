// Settings over a canoa.cfg in YAML or JSON, and over layers: the page writes the person's own file in its own format, and
// says which values a lower layer gives.
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/config_layers.dart';
import 'package:canoa/settings/settings_row.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';

const _yaml = '''#!yaml
desktop:
  start_hidden: false
  close_to_tray: true
store:
  catalog_max_bytes: 4194304
host:
  agents:
    claude:
      program: npx
      listen: 127.0.0.1:7801
''';

const _json = '''#!json
{"desktop": {"start_hidden": false, "close_to_tray": true},
 "store": {"catalog_max_bytes": 4194304},
 "host": {"agents": {"claude": {"program": "npx", "listen": "127.0.0.1:7801"}}}}
''';

Future<void> _toggle(WidgetTester tester, String title) async {
  await tester.tap(find.descendant(of: find.widgetWithText(SwitchRow, title), matching: find.byType(Switch)));
  await tester.pumpAndSettle();
}

void main() {
  for (final (label, text, format) in [('YAML', _yaml, ConfigFormat.yaml), ('JSON', _json, ConfigFormat.json)]) {
    testWidgets('a General change over a $label file is written as $label, with its marker and its other tables', (tester) async {
      final h = await openSettings(tester, config: CanoaConfig.parse(text), section: 'general');
      await _toggle(tester, 'Close the window to the tray');
      final written = h.last.encode();
      expect(written, startsWith('#!${format.name}\n'));
      final back = decodeConfig(written);
      expect(back.format, format);
      expect((back.data['desktop'] as Map)['close_to_tray'], isFalse);
      expect(back.data['store'], {'catalog_max_bytes': 4194304});
      expect(((back.data['host'] as Map)['agents'] as Map)['claude'], {'program': 'npx', 'listen': '127.0.0.1:7801'});
      if (format == ConfigFormat.json) expect(jsonDecode(written.substring('#!json\n'.length)), isA<Map>());
    });
  }

  testWidgets('a value a lower layer gives is marked, one the person overrides says so, and Reset takes it back', (tester) async {
    final below = <ConfigLayer>[
      (origin: ConfigOrigin.installed, data: {'desktop': {'start_hidden': true, 'close_to_tray': true}}),
      (origin: ConfigOrigin.system, data: {'desktop': {'minimize_to_tray': true}, 'sessions': {'keep': 500}}),
    ];
    final config = CanoaConfig.parse('#!json\n{"desktop": {"close_to_tray": false}}\n', below: below);
    final h = await openSettings(tester, config: config, section: 'general');
    expect(find.descendant(of: find.widgetWithText(SwitchRow, 'Start hidden in the tray'), matching: find.text('From the installed file')), findsOneWidget);
    expect(find.descendant(of: find.widgetWithText(SwitchRow, 'Minimize the window to the tray'), matching: find.text('From the system file')), findsOneWidget);
    final overridden = find.widgetWithText(SwitchRow, 'Close the window to the tray');
    expect(find.descendant(of: overridden, matching: find.text('Overrides the installed file')), findsOneWidget);
    expect(find.byKey(const Key('setting-reset')), findsOneWidget);

    await tester.tap(find.byKey(const Key('setting-reset')));
    await tester.pumpAndSettle();
    expect(h.last.closeToTray, isTrue, reason: 'the installed file\'s again');
    expect(decodeConfig(h.last.encode()).data, isEmpty, reason: 'nothing of a lower layer is copied into the person\'s file');
    expect(find.byKey(const Key('setting-reset')), findsNothing);
    expect(find.descendant(of: overridden, matching: find.text('From the installed file')), findsOneWidget);
  });
}
