// Overlays installed and synced from a local bare git repository, project trust, and every layer merged from where it
// really lives. Needs git on PATH; skipped without it.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/aos.dart';
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

/// A bare repository whose trees are written object by object, so they can hold what `git add` refuses: a `.git`
/// directory, names that differ only in case, or a tree repeated a million times.
final class Crafted {
  Crafted(this.dir) {
    git(Directory.systemTemp.path, ['init', '--quiet', '--bare', dir]);
  }
  final String dir;

  Future<String> _object(List<String> args, String input) async {
    final process = await Process.start('git', ['--git-dir=$dir', ...args]);
    process.stdin.write(input);
    await process.stdin.close();
    final out = await process.stdout.transform(utf8.decoder).join();
    expect(await process.exitCode, 0, reason: 'git ${args.join(' ')}');
    return out.trim();
  }

  Future<String> blob(String text) => _object(['hash-object', '-w', '--stdin'], text);

  /// [entries]: a name to a blob's text, a nested map, or a tree id given as `tree:<id>`.
  Future<String> tree(Map<String, Object> entries) async {
    final lines = StringBuffer();
    for (final MapEntry(:key, :value) in entries.entries) {
      final line = switch (value) {
        final String text when text.startsWith('tree:') => '040000 tree ${text.substring(5)}\t$key',
        final String text => '100644 blob ${await blob(text)}\t$key',
        final Map<String, Object> inner => '040000 tree ${await tree(inner)}\t$key',
        _ => throw ArgumentError(value),
      };
      lines.write('$line\u0000');
    }
    return _object(['mktree', '-z', '--missing'], '$lines');
  }

  /// Commits [tree] as [branch]'s head; the commit's id.
  Future<String> commit(String tree, {String branch = 'main', String? parent}) async {
    final id = await _object([
      '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid',
      'commit-tree', tree, if (parent != null) ...['-p', parent], '-m', 'crafted',
    ], '');
    git(Directory.systemTemp.path, ['--git-dir=$dir', 'update-ref', 'refs/heads/$branch', id]);
    return id;
  }
}

void main() {
  late Directory temp;
  setUp(() => temp = Directory.systemTemp.createTempSync('canoa-profile-'));
  tearDown(() => temp.deleteSync(recursive: true));

  group('an overlay from git', skip: hasGit ? false : 'git is not on PATH', () {
    test('installs at a commit, shows what a sync changes, and changes nothing until applied', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('team/overlay.cfg', '[harness.claude-code]\nmodel = "one"\n');
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
      expect(install.added, unorderedEquals(['overlay.cfg', 'skills/review/SKILL.md', 'old.md']));
      expect(store.installed('team'), isNull, reason: 'checking installs nothing');
      final installed = await store.apply(install);
      expect(installed.commit, first);
      expect(installed.digest, install.digest, reason: 'the record keeps the tree digest installed');
      final dir = store.directoryOf('team');
      expect(digestEntries(await IoProfileHome('${temp.path}/home').readTree(dir)).digest, install.digest);
      expect(File('$dir/overlay.cfg').readAsStringSync(), contains('"one"'));
      expect(File('$dir/secret.txt').existsSync(), isFalse);
      expect(File('$dir/../secret.txt').existsSync(), isFalse);

      upstream.write('team/overlay.cfg', '[harness.claude-code]\nmodel = "two"\n');
      upstream.write('team/new.md', 'new\n');
      upstream.delete('team/old.md');
      final second = upstream.commit('second');

      final sync = await store.check('team', source);
      expect((sync.from, sync.commit), (first, second));
      expect(sync.added, ['new.md']);
      expect(sync.changed, ['overlay.cfg']);
      expect(sync.removed, ['old.md']);
      expect(sync.digest, isNot(install.digest));
      expect(store.installed('team')!.commit, first, reason: 'pinned until the person applies');
      expect(File('$dir/overlay.cfg').readAsStringSync(), contains('"one"'));

      await store.apply(sync);
      expect(store.installed('team')!.commit, second);
      expect(File('$dir/overlay.cfg').readAsStringSync(), contains('"two"'));
      expect(File('$dir/old.md').existsSync(), isFalse);

      final pinned = await store.check('team', GitOverlay(repo: upstream.bare, reference: first, path: 'team'));
      expect(pinned.commit, first, reason: 'a commit id names that commit, wherever main is');
      expect(await store.check('team', source), isA<OverlayUpdate>().having((u) => u.changesNothing, 'nothing', true));
    });

    test('a link, and a file name this computer cannot hold safely, refuse the whole overlay', () async {
      final linked = Upstream(Directory('${temp.path}/linked'));
      linked.write('overlay.cfg', '');
      linked.entry('120000', 'kb/link', '../../elsewhere/secret.txt');
      linked.commit('a link');
      await expectLater(OverlayStore('${temp.path}/linked-home').check('o', GitOverlay(repo: linked.bare, reference: 'main')),
          throwsA(isA<OverlayException>().having((e) => e.message, 'message', contains('a link'))));

      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('overlay.cfg', '');

      upstream.entry('100644', 'skills/CON.md', 'device');
      upstream.commit('device name');
      final store = OverlayStore('${temp.path}/overlays');
      await expectLater(store.check('o', GitOverlay(repo: upstream.bare, reference: 'main')),
          throwsA(isA<OverlayException>()));
    });

    test('a tree whose digest is not the pin canoa.cfg names is refused before anything is installed', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('overlay.cfg', '[env]\nMODE = "on"\n');
      upstream.commit('one');
      final store = OverlayStore('${temp.path}/overlays');
      final source = GitOverlay(repo: upstream.bare, reference: 'main');
      final digest = (await store.check('o', source)).digest;
      expect((await store.check('o', source.pinnedTo(digest))).digest, digest);
      await expectLater(store.check('o', source.pinnedTo('sha256:${'0' * 64}')),
          throwsA(isA<OverlayException>().having((e) => e.message, 'message', contains('not the pin'))));
      expect(store.installed('o'), isNull);
    });

    test('a ref the source does not have, and a source git must not run, are refused', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('overlay.cfg', '');
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

  group('a hostile overlay source', skip: hasGit ? false : 'git is not on PATH', () {
    late Crafted source;
    late OverlayStore store;
    setUp(() {
      source = Crafted('${temp.path}/source.git');
      store = OverlayStore('${temp.path}/overlays');
    });
    GitOverlay at(String reference) => GitOverlay(repo: source.dir, reference: reference);

    for (final (what, entries) in <(String, Map<String, Object>)>[
      ('a .git directory in another case, deeper', {'skills': {'.GIT': {'config': 'x'}}}),
      ('the short name Windows gives .git', {'git~1': {'config': 'x'}}),
      ('two names that differ only in case', {'CONTEXT.md': 'benign', 'context.md': 'evil'}),
      ('two directories that differ only in case', {'Kb': {'a.md': 'benign'}, 'kb': {'b.md': 'evil'}}),
      ('a short name Windows could give another file', {'1234567890.md': 'benign', '123456~1.MD': 'evil'}),
      ('a character Windows refuses in a name', {'a|b.md': 'x'}),
      ('a wildcard Windows refuses in a name', {'what?.md': 'x'}),
    ]) {
      test('$what refuses the whole overlay', () async {
        await source.commit(await source.tree({'overlay.cfg': '', ...entries}));
        await expectLater(store.check('o', at('main')), throwsA(isA<OverlayException>()));
      });
    }

    test('a .git directory, and every excluded name, is no part of an overlay: never installed', () async {
      await source.commit(await source.tree({
        'overlay.cfg': '',
        '.git': {'config': '[core]\n\tfsmonitor = "touch pwned #"\n', 'HEAD': 'ref: refs/heads/x\n'},
        'lib': {'__pycache__': {'m.cpython-312.pyc': 'x'}, 'node_modules': {'p': {'index.js': 'x'}}},
      }));
      final update = await store.check('o', at('main'));
      expect(update.files.keys, ['overlay.cfg']);
      expect(update.skipped, unorderedEquals(['.git/HEAD', '.git/config', 'lib/__pycache__/m.cpython-312.pyc',
          'lib/node_modules/p/index.js']));
      await store.apply(update);
      expect(Directory('${store.directoryOf('o')}/.git').existsSync(), isFalse);
      expect(Directory('${store.directoryOf('o')}/lib').existsSync(), isFalse);
    });

    test('a commit id pins that commit, whatever branch or tag carries its name', () async {
      final pinned = await source.commit(await source.tree({'overlay.cfg': 'name = "pinned"\n'}));
      final other = await source.commit(await source.tree({'overlay.cfg': 'name = "other"\n'}), parent: pinned);
      await source.commit(await source.tree({'overlay.cfg': 'name = "other"\n'}), branch: pinned, parent: other);
      expect((await store.check('o', at(pinned))).commit, pinned);
      await expectLater(store.check('o', at(pinned.substring(0, 7))), throwsA(isA<OverlayException>()),
          reason: 'a short id names no commit: a branch could take its name');
    });

    test('moving the pin to the commit installed changes the source, so applying is offered', () async {
      final first = await source.commit(await source.tree({'overlay.cfg': ''}));
      await store.apply(await store.check('o', at('main')));
      final repinned = await store.check('o', at(first));
      expect(repinned.commit, first);
      expect(repinned.changesNothing, isFalse);
      await store.apply(repinned);
      expect(store.installed('o')!.source, at(first).spec);
    });

    test('an update carries the overlay.cfg it installs, for the person to read first', () async {
      await source.commit(await source.tree({'overlay.cfg': 'name = "shown"\n'}));
      expect((await store.check('o', at('main'))).manifest, 'name = "shown"\n');
    });

    test('a tree of a million paths is refused as soon as it passes the limit', () async {
      var level = await source.tree({'f': 'x'});
      for (var depth = 0; depth < 6; depth++) {
        level = await source.tree({for (var i = 0; i < 10; i++) 'd$i': 'tree:$level'});
      }
      await source.commit(await source.tree({'overlay.cfg': '', 'bomb': 'tree:$level'}));
      final clock = Stopwatch()..start();
      await expectLater(store.check('o', at('main')), throwsA(isA<OverlayException>()));
      expect(clock.elapsed, lessThan(const Duration(seconds: 15)));
    });

    test('a million entries under an excluded name are refused as soon as they pass the limit', () async {
      var level = await source.tree({'f': 'x'});
      for (var depth = 0; depth < 6; depth++) {
        level = await source.tree({for (var i = 0; i < 10; i++) 'd$i': 'tree:$level'});
      }
      await source.commit(await source.tree({'overlay.cfg': '', 'node_modules': 'tree:$level'}));
      final clock = Stopwatch()..start();
      await expectLater(store.check('o', at('main')), throwsA(isA<OverlayException>()));
      expect(clock.elapsed, lessThan(const Duration(seconds: 15)));
    });

    test('git is run by its full path, never by a name the current directory could answer', () {
      expect(isAbsolutePath(store.gitPath), isTrue, reason: store.gitPath);
    });
  });

  group('every layer', skip: hasGit ? false : 'git is not on PATH', () {
    test('merges from where each lives; a project profile counts only while trusted as it is', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('profiles/team/overlay.cfg', '''
[env]
GITHUB_TOKEN = { provider = "env", key = "GITHUB_TOKEN" }

[harness.claude-code]
model = "team-model"
effort = "medium"

[[mcp.servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example/mcp"
''');
      upstream.write('profiles/team/CONTEXT.md', 'Be brief.\n');
      upstream.write('profiles/team/skills/review/SKILL.md', '# Review\n');
      upstream.commit('team');

      final configDir = Directory('${temp.path}/config')..createSync();
      final config = CanoaConfig.parse('''
[overlays]
team = "${upstream.bare.replaceAll(r'\', '/')}@main#profiles/team"
[profile]
share_env = ["GITHUB_TOKEN"]
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
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('overlay.cfg', '');
      upstream.commit('one');
      final home = IoProfileHome('${temp.path}/config');
      final source = GitOverlay(repo: upstream.bare, reference: 'main');
      await home.overlays.apply(await home.overlays.check('o', source));
      final moved = {'o': GitOverlay(repo: upstream.bare, reference: 'other')};
      final result = await profileLayers(overlays: moved, user: const Profile(), agents: const {}, agent: 'a', home: home);
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'user']);
      expect(result.notes.single.text, contains('not the source canoa.cfg names'));
    });
    test('an installed copy is digested each time it is used: a change leaves it out, a pin ends unpinned', () async {
      final upstream = Upstream(Directory('${temp.path}/up'));
      upstream.write('overlay.cfg', '[env]\nMODE = "on"\n');
      upstream.commit('one');
      final home = IoProfileHome('${temp.path}/config');
      var config = CanoaConfig.parse('[overlays]\no = "${upstream.bare.replaceAll(r'\', '/')}@main"\n');
      final installed = await home.overlays.apply(await home.overlays.check('o', config.overlays['o']! as GitOverlay));
      Future<ProfileLayers> layers() =>
          profileLayers(overlays: config.overlays, user: const Profile(), agents: const {}, agent: 'a', home: home);

      var result = await layers();
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'overlay:o', 'user']);
      List<Object?> weakenings() => ((result.effective.view()['overlays'] as List).single as Map)['weakenings'] as List;
      expect(weakenings(), [['unpinned', '']]);

      config = CanoaConfig.parse(config.withOverlayPin('o', installed.digest!).encode());
      expect((config.overlays['o']! as GitOverlay).pin, installed.digest, reason: 'the pin is kept in canoa.cfg');
      result = await layers();
      expect(weakenings(), isEmpty);

      final dir = home.overlays.directoryOf('o');
      Directory('$dir/.git').createSync();
      File('$dir/.git/config').writeAsStringSync('[core]\n');
      result = await layers();
      expect(result.layers.map((layer) => '${layer.id}'), ['built-in', 'user']);
      expect(result.notes.last.text, contains('no part of an overlay'));
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
