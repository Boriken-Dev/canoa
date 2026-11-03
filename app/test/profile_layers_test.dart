// Overlays installed and synced from a local bare git repository, project trust, and every layer merged from where it
// really lives. Needs git on PATH; skipped without it.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlay_store_io.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/profile/project_trust.dart';
import 'package:flutter_test/flutter_test.dart';

final hasGit = () {
  try {
    return Process.runSync('git', ['--version']).exitCode == 0;
  } on ProcessException {
    return false;
  }
}();

String git(String dir, List<String> args, {String? input}) {
  final result = Process.runSync('git', [
    '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid', '-c', 'commit.gpgsign=false',
    '-c', 'core.autocrlf=false', '-c', 'init.defaultBranch=main', '-c', 'core.protectNTFS=false', ...args,
  ], workingDirectory: dir);
  expect(result.exitCode, 0, reason: 'git ${args.join(' ')}: ${result.stderr}');
  return (result.stdout as String).trim();
}

/// A working repository and a bare one it pushes to, as an overlay's source lives.
final class Upstream {
  Upstream(Directory root) : work = Directory('${root.path}/work'), bare = '${root.path}/source.git' {
    work.createSync(recursive: true);
    git(work.path, ['init', '--quiet']);
    git(root.path, ['init', '--quiet', '--bare', bare]);
  }
  final Directory work;
  final String bare;

  void write(String path, String text) {
    final file = File('${work.path}/$path');
    file.parent.createSync(recursive: true);
    file.writeAsStringSync(text);
  }

  void delete(String path) => File('${work.path}/$path').deleteSync();

  final _entries = <String>[];

  /// Adds an entry of [mode] holding [text] that the working tree does not have: a link, or a name this file system
  /// could not hold.
  void entry(String mode, String path, String text) {
    final file = File('${work.path}/.entry')..writeAsStringSync(text);
    _entries.add('$mode,${git(work.path, ['hash-object', '-w', file.path])},$path');
    file.deleteSync();
  }

  /// Commits everything and pushes it; the new commit's id.
  String commit(String message) {
    git(work.path, ['add', '-A']);
    for (final entry in _entries) {
      git(work.path, ['update-index', '--add', '--cacheinfo', entry]);
    }
    _entries.clear();
    git(work.path, ['commit', '--quiet', '-m', message]);
    git(work.path, ['push', '--quiet', bare, 'HEAD:refs/heads/main']);
    return git(work.path, ['rev-parse', 'HEAD']);
  }
}

void main() {
  late Directory temp;
  setUp(() => temp = Directory.systemTemp.createTempSync('canoa-profile-'));
  tearDown(() => temp.deleteSync(recursive: true));

  group('an overlay from git', skip: hasGit ? false : 'git is not on PATH', () {
    test('installs at a commit, shows what a sync changes, and changes nothing until applied', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('team/profile.toml', 'model = "one"\n[skills.review]\npath = "skills/review"\n');
      upstream.write('team/skills/review/SKILL.md', '# Review\n');
      upstream.write('team/old.md', 'old\n');
      upstream.write('elsewhere/secret.txt', 'not part of the overlay\n');
      final first = upstream.commit('first');

      final store = OverlayStore('${temp.path}/home/overlays');
      final source = GitOverlay(repo: upstream.bare, reference: 'main', path: 'team');
      expect(store.installed('team'), isNull);
      final install = await store.check('team', source);
      expect(install.from, isNull);
      expect(install.commit, first);
      expect(install.added, unorderedEquals(['profile.toml', 'skills/review/SKILL.md', 'old.md']));
      expect(store.installed('team'), isNull, reason: 'checking installs nothing');
      final installed = await store.apply(install);
      expect(installed.commit, first);
      final dir = store.directoryOf('team');
      expect(File('$dir/profile.toml').readAsStringSync(), contains('"one"'));
      expect(File('$dir/secret.txt').existsSync(), isFalse);
      expect(File('$dir/../secret.txt').existsSync(), isFalse);

      upstream.write('team/profile.toml', 'model = "two"\n');
      upstream.write('team/new.md', 'new\n');
      upstream.delete('team/old.md');
      upstream.entry('120000', 'team/link', '../../elsewhere/secret.txt');
      final second = upstream.commit('second');

      final sync = await store.check('team', source);
      expect((sync.from, sync.commit), (first, second));
      expect(sync.added, ['new.md']);
      expect(sync.changed, ['profile.toml']);
      expect(sync.removed, ['old.md']);
      expect(sync.skipped, ['link'], reason: 'a link is never installed');
      expect(store.installed('team')!.commit, first, reason: 'pinned until the person applies');
      expect(File('$dir/profile.toml').readAsStringSync(), contains('"one"'));

      await store.apply(sync);
      expect(store.installed('team')!.commit, second);
      expect(File('$dir/profile.toml').readAsStringSync(), contains('"two"'));
      expect(File('$dir/old.md').existsSync(), isFalse);
      expect(FileSystemEntity.typeSync('$dir/link', followLinks: false), FileSystemEntityType.notFound);

      final pinned = await store.check('team', GitOverlay(repo: upstream.bare, reference: first, path: 'team'));
      expect(pinned.commit, first, reason: 'a commit id names that commit, wherever main is');
      expect(await store.check('team', source), isA<OverlayUpdate>().having((u) => u.changesNothing, 'nothing', true));
    });

    test('a file name this computer cannot hold safely refuses the whole overlay', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('profile.toml', 'model = "m"\n');

      upstream.entry('100644', 'skills/CON.md', 'device');
      upstream.commit('device name');
      final store = OverlayStore('${temp.path}/overlays');
      await expectLater(store.check('o', GitOverlay(repo: upstream.bare, reference: 'main')),
          throwsA(isA<OverlayException>()));
    });

    test('a ref the source does not have, and a source git must not run, are refused', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('profile.toml', 'model = "m"\n');
      upstream.commit('one');
      final store = OverlayStore('${temp.path}/overlays');
      await expectLater(store.check('o', GitOverlay(repo: upstream.bare, reference: 'nope')),
          throwsA(isA<OverlayException>()));
      final marker = File('${temp.path}/ran');
      await expectLater(
        store.check('o', GitOverlay(repo: 'ext::sh -c touch% ${marker.path}', reference: 'main')),
        throwsA(isA<ProfileException>()),
      );
      expect(marker.existsSync(), isFalse);
    });
  });

  group('every layer', skip: hasGit ? false : 'git is not on PATH', () {
    test('merges from where each lives; a project profile counts only while trusted as it is', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('profiles/team/profile.toml', '''
model = "team-model"
effort = "medium"
context = { file = "CONTEXT.md" }
[env.GITHUB_TOKEN]
from_env = "GITHUB_TOKEN"
[skills.review]
path = "skills/review"
[mcp.docs]
type = "http"
url = "https://docs.example/mcp"
''');
      upstream.write('profiles/team/CONTEXT.md', 'Be brief.\n');
      upstream.commit('team');

      final configDir = Directory('${temp.path}/config')..createSync();
      final config = CanoaConfig.parse('''
[overlays]
team = "${upstream.bare.replaceAll(r'\', '/')}@main#profiles/team"
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
      final home = IoProfileHome(configDir.path);
      final team = config.overlays['team']! as GitOverlay;
      await home.overlays.apply(await home.overlays.check('team', team));

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
        {'name': 'GITHUB_TOKEN', 'value': {'from_env': 'GITHUB_TOKEN'}, 'layer': 'overlay:team'},
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

    test('an installed overlay from another source than Canoa.cfg names is left out until synced', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('profile.toml', 'model = "m"\n');
      upstream.commit('one');
      final home = IoProfileHome('${temp.path}/config');
      final source = GitOverlay(repo: upstream.bare, reference: 'main');
      await home.overlays.apply(await home.overlays.check('o', source));
      final moved = {'o': GitOverlay(repo: upstream.bare, reference: 'other')};
      final result = await profileLayers(overlays: moved, user: const Profile(), agents: const {}, agent: 'a', home: home);
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'user']);
      expect(result.notes.single.text, contains('not the source Canoa.cfg names'));
    });
  });

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

  test('Canoa.cfg keeps [overlays], [profile] and each agent profile when the app saves it', () {
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

  test('Canoa.cfg refuses an overlay source git must not run, and a profile the schema refuses', () {
    expect(() => CanoaConfig.parse('[overlays]\nx = "ext::sh -c id@main"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[overlays]\nx = { dir = "" }\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[profile]\ncolour = "red"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[agents.a.profile.env.A]\nvalue = "1"\nfrom_env = "B"\n'), throwsFormatException);
  });
}
