// The settings view of profiles: every value beside its layer, an overlay installed only on a click that shows what it
// changes, and a project profile used only once trusted, revocably.
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home.dart';
import 'package:canoa/profile/profiles_dialog.dart';
import 'package:canoa/profile/project_trust.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class FakeOverlays implements OverlayManager {
  final records = <String, InstalledOverlay>{};
  final checked = <String>[];

  @override
  InstalledOverlay? installed(String name) => records[name];

  @override
  String directoryOf(String name) => '/overlays/$name';

  @override
  Future<OverlayUpdate> check(String name, GitOverlay source) async {
    checked.add(name);
    return OverlayUpdate(name: name, source: source, from: records[name]?.commit, commit: 'c' * 40,
        files: const {'profile.toml': 'b1'}, skipped: const ['link'], added: const ['profile.toml'], changed: const [],
        removed: const []);
  }

  @override
  Future<InstalledOverlay> apply(OverlayUpdate update) async => records[update.name] =
      InstalledOverlay(name: update.name, source: update.source.spec, commit: update.commit, files: update.files);

  @override
  Future<void> remove(String name) async => records.remove(name);
}

final class FakeHome implements ProfileHome {
  FakeHome(this.files);
  final Map<String, String> files;
  String? trustRecord;
  @override
  final FakeOverlays overlays = FakeOverlays();
  @override
  late final trust = ProjectTrust(read: () async => trustRecord, write: (text) async => trustRecord = text);
  @override
  String get configDir => '/config';
  @override
  String join(String dir, String relative) => '$dir/$relative';
  @override
  String? canonicalDirectory(String directory) => directory;
  @override
  Future<List<int>?> readBytes(String path) async => files[path] == null ? null : utf8.encode(files[path]!);
  @override
  Future<String?> readSecret(String path, {String? within}) async => files[path];
}

String layerOf(WidgetTester tester, String key) => tester.widget<Text>(find.byKey(Key('profile-layer-$key'))).data!;

void main() {
  final config = CanoaConfig.parse('''
[overlays]
team = "https://example.invalid/p.git@main#team"
[profile]
effort = "high"
[profile.env.TOKEN]
from_env = "MY_TOKEN"
[agents.Claude.profile]
model = "claude-x"
''');

  Future<FakeHome> open(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1400, 2400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final home = FakeHome({
      '/overlays/team/profile.toml': 'model = "team"\nmode = "default"\n[skills.review]\npath = "skills/review"\n',
      '/work/.canoa/profile.toml': 'mode = "plan"\n',
    });
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ProfilesDialog(
      config: config,
      agents: const ['Claude', 'Codex'],
      home: home,
      workingDirectory: '/work',
    ))));
    await tester.pumpAndSettle();
    return home;
  }

  testWidgets('every value names its layer, and a reference shows as a reference', (tester) async {
    final home = await open(tester);
    expect(layerOf(tester, 'model'), 'agent:Claude');
    expect(layerOf(tester, 'effort'), 'user');
    expect(layerOf(tester, 'env.TOKEN'), 'user');
    expect(find.text('from the environment variable MY_TOKEN'), findsOneWidget);
    expect(find.byKey(const Key('profile-row-mode')), findsNothing, reason: 'the overlay is not installed');
    expect(find.textContaining('Not installed'), findsWidgets);

    await tester.tap(find.byKey(const Key('profile-agent')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Codex').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('profile-row-model')), findsNothing, reason: 'Codex has no agent layer');
    expect(home.overlays.checked, isEmpty, reason: 'nothing is fetched until asked');
  });

  testWidgets('an overlay installs only after the person sees what it changes', (tester) async {
    final home = await open(tester);
    await tester.tap(find.byKey(const Key('profile-sync-team')));
    await tester.pumpAndSettle();
    expect(find.text('Install team?'), findsOneWidget);
    expect(find.textContaining('profile.toml'), findsWidgets);
    expect(find.textContaining('links and submodules are never installed'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(home.overlays.records, isEmpty);

    await tester.tap(find.byKey(const Key('profile-sync-team')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('overlay-apply')));
    await tester.pumpAndSettle();
    expect(home.overlays.records['team']!.commit, 'c' * 40);
    expect(layerOf(tester, 'mode'), 'overlay:team');
    expect(layerOf(tester, 'skills.review'), 'overlay:team');
    expect(layerOf(tester, 'model'), 'agent:Claude', reason: 'the agent entry is above the overlay');
  });

  testWidgets('a project profile is ignored until trusted, shown before it is, and revocable', (tester) async {
    final home = await open(tester);
    expect(find.textContaining('you have not trusted this directory'), findsOneWidget);
    expect(find.byKey(const Key('profile-row-mode')), findsNothing);

    await tester.tap(find.byKey(const Key('profile-trust')));
    await tester.pumpAndSettle();
    expect(find.text('mode = "plan"\n'), findsOneWidget, reason: 'what is trusted is shown');
    await tester.tap(find.text('Trust').last);
    await tester.pumpAndSettle();
    expect(layerOf(tester, 'mode'), 'project');
    expect(home.trustRecord, contains('/work'));

    await tester.tap(find.byKey(const Key('profile-revoke-/work')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('profile-row-mode')), findsNothing);
    expect(find.textContaining('you have not trusted this directory'), findsOneWidget);
  });
}
