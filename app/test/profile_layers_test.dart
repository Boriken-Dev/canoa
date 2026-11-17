// Project trust, and every layer merged from where it really lives: the overlays a host installed, read from its folder.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/profile/project_trust.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/disk_overlays.dart';

void main() {
  late Directory temp;
  setUp(() => temp = Directory.systemTemp.createTempSync('canoa-profile-'));
  tearDown(() => temp.deleteSync(recursive: true));

  group('every layer', () {
    test('merges from where each lives; a project profile counts only while trusted as it is', () async {
      const teamConfig = '''
[env]
GITHUB_TOKEN = { provider = "env", key = "GITHUB_TOKEN" }

[harness.claude-code]
model = "team-model"
effort = "medium"

[[mcp.servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example/mcp"
''';

      final configDir = Directory('${temp.path}/config')..createSync();
      final config = CanoaConfig.parse('''
[overlays]
team = "https://example.invalid/team.git@main#profiles/team"
[profile]
effort = "high"
[profile.env.RUST_LOG]
value = "info"
[agents.Claude.profile]
model = "claude-x"
unset = ["context"]
[agents.Claude.profile.skills.review]
remove = true
''');
      final disk = DiskOverlays('${configDir.path}/overlays');
      final home = IoProfileHome(configDir.path, overlays: disk);
      final team = config.overlays['team']! as GitOverlay;
      disk.install('team', team, {'overlay.cfg': teamConfig, 'CONTEXT.md': 'Be brief.\n', 'skills/review/SKILL.md': '# Review\n'});

      final project = Directory('${temp.path}/project')..createSync();
      final projectFile = File('${project.path}/$projectProfileFile')..parent.createSync();
      projectFile.writeAsStringSync('mode = "plan"\n[env.RUST_LOG]\nvalue = "debug"\n[mcp.docs]\nremove = true\n');

      Future<ProfileLayers> layers() => profileLayers(
        overlays: config.overlays,
        user: config.profile,
        agents: config.agentProfiles,
        agent: 'Claude',
        home: home,
        workingDirectory: project.path,
        session: Profile.parse('effort = "low"\n', LayerKind.session),
      );

      var result = await layers();
      expect(result.project!.state, TrustState.untrusted);
      expect([for (final layer in result.layers) '${layer.id}'], ['built-in', 'overlay:team', 'user', 'agent:Claude', 'session']);
      var view = result.effective.view();
      expect(view['mode'], isNull, reason: 'an untrusted project sets nothing');
      expect((view['mcp'] as List).single, containsPair('layer', 'overlay:team'));

      await home.trust.trust(result.project!.directory, result.project!.profile!);
      result = await layers();
      expect(result.project!.state, TrustState.trusted);
      view = result.effective.view();
      expect(view['model'], {'value': 'claude-x', 'layer': 'agent:Claude'});
      expect(view['effort'], {'value': 'low', 'layer': 'session'});
      expect(view['mode'], {'value': 'plan', 'layer': 'project'});
      expect(view.containsKey('context'), isFalse);
      expect(view['env'], [
        {'name': 'TEAM_OVERLAY_ROOT', 'value': {'value': home.overlays.directoryOf('team')}, 'layer': 'overlay:team'},
        {'name': 'GITHUB_TOKEN', 'value': {'reference': 'env:GITHUB_TOKEN', 'optional': false}, 'layer': 'overlay:team'},
        {'name': 'RUST_LOG', 'value': {'value': 'debug'}, 'layer': 'project'},
      ]);
      expect(view['skills'], isEmpty);
      expect(view['mcp'], isEmpty);
      expect(view['removed'], [
        {'key': 'mcp.docs', 'layer': 'project', 'was': 'overlay:team'},
        {'key': 'context', 'layer': 'agent:Claude', 'was': 'overlay:team'},
        {'key': 'skills.review', 'layer': 'agent:Claude', 'was': 'overlay:team'},
      ]);

      projectFile.writeAsStringSync('mode = "plan"\n[mcp.evil]\ntype = "stdio"\ncommand = "evil"\n');
      result = await layers();
      expect(result.project!.state, TrustState.changed);
      expect(result.effective.view()['mcp'], [contains('name')], reason: 'the changed profile adds nothing');
      expect((result.effective.view()['mcp'] as List).single['name'], 'docs');

      await home.trust.trust(result.project!.directory, result.project!.profile!);
      expect((await layers()).effective.view()['mode'], {'value': 'plan', 'layer': 'project'});
      await home.trust.revoke(result.project!.directory);
      result = await layers();
      expect(result.project!.state, TrustState.untrusted);
      expect(result.effective.view()['mode'], isNull);

      final reread = IoProfileHome(configDir.path);
      await reread.trust.load();
      expect(reread.trust.projects, isEmpty, reason: 'revoking is kept');
    });

    test('an installed overlay from another source than canoa.cfg names is left out until synced', () async {
      final disk = DiskOverlays('${temp.path}/config/overlays');
      final home = IoProfileHome('${temp.path}/config', overlays: disk);
      const repo = 'https://example.invalid/o.git';
      disk.install('o', GitOverlay(repo: repo, reference: 'main'), {'overlay.cfg': ''});
      final moved = {'o': GitOverlay(repo: repo, reference: 'other')};
      final result = await profileLayers(overlays: moved, user: const Profile(), agents: const {}, agent: 'a', home: home);
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'user']);
      expect(result.notes.single.text, contains('not the source canoa.cfg names'));
    });
    test('an installed copy is digested each time it is used: a change leaves it out, a pin ends unpinned', () async {
      final disk = DiskOverlays('${temp.path}/config/overlays');
      final home = IoProfileHome('${temp.path}/config', overlays: disk);
      var config = CanoaConfig.parse('[overlays]\no = "https://example.invalid/o.git@main"\n');
      final installed = disk.install('o', config.overlays['o']! as GitOverlay, {'overlay.cfg': '[env]\nMODE = "on"\n'});
      Future<ProfileLayers> layers() =>
          profileLayers(overlays: config.overlays, user: const Profile(), agents: const {}, agent: 'a', home: home);

      var result = await layers();
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'overlay:o', 'user']);
      List<Object?> nonDefaults() => ((result.effective.view()['overlays'] as List).single as Map)['non_defaults'] as List;
      expect(nonDefaults(), [['unpinned', '']]);

      config = CanoaConfig.parse(config.withOverlayPin('o', installed.digest!).encode());
      expect((config.overlays['o']! as GitOverlay).pin, installed.digest, reason: 'the pin is kept in canoa.cfg');
      result = await layers();
      expect(nonDefaults(), isEmpty);

      final dir = home.overlays.directoryOf('o');
      Directory('$dir/.git').createSync();
      File('$dir/.git/config').writeAsStringSync('[core]\n');
      result = await layers();
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'overlay:o', 'user'],
          reason: 'an excluded name is no part of the overlay: the digest leaves it out, and nothing reads it');
      Directory('$dir/.git').deleteSync(recursive: true);

      File('$dir/overlay.cfg').writeAsStringSync('[env]\nMODE = "changed"\n');
      result = await layers();
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'user']);
      expect(result.notes.last.text, contains('Its files changed'));
    });  });

  test('a secret file of a project layer is read only inside the project, links followed', () async {
    final project = Directory('${temp.path}/project')..createSync();
    File('${project.path}/token').writeAsStringSync('inside\n');
    final outside = File('${temp.path}/outside')..writeAsStringSync('outside');
    final home = IoProfileHome('${temp.path}/config');
    final effective = mergeProfiles([
      Layer(const LayerId(LayerKind.project), Profile.parse('[env.A]\nfrom_file = "token"\n', LayerKind.project),
          dir: project.path),
    ]);
    final resolved = await effective.environment(lookup: (_) => null, readFile: home.readSecret);
    expect(resolved['A']!.expose(), 'inside');
    expect('$resolved', isNot(contains('inside')));
    try {
      Link('${project.path}/escape').createSync(outside.path);
    } on FileSystemException {
      markTestSkipped('this account cannot make links');
      return;
    }
    final escaping = mergeProfiles([
      Layer(const LayerId(LayerKind.project), Profile.parse('[env.A]\nfrom_file = "escape"\n', LayerKind.project),
          dir: project.path),
    ]);
    await expectLater(escaping.environment(lookup: (_) => null, readFile: home.readSecret),
        throwsA(isA<ProfileException>().having((e) => e.code, 'code', 'resolve')));
  });

  test('canoa.cfg keeps [overlays], [profile] and each agent profile when the app saves it', () {
    const text = '''
[overlays]
team = "https://example.invalid/p.git@main#team"
mine = { dir = "profiles/mine" }
[profile]
model = "m"
[profile.env.TOKEN]
from_env = "MY_TOKEN"
[agents.Claude.profile]
effort = "high"
[agents.mine]
program = "my-agent"
[agents.mine.profile.skills.s]
path = "/skills/s"
''';
    final config = CanoaConfig.parse(text);
    expect(config.overlays.keys, ['team', 'mine']);
    expect(config.agents.single.label, 'mine', reason: 'a profile alone adds no agent');
    expect(config.agentProfiles.keys, ['Claude', 'mine']);
    final again = CanoaConfig.parse(config.copyWith(startHidden: true).encode());
    expect(again.overlays.keys, ['team', 'mine']);
    expect((again.overlays['team']! as GitOverlay).spec, 'https://example.invalid/p.git@main#team');
    expect(again.profile.env['TOKEN'], isA<EnvFromEnv>());
    expect(again.agentProfiles['Claude']!.effort, 'high');
    expect(again.agentProfiles['mine']!.skills['s']!.path, '/skills/s');
    expect(again.agents.single.program, 'my-agent');
    expect(again.tables, config.tables);
  });

  test('canoa.cfg refuses an overlay source git must not run, and a profile the schema refuses', () {
    expect(() => CanoaConfig.parse('[overlays]\nx = "ext::sh -c id@main"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[overlays]\nx = { dir = "" }\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[profile]\ncolour = "red"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[agents.a.profile.env.A]\nvalue = "1"\nfrom_env = "B"\n'), throwsFormatException);
  });

  test('a secret file is a regular file of at most maxSecretFile bytes, less one final line break only', () async {
    final dir = Directory('${temp.path}/user')..createSync();
    File('${dir.path}/big').writeAsBytesSync(List.filled(maxSecretFile + 1, 0x78));
    File('${dir.path}/fits').writeAsBytesSync(List.filled(maxSecretFile, 0x78));
    Directory('${dir.path}/a-directory').createSync();
    File('${dir.path}/carriage').writeAsStringSync('value\r');
    final home = IoProfileHome('${temp.path}/config');
    Future<String?> resolve(String name) async {
      final effective = mergeProfiles([
        Layer(const LayerId(LayerKind.user), Profile.parse('[env.A]\nfrom_file = "$name"\n', LayerKind.user),
            dir: dir.path),
      ]);
      try {
        return (await effective.environment(lookup: (_) => null, readFile: home.readSecret))['A']!.expose();
      } on ProfileException {
        return null;
      }
    }
    expect(await resolve('big') == null, isTrue, reason: 'over the limit');
    expect(await resolve('a-directory'), isNull);
    expect((await resolve('fits'))!.length, maxSecretFile);
    expect(await resolve('carriage'), 'value\r', reason: 'as canoa reads it');
  });
}
