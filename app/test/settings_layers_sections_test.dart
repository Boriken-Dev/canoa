// Every Settings section says which layer of canoa.cfg a value comes from, through the shared row, and Reset takes the person's
// value out of their file so that the lower layer's shows again.
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/config_layers.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/hosting_panel.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_host.dart';
import 'fixtures/settings_fixture.dart';

/// The person's own file in JSON, over a system file that holds [system].
CanoaConfig _layered({required Map<String, dynamic> own, required Map<String, dynamic> system}) => CanoaConfig.parse(
  '#!json\n${jsonEncode(own)}\n',
  below: [(origin: ConfigOrigin.system, data: system)],
);

Finder get _from => find.text('From the system file');
Finder get _overrides => find.text('Overrides the system file');
Finder get _reset => find.byKey(const Key('setting-reset'));

Future<void> _resetOne(WidgetTester tester) async {
  expect(_reset, findsOneWidget);
  await tester.ensureVisible(_reset);
  await tester.tap(_reset);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('General', (tester) async {
    final config = _layered(
      own: {'sessions': {'keep': 50}},
      system: {'sessions': {'keep': 500}, 'desktop': {'start_hidden': true}},
    );
    final h = await openSettings(tester, config: config, section: 'general');
    expect(_from, findsWidgets);
    expect(_overrides, findsOneWidget);
    await _resetOne(tester);
    expect(h.last.sessions.keep, 500);
    expect(_reset, findsNothing);
  });

  testWidgets('Appearance', (tester) async {
    final config = _layered(
      own: {'appearance': {'brightness': 'dark'}},
      system: {'appearance': {'brightness': 'light', 'look': 'terminal'}},
    );
    final h = await openSettings(tester, config: config, section: 'appearance');
    expect(_from, findsOneWidget, reason: 'the look');
    expect(_overrides, findsOneWidget, reason: 'the brightness');
    await _resetOne(tester);
    expect(h.last.brightness, 'light');
  });

  testWidgets('Notifications', (tester) async {
    final config = _layered(
      own: {'notifications': {'finished': true}},
      system: {'notifications': {'finished': false, 'question': false}},
    );
    final h = await openSettings(tester, config: config, section: 'notifications');
    expect(_from, findsOneWidget, reason: 'the question');
    expect(_overrides, findsOneWidget, reason: 'finished');
    await _resetOne(tester);
    expect(h.last.notifications.finished, isFalse);
  });

  testWidgets('Sources', (tester) async {
    final codex = AgentSource.hosted('Codex', harness: 'codex', local: true);
    final config = _layered(
      own: {'agents': {'mine': {'program': 'mine', 'args': ['--own']}}},
      system: {'sources': {'hidden': ['Codex']}, 'agents': {'mine': {'program': 'mine', 'args': ['--acp']}, 'theirs': {'program': 'theirs'}}},
    );
    final h = await openSettings(tester, config: config, section: 'sources', presets: [codex]);
    expect(_from, findsNWidgets(2), reason: 'the hidden preset and the system\'s own source');
    expect(_overrides, findsOneWidget, reason: 'a source of the person\'s that replaces the system\'s');
    await _resetOne(tester);
    expect(h.last.agents.firstWhere((agent) => agent.label == 'mine').args, ['--acp']);
  });

  testWidgets('Profiles', (tester) async {
    final config = _layered(
      own: {'overlays': {'team': {'dir': 'own-profiles'}}},
      system: {'overlays': {'team': {'dir': 'system-profiles'}, 'extra': {'dir': 'extra-profiles'}}},
    );
    final h = await openSettings(tester, config: config, section: 'profiles', agents: const ['Claude']);
    expect(_from, findsOneWidget, reason: 'the overlay only the system names');
    expect(_overrides, findsOneWidget);
    await _resetOne(tester);
    expect(jsonEncode(h.last.tables['overlays']), contains('system-profiles'));
    expect(jsonEncode(h.last.tables['overlays']), isNot(contains('own-profiles')));
  });

  testWidgets('MCP servers', (tester) async {
    final config = _layered(
      own: {'profile': {'mcp': {'docs': {'type': 'http', 'url': 'https://own.example/mcp'}}}},
      system: {'profile': {'mcp': {
        'docs': {'type': 'http', 'url': 'https://system.example/mcp'},
        'wiki': {'type': 'http', 'url': 'https://wiki.example/mcp'},
      }}},
    );
    final h = await openSettings(tester, config: config, section: 'mcp', agents: const ['Claude']);
    expect(_from, findsOneWidget, reason: 'the server only the system gives');
    expect(_overrides, findsOneWidget);
    await _resetOne(tester);
    expect(jsonEncode(h.last.tables['profile']), contains('system.example'));
    expect(jsonEncode(h.last.tables['profile']), isNot(contains('own.example')));
  });

  testWidgets('Permissions', (tester) async {
    final config = _layered(
      own: {'permissions': {'default_mode': 'auto', 'protected': ['mine/']}},
      system: {'permissions': {'default_mode': 'plan', 'allow_once_only': true, 'protected': ['theirs/']}},
    );
    final h = await openSettings(tester, config: config, section: 'permissions');
    expect(_from, findsOneWidget, reason: 'allow once only');
    expect(_overrides, findsNWidgets(2), reason: 'the mode and the protected paths');
    await tester.ensureVisible(_reset.first);
    await tester.tap(_reset.first);
    await tester.pumpAndSettle();
    expect(h.last.defaultMode, 'plan');
    await tester.ensureVisible(_reset);
    await tester.tap(_reset);
    await tester.pumpAndSettle();
    expect(h.last.protectedPaths, ['theirs/']);
    expect(tester.widget<TextField>(find.byKey(const Key('settings-protected-paths'))).controller!.text, 'theirs/');
  });

  testWidgets('Usage', (tester) async {
    final config = _layered(own: {'usage': {'warn_permille': 700}}, system: {'usage': {'warn_permille': 900}});
    final h = await openSettings(tester, config: config, section: 'usage');
    expect(_overrides, findsOneWidget);
    await _resetOne(tester);
    expect(h.last.warnPermille, 900);
  });

  testWidgets('Catalogs', (tester) async {
    const mine = 'https://example.org/mine.git@main#catalog.toml';
    const theirs = 'https://example.org/theirs.git@main#catalog.toml';
    final config = _layered(
      own: {'catalogs': {'team': mine}, 'store': {'catalog_max_bytes': 2000}},
      system: {'catalogs': {'team': theirs, 'more': theirs}, 'store': {'catalog_max_bytes': 1000}},
    );
    final h = await openSettings(tester, config: config, section: 'catalogs');
    expect(_from, findsOneWidget, reason: 'the catalog only the system names');
    expect(_overrides, findsNWidgets(2), reason: 'a catalog and a limit');
    await tester.ensureVisible(_reset.first);
    await tester.tap(_reset.first);
    await tester.pumpAndSettle();
    expect((h.last.tables['catalogs'] as Map)['team'], theirs);
    await _resetOne(tester);
    expect((h.last.tables['store'] as Map)['catalog_max_bytes'], 1000);
  });

  testWidgets('Hosting', (tester) async {
    final tool = FakeTool();
    tool.answer = (args) => const HostOutput(0, '', '');
    final saved = <CanoaConfig>[];
    final config = _layered(
      own: {'host': {'agents': {'alpha': {'program': 'own-program', 'listen': '127.0.0.1:7801'}}}},
      system: {'host': {'agents': {
        'alpha': {'program': 'system-program', 'listen': '127.0.0.1:7801'},
        'beta': {'program': 'beta-program', 'listen': '127.0.0.1:7802'},
      }}},
    );
    await tester.binding.setSurfaceSize(const Size(900, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: HostingPanel(
          supervisor: HostSupervisor(tool),
          config: config,
          onSave: (changed) async {
            saved.add(changed);
            return null;
          },
          freePort: () async => 7811,
        ),
      ),
    ));
    await tester.pumpAndSettle();
    expect(_from, findsOneWidget, reason: 'the entry only the system names');
    expect(_overrides, findsOneWidget);
    await _resetOne(tester);
    expect(saved.single.hostEntries.firstWhere((entry) => entry.name == 'alpha').runs, 'system-program');
    expect(decodeConfig(saved.single.encode()).data['host'], isNull, reason: 'nothing of the system file is copied into the person\'s');
  });
}
