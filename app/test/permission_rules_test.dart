// What a remembered rule covers and what it never does: the matcher is the one place a request is allowed without the person.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/ledger/ledger_backend_io.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/permissions/real_path_io.dart' show findRepoRoot, resolveExisting;
import 'package:flutter_test/flutter_test.dart';

const _source = 'Scripted';
const _folder = '/work/calc';

/// Links that lead nowhere else, as on a machine with none.
String? _plain(String path) => path;

PermissionRule _rule(RuleType type, String value, {String tool = 'execute', String toolName = '', String source = _source, String peer = '', String folder = _folder}) =>
    PermissionRule(id: 'r1', source: source, peer: peer, folder: folder, tool: tool, toolName: toolName, type: type, value: value, made: DateTime.utc(2026, 11, 12));

RuleSubject _run(String command, {Map<String, Object?> more = const {}, String name = ''}) => subjectOf(
  kind: 'execute',
  rawInput: {'command': command, ...more},
  toolCall: name.isEmpty ? null : {'_meta': {'claudeCode': {'toolName': name}}},
)!;

bool _allows(PermissionRule rule, RuleSubject? subject,
    {String source = _source, String peer = '', String folder = _folder, String? kind, PermissionGuards guards = const PermissionGuards(), RealPath? real = _plain}) =>
    subject != null && rule.matches(source: source, peer: peer, folder: folder, kind: kind ?? rule.tool, subject: subject, guards: guards, real: real);

RuleSubject? _edit(String path, [Map<String, Object?> more = const {}]) =>
    subjectOf(kind: 'edit', rawInput: {'file_path': path, 'old_string': 'a', 'new_string': 'b', ...more});

void main() {
  fileBackendTests();
  group('a rule for an exact command', () {
    final rule = _rule(RuleType.command, 'npm test');

    test('allows that command, from that source, in that folder', () {
      expect(_allows(rule, _run('npm test')), isTrue);
    });

    test('a near miss, or a command with more after it, still asks: there are no prefix rules', () {
      for (final other in ['npm test ', ' npm test', 'npm  test', 'npm test -- a.js', 'npm test; rm -rf ~', 'NPM test', 'npm tests', '', 'npm test --silent']) {
        expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': other})), isFalse, reason: other);
      }
    });

    test('another folder, another source, another peer and another kind of tool still ask', () {
      expect(_allows(rule, _run('npm test'), folder: '/work/other'), isFalse);
      expect(_allows(rule, _run('npm test'), folder: '/work/calc/sub'), isFalse, reason: 'here is this folder, not what is under it');
      expect(_allows(rule, _run('npm test'), source: 'Other'), isFalse);
      expect(_allows(rule, _run('npm test'), peer: 'peer-b'), isFalse);
      expect(_allows(rule, _run('npm test'), kind: 'edit'), isFalse);
      expect(rule.matches(source: _source, folder: _folder, kind: null, subject: _run('npm test')), isFalse);
    });

    test('the tool the agent names has to be the same: a Bash rule never covers PowerShell', () {
      final bash = _rule(RuleType.command, 'npm test', toolName: 'Bash');
      expect(_allows(bash, _run('npm test', name: 'Bash')), isTrue);
      expect(_allows(bash, _run('npm test', name: 'PowerShell')), isFalse);
      expect(_allows(bash, _run('npm test')), isFalse, reason: 'a request that names no tool is not one that named Bash');
      expect(_allows(rule, _run('npm test', name: 'Bash')), isFalse, reason: 'nor the other way round');
    });

    test('input that says more than a command is never covered', () {
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test', 'run_in_background': true})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test', 'cwd': '/'})), isFalse);
      expect(_allows(rule, _run('npm test', more: {'description': 'Run the tests', 'timeout': 5000})), isTrue,
          reason: 'a description and a timeout change nothing about what runs');
    });

    test('a command written another way is not the same command', () {
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm\u00a0test'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test\n'})), isFalse);
    });

    test('a rule is made only for a command of printable ASCII: nothing that looks like another character', () {
      for (final command in [
        'npm test\n', 'npm\ttest', 'npm test\u202e', 'npm test\u200b', 'npm test\u2028x', 'npm test\u0007', 'npm test\ufeff',
        'echo \u201chi\u201d', 'ls \u2013la', 'ls \u2014la', 'ls\u00a0-la', 'ls\u2003-la', 'ls\u3000-la', 'npm test\u{e0041}', 'npm\u034f test',
        'npm\u3164test', 'npm\u180etest', 'caf\u00e9', 'ls \u2212la',
      ]) {
        final subject = subjectOf(kind: 'execute', rawInput: {'command': command})!;
        expect(scopeProblem(RuleType.command, command, subject, _folder), isNotNull, reason: command.codeUnits.toString());
      }
      expect(scopeProblem(RuleType.command, 'npm test', _run('npm test'), _folder), isNull);
      expect(scopeProblem(RuleType.command, 'npm', _run('npm test'), _folder), isNotNull);
    });
  });

  group('a path as the agent wrote it', () {
    test('is read only when absolute: nothing relative, ~, UNC, device, stream or drive-relative', () {
      for (final path in [
        'src/a.js', './a.js', '../a.js', 'a.js', '~/a.js', '~', '~root/x', r'\\server\share\a.js', '//server/share/a.js', r'\\?\C:\work\a.js',
        'C:a.js', r'C:work\a.js', '/work/calc/a.js:stream', r'C:\work\calc\a.js:$DATA', '/work/calc/x:y/a.js', '/C:/work/a.js', '', '/work\u0000/a',
        '/..', '/work/../../a.js',
      ]) {
        expect(_edit(path), isNull, reason: path);
        expect(cleanPath(path), isNull, reason: path);
      }
      expect(cleanPath('/work//calc/./src/../a.js'), '/work/calc/a.js');
      for (final path in ['/work/calc/src/j/../evil', '/work/calc/./src/a.js', '/work/calc/src/..', r'C:\work\calc\src\j\..\evil', '/work/calc/src/a/./b']) {
        expect(_edit(path), isNull, reason: 'the system resolves .. after links: $path');
      }
      expect(scopeProblem(RuleType.path, '/work/calc/src/../src', _edit('/work/calc/src/a.js')!, _folder, real: _plain), contains('.. '));
      expect(cleanPath(r'C:\Work\Calc\a.js'), 'c:/work/calc/a.js');
      expect(cleanPath('c:/Work'), 'c:/work');
    });
  });

  group('a rule for changes under a path', () {
    final rule = _rule(RuleType.path, '/work/calc/src', tool: 'edit');

    test('covers the file and what is under it', () {
      expect(_allows(rule, _edit('/work/calc/src')), isTrue);
      expect(_allows(rule, _edit('/work/calc/src/a.js')), isTrue);
      expect(_allows(rule, _edit('/work/calc/src/deep/er/a.js')), isTrue);
      expect(_allows(rule, _edit('/work/calc/./src/../src/a.js')), isFalse, reason: 'a path with . or .. is not read at all');
    });

    test('never covers outside it, by a sibling name or by climbing out', () {
      for (final path in ['/work/calc/srcx/a.js', '/work/calc/a.js', '/work/calc/src/../a.js', '/etc/passwd', '/work/calc/src/../../other/a.js']) {
        expect(_allows(rule, _edit(path)), isFalse, reason: path);
      }
    });

    test('never covers a change that names a second place or anything it does not know', () {
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'new_path': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js', 'path': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js', 'destination': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'paths': ['/work/calc/src/a.js']})), isFalse);
    });

    test('takes edits only as a list of old and new strings, and no patch or diff', () {
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'edits': [{'old_string': 'a', 'new_string': 'b', 'replace_all': true}]})), isTrue);
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'edits': [{'old_string': 'a', 'new_string': 'b', 'path': '/etc/x'}]})), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'edits': ['a']})), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'edits': 'a'})), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'patch': '--- /etc/x'})), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js', {'diff': '--- /etc/x'})), isFalse);
    });

    test('does not cover another folder, source or kind of tool', () {
      expect(_allows(rule, _edit('/work/calc/src/a.js'), folder: '/work/other'), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js'), source: 'Other'), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js'), kind: 'delete'), isFalse);
    });

    test('a rule whose path lies outside its folder, or is not absolute, covers nothing', () {
      expect(_allows(_rule(RuleType.path, '/etc', tool: 'edit'), _edit('/etc/passwd')), isFalse);
      expect(_allows(_rule(RuleType.path, '../other', tool: 'edit'), _edit('/work/other/a.js')), isFalse);
      expect(_allows(_rule(RuleType.path, 'src', tool: 'edit'), _edit('/work/calc/src/a.js')), isFalse);
    });

    test('a Windows path has no case and either slash', () {
      final win = _rule(RuleType.path, r'C:\Work\Calc\src', tool: 'edit', folder: r'C:\Work\Calc');
      expect(_allows(win, _edit(r'c:/work/calc/SRC/a.js'), folder: r'c:\work\calc'), isTrue);
      expect(_allows(win, _edit(r'C:\Work\Other\a.js'), folder: r'C:\Work\Calc'), isFalse);
      expect(_allows(win, _edit(r'D:\Work\Calc\src\a.js'), folder: r'C:\Work\Calc'), isFalse);
    });

    test('is made nowhere links cannot be followed: without a resolver it covers nothing', () {
      expect(_allows(rule, _edit('/work/calc/src/a.js'), real: null), isFalse);
      expect(pathsProblem(null), isNotNull);
      expect(scopeProblem(RuleType.path, '/work/calc/src', _edit('/work/calc/src/a.js')!, _folder), isNotNull, reason: 'no resolver given');
    });
  });

  group('names a path rule never covers', () {
    final wide = _rule(RuleType.path, '/work/calc', tool: 'edit');

    test('every segment that starts with a dot, at any depth', () {
      for (final path in [
        '/work/calc/.git/config', '/work/calc/sub/.git/hooks/pre-commit', '/work/calc/.claude/settings.json', '/work/calc/.codex/config.toml',
        '/work/calc/.gemini/settings.json', '/work/calc/.cursor/rules', '/work/calc/.vscode/tasks.json', '/work/calc/.idea/a.xml',
        '/work/calc/.husky/pre-commit', '/work/calc/.mcp.json', '/work/calc/.envrc', '/work/calc/.env', '/work/calc/.github/workflows/ci.yml',
        '/work/calc/src/.hidden/x', '/work/calc/src/.npmrc', '/work/calc/.GIT/config', '/work/calc/..foo/x', '/work/calc/...x',
      ]) {
        expect(_allows(wide, _edit(path)), isFalse, reason: path);
      }
    });

    test('a name Windows reads as another: trailing dots and spaces, and short names', () {
      for (final path in [
        '/work/calc/.claude./settings.json', '/work/calc/.claude../settings.json', '/work/calc/.claude /settings.json', '/work/calc/package.json.',
        '/work/calc/package.json ', '/work/calc/package.json. .', '/work/calc/Makefile.', '/work/calc/GIT~1/config', '/work/calc/PACKAGE~1.JSO',
        '/work/calc/sub/CLAUDE~1.MD', '/work/calc/ /x', '/work/calc/... /x',
      ]) {
        expect(_allows(wide, _edit(path)), isFalse, reason: path);
      }
    });

    test('a Windows device name, with or without an extension', () {
      for (final name in ['CON', 'con', 'PRN', 'AUX', 'NUL', 'COM1', 'com9', 'LPT1', 'lpt9', 'CON.txt', 'nul.json', 'aux.a.b', 'COM1 .', 'sub/CON/x']) {
        expect(_allows(wide, _edit('/work/calc/$name')), isFalse, reason: name);
      }
      for (final name in ['CONSOLE', 'COM10', 'console.log', 'auxiliary.txt', 'nullable.dart', 'com']) {
        expect(_allows(wide, _edit('/work/calc/$name')), isTrue, reason: name);
      }
    });

    test('the longer list of files that run code, and its known gaps are not covered', () {
      for (final path in [
        'venv/lib/x.py', 'env/bin/activate', 'a/__pycache__/x.pyc', 'x.pth', 'pytest.ini', 'tox.ini', 'setup.cfg', 'noxfile.py', 'requirements.txt',
        'requirements-dev.txt', 'my.code-workspace', 'shell.nix', 'flake.nix', 'gradlew', 'mvnw', 'gradle/wrapper/gradle-wrapper.properties',
        'pom.xml', 'build.gradle', 'build.gradle.kts', 'rust-toolchain', 'rust-toolchain.toml', 'go.mod', 'go.work', 'App.csproj', 'Directory.Packages.props',
        'nuget.config', 'NuGet.Config', 'CMakeLists.txt', 'meson.build', 'Gemfile', 'Dockerfile', 'vendor/x/y.go', 'vite.config.js', 'vite.config.mts',
        'webpack.config.babel.js', 'gulpfile.js', 'Gruntfile.coffee',
      ]) {
        expect(_allows(wide, _edit('/work/calc/$path')), isFalse, reason: path);
      }
    });

    test('a name with anything but printable ASCII, which has more than one spelling', () {
      for (final path in [
        '/work/calc/.g\u200bit/config', '/work/calc/pack\u00adage.json', '/work/calc/caf\u00e9/a.js', '/work/calc/cafe\u0301/a.js', '/work/calc/\u202eevil/a',
        '/work/calc/src/\u00e5.js', '/work/calc/a\u200db.js', '/work/calc/a\tb.js',
      ]) {
        expect(_allows(wide, _edit(path)), isFalse, reason: path.codeUnits.toString());
      }
    });

    test('files that run code or hold agent instructions, with or without case', () {
      for (final path in [
        'package.json', 'web/package.json', 'Makefile', 'makefile', 'justfile', 'pyproject.toml', 'setup.py', 'build.rs', 'Rakefile', 'Cargo.toml',
        'conftest.py', 'sub/conftest.py', 'AGENTS.md', 'AGENTS.override.md', 'CLAUDE.md', 'sub/CLAUDE.local.md', 'GEMINI.md', 'gemini.md',
        'Directory.Build.props', 'Directory.Build.targets', 'mise.toml', 'node_modules/x/index.js', 'vite.config.js', 'a/webpack.config.mjs',
        'jest.config.cjs', 'tailwind.config.ts', 'VITE.CONFIG.JS',
      ]) {
        expect(_allows(wide, _edit('/work/calc/$path')), isFalse, reason: path);
      }
    });

    test('and an ordinary file is still covered, a dot above the session folder does not matter', () {
      expect(_allows(wide, _edit('/work/calc/src/main.dart')), isTrue);
      expect(_allows(wide, _edit('/work/calc/README.md')), isTrue);
      expect(_allows(wide, _edit('/work/calc/a.conf.json')), isTrue);
      final hidden = _rule(RuleType.path, '/work/.hidden/calc', tool: 'edit', folder: '/work/.hidden/calc');
      expect(_allows(hidden, _edit('/work/.hidden/calc/a.js'), folder: '/work/.hidden/calc'), isTrue);
      expect(_allows(hidden, _edit('/work/.hidden/calc/.x/a.js'), folder: '/work/.hidden/calc'), isFalse);
    });

    test('a scope that is one of them is refused with the reason', () {
      expect(scopeProblem(RuleType.path, '/work/calc/.git', _edit('/work/calc/.git/config')!, _folder, real: _plain), contains('protected list'));
      expect(scopeProblem(RuleType.path, '/work/calc', _edit('/work/calc/.claude./settings.json')!, _folder, real: _plain), contains('protected list'));
      expect(scopeProblem(RuleType.path, '/work/calc/src', _edit('/work/calc/src/a.js')!, _folder, real: _plain), isNull);
    });
  });

  group('home and Canoa\'s settings', () {
    const guards = PermissionGuards(home: '/home/u', config: '/home/u/.config/canoa');

    test('no path rule at all where the session folder is home, holds it, or touches Canoa\'s settings', () {
      for (final folder in ['/home/u', '/home', '/', '/home/u/.config/canoa', '/home/u/.config/canoa/sub']) {
        expect(folderProblem(folder, guards: guards, real: _plain), isNotNull, reason: folder);
      }
      for (final folder in ['/home/u/proj', '/work/calc', '/home/other']) {
        expect(folderProblem(folder, guards: guards, real: _plain), isNull, reason: folder);
      }
      final inHome = _rule(RuleType.path, '/home/u/a', tool: 'edit', folder: '/home/u');
      expect(_allows(inHome, _edit('/home/u/a/x.txt'), folder: '/home/u', guards: guards), isFalse);
    });

    test('are read after their links are resolved too', () {
      String? real(String path) => path == '/home/u' ? '/data/u' : path.startsWith('/home/u/') ? '/data/u${path.substring(7)}' : path;
      expect(folderProblem('/data/u', guards: guards, real: real), isNotNull, reason: 'the folder is what home links to');
      expect(folderProblem('/data', guards: guards, real: real), isNotNull, reason: 'it holds what home links to');
      expect(folderProblem('/data/u/proj', guards: guards, real: real), isNull);
      String? configLink(String path) => path.startsWith('/work/cfg') ? '/home/u/.config/canoa${path.substring(9)}' : path;
      expect(folderProblem('/work/cfg', guards: const PermissionGuards(config: '/home/u/.config/canoa'), real: configLink), isNotNull);
    });

    test('a request inside them is never covered, even by a rule made before', () {
      final inConfig = _rule(RuleType.path, '/home/u/.config/canoa/x', tool: 'edit', folder: '/home/u/proj');
      expect(_allows(inConfig, _edit('/home/u/.config/canoa/x/a'), folder: '/home/u/proj', guards: guards), isFalse);
    });
  });

  group('links in a path', () {
    final rule = _rule(RuleType.path, '/work/calc/src', tool: 'edit');
    final links = {'/work/calc/src/out': '/etc', '/work/calc/src/hooks': '/work/calc/.git/hooks', '/work/calc/src/pkg': '/work/calc/package.json'};
    String? real(String path) {
      for (final entry in links.entries) {
        if (path == entry.key || path.startsWith('${entry.key}/')) return entry.value + path.substring(entry.key.length);
      }
      return path;
    }

    test('a link that leads out of the folder, or into a protected place, covers nothing', () {
      expect(_allows(rule, _edit('/work/calc/src/a.js'), real: real), isTrue);
      expect(_allows(rule, _edit('/work/calc/src/out/passwd'), real: real), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/hooks/pre-commit'), real: real), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/pkg'), real: real), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/a.js'), real: (_) => null), isFalse, reason: 'a link that cannot be followed');
      expect(scopeProblem(RuleType.path, '/work/calc/src', _edit('/work/calc/src/out/x')!, _folder, real: real), isNotNull);
    });

    test('real links on this machine are followed, to the deepest part that exists', () {
      final dir = Directory.systemTemp.createTempSync('canoa-rules-');
      addTearDown(() => dir.deleteSync(recursive: true));
      final inside = Directory('${dir.path}/folder/src')..createSync(recursive: true);
      final outside = Directory('${dir.path}/outside')..createSync();
      try {
        Link('${inside.path}/out').createSync(outside.path);
      } on FileSystemException {
        markTestSkipped('this machine cannot make a link here');
        return;
      }
      final folder = cleanPath('${dir.path}/folder')!;
      final scope = '$folder/src';
      expect(pathProblem(scope: scope, file: '$scope/a.js', folder: folder, real: resolveExisting), isNull, reason: 'a file not made yet');
      expect(pathProblem(scope: scope, file: '$scope/out/a.js', folder: folder, real: resolveExisting), isNotNull);
      expect(pathProblem(scope: scope, file: '$scope/out/deeper/new/a.js', folder: folder, real: resolveExisting), isNotNull);
      expect(resolveExisting('${dir.path}/nowhere/x/y'), isNotNull, reason: 'what does not exist is kept as written after the part that does');
    });
  });

  group('a rule for an MCP tool', () {
    final rule = _rule(RuleType.mcp, 'mcp__marker__get_mark', tool: 'other', toolName: 'mcp__marker__get_mark');
    RuleSubject? mcp(String name, {String kind = 'other'}) =>
        subjectOf(kind: kind, rawInput: {'x': 1}, toolCall: {'_meta': {'claudeCode': {'toolName': name}}});

    test('allows that tool on that server with any input, and no other', () {
      expect(_allows(rule, mcp('mcp__marker__get_mark')), isTrue);
      expect(_allows(rule, mcp('mcp__marker__get_mark2')), isFalse);
      expect(_allows(rule, mcp('mcp__other__get_mark')), isFalse);
      expect(_allows(rule, mcp('mcp__marker__get_mark'), source: 'Other'), isFalse);
    });

    test('is made only from the structured tool name, never from a title', () {
      expect(subjectOf(kind: 'other', rawInput: {'x': 1}), isNull);
      expect(subjectOf(kind: 'other', rawInput: {'x': 1}, toolCall: {'title': 'mcp__marker__get_mark'}), isNull);
      expect(mcp('mcp__a__get__c'), isA<McpSubject>().having((s) => s.server, 'server', 'a'));
      expect(mcp('mcp____x'), isNull, reason: 'a server named by nothing');
      expect(mcp('mark'), isNull);
    });

    test('is offered only for a tool whose name begins with a verb that reads', () {
      for (final name in ['read', 'read_file', 'get_mark', 'list_directory', 'search_files', 'find', 'view_image', 'show', 'describe_table', 'status', 'stat', 'info', 'getUser', 'readFile', 'List-things']) {
        expect(mcp('mcp__fs__$name'), isA<McpSubject>(), reason: name);
      }
      for (final name in ['write_file', 'edit_file', 'move_file', 'create_directory', 'delete', 'mark', 'fetch', 'send_message', 'upload', 'put', 'set', 'rename', 'copy_file', 'append', 'listen_and_write', 'x_read', 'readonly_but_unknown']) {
        expect(mcp('mcp__fs__$name'), name == 'readonly_but_unknown' ? isNull : isNull, reason: name);
      }
    });

    test('is not offered when a later word writes or joins another action', () {
      for (final name in [
        'get_or_create_issue', 'search_replace', 'find-and-replace', 'read_write', 'statusSet', 'getOrCreate', 'view_update', 'describe_and_drop',
        'infoDelete', 'readFileAndPost', 'get__write', 'list_then_delete', 'read_upsert', 'get_and', 'show_apply_patch', 'find_commit', 'status_reset',
        'list_grant', 'read_install', 'search_save', 'view_store', 'get_clear', 'describe_truncate', 'info_publish', 'stat_mv', 'get_rm', 'read_start',
      ]) {
        expect(mcp('mcp__fs__$name'), isNull, reason: name);
      }
      for (final name in ['read_file', 'get_issue', 'list_all_issues', 'search_files_by_name', 'getIssueById', 'view_image', 'describe_table']) {
        expect(mcp('mcp__fs__$name'), isA<McpSubject>(), reason: name);
      }
      expect(readsOnly('get_or_create'), isFalse);
      expect(nameWords('readFileAndPost'), ['read', 'file', 'and', 'post']);
      expect(nameWords('get__write'), ['get', 'write']);
    });

    test('a name that claims to be an MCP tool and is not shaped like one is nothing, never a command or a path', () {
      for (final name in ['mcp____x', 'mcp__x__', 'mcp__', 'mcp__x', 'mcp____']) {
        final call = {'_meta': {'claudeCode': {'toolName': name}}};
        expect(subjectOf(kind: 'execute', rawInput: {'command': 'ls'}, toolCall: call), isNull, reason: name);
        expect(subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js'}, toolCall: call), isNull, reason: name);
      }
      expect(subjectOf(kind: 'execute', rawInput: {'command': 'ls'}, toolCall: {'_meta': {'claudeCode': {'toolName': 'Bash'}}}), isA<CommandSubject>());
    });

    test('is never made for a tool that executes, by its kind or by its name', () {
      expect(mcp('mcp__shell__list', kind: 'execute'), isNull);
      for (final name in ['mcp__marker__run', 'mcp__marker__exec_cmd', 'mcp__terminal__open', 'mcp__marker__process_file', 'mcp__marker__eval', 'mcp__sh__Shell',
        'mcp__marker__run_script', 'mcp__marker__powershell', 'mcp__marker__bash', 'mcp__marker__get_shell', 'mcp__terminal__read_output', 'mcp__marker__list_processes']) {
        expect(mcp(name), isNull, reason: name);
      }
      expect(mcp('mcp__marker__get_mark'), isA<McpSubject>());
    });

    test('is described by tool and server', () {
      expect(rule.describe, 'Use the tool get_mark of marker, with any input');
      expect(_rule(RuleType.mcp, 'mcp__my_server__do_it').describe, 'Use the tool do_it of my_server, with any input');
    });
  });

  group('what a rule can be made of', () {
    test('a tool of another kind, or with no command, has no subject', () {
      expect(subjectOf(kind: 'read', rawInput: {'file_path': '/a'}), isNull);
      expect(subjectOf(kind: 'execute', rawInput: {'command': ''}), isNull);
      expect(subjectOf(kind: 'execute', rawInput: 'ls'), isNull);
      expect(subjectOf(kind: 'execute'), isNull);
    });

    test('the scope a person edits has to cover the request it is made from', () {
      final file = _edit('/work/calc/src/a.js')!;
      expect(scopeProblem(RuleType.path, '/work/calc/src', file, _folder, real: _plain), isNull);
      expect(scopeProblem(RuleType.path, 'src/a.js', file, _folder, real: _plain), isNotNull, reason: 'not absolute');
      expect(scopeProblem(RuleType.path, '/work', file, _folder, real: _plain), isNotNull, reason: 'wider than the folder');
      expect(scopeProblem(RuleType.path, '/work/calc/lib', file, _folder, real: _plain), isNotNull);
      expect(scopeProblem(RuleType.path, '/work/calc', _run('ls'), _folder, real: _plain), isNotNull, reason: 'a command has no path');
      expect(scopeProblem(RuleType.unsupported, 'x', _run('ls'), _folder), isNotNull);
    });
  });

  group('a rule an earlier version wrote', () {
    const legacy = '{"version":1,"rules":[{"id":"old","source":"$_source","folder":"$_folder","tool":"execute","type":"prefix","value":"npm test","made":"2026-11-01T00:00:00Z","fired":4,"by":"canoa"}]}';

    test('loads, is shown as no longer supported, never applies, and is written back as it was', () async {
      final backend = MemoryLedgerBackend()..document = legacy;
      final rules = PermissionRules(backend);
      await rules.load();
      final old = rules.rules.single;
      expect((old.type, old.typeName, old.fired), (RuleType.unsupported, 'prefix', 4));
      expect(old.describe, startsWith('No longer supported (prefix)'));
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test -- a.js')), isNull);
      expect(rules.conflict(_folder, RuleType.path), isNull, reason: 'an inactive rule holds nothing back');
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls -la');
      await rules.settled();
      expect(jsonDecode(backend.document!)['rules'][0]['type'], 'prefix');
      expect(rules.revoke('old'), isTrue);
    });
  });

  group('the list of rules', () {
    PermissionRules make([MemoryLedgerBackend? backend, RuleLog? log]) =>
        PermissionRules(backend ?? MemoryLedgerBackend(), log: log, real: _plain, clock: () => DateTime.utc(2026, 11, 12, 8));

    test('keeps what is made, counts uses, and revoking stops one matching', () async {
      final backend = MemoryLedgerBackend();
      final rules = make(backend);
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test'), same(rule),
          reason: 'the same rule is not made twice');
      final subject = _run('npm test');
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: subject), same(rule));
      rules.recordUse(rule, 'npm test');
      rules.recordUse(rule, 'npm test');
      await rules.settled();
      final reread = make(backend);
      await reread.load();
      expect(reread.rules.single.fired, 2);
      expect(reread.rules.single.value, 'npm test');
      expect(reread.rules.single.external, isFalse);
      expect(reread.rules.single.made, DateTime.utc(2026, 11, 12, 8));
      expect(reread.revoke(rule.id), isTrue);
      expect(reread.revoke(rule.id), isFalse);
      expect(reread.match(source: _source, folder: _folder, kind: 'execute', subject: subject), isNull);
      await reread.settled();
      final again = make(backend);
      await again.load();
      expect(again.rules, isEmpty, reason: 'the revoke was written');
    });

    test('a folder keeps one kind of rule: commands, tools and file changes each exclude the other two, for any source or peer', () {
      final rules = make();
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      expect(rules.conflict(_folder, RuleType.path), isNotNull);
      expect(rules.conflict(_folder, RuleType.mcp), isNotNull, reason: 'a tool that writes files beside a command that runs them');
      expect(rules.conflict('/work/calc/sub', RuleType.path), isNotNull, reason: 'a folder inside it');
      expect(rules.conflict('/work', RuleType.mcp), isNotNull, reason: 'a folder that holds it');
      expect(rules.conflict('/work/other', RuleType.path), isNull);
      expect(rules.conflict(_folder, RuleType.command), isNull, reason: 'the same kind stands together');
      final tools = make();
      tools.add(source: 'Elsewhere', peer: 'peer-b', folder: _folder, kind: 'other', toolName: 'mcp__a__get_b', type: RuleType.mcp, value: 'mcp__a__get_b');
      expect(tools.conflict(_folder, RuleType.command), isNotNull, reason: 'another source and peer count');
      expect(tools.conflict(_folder, RuleType.path), isNotNull);
      expect(tools.conflict(_folder, RuleType.mcp), isNull);
      final paths = make();
      paths.add(source: _source, folder: _folder, kind: 'edit', type: RuleType.path, value: '/work/calc/src');
      expect(paths.conflict('/work', RuleType.command), isNotNull);
      expect(paths.conflict(_folder, RuleType.mcp), isNotNull);
      expect(paths.conflict(_folder, RuleType.path), isNull);
    });

    test('folders are compared once links are resolved, and a repository counts as one tree', () {
      String? junction(String path) => path.startsWith('/alias') ? '/work/calc${path.substring(6)}' : path;
      final linked = PermissionRules(MemoryLedgerBackend(), real: junction);
      linked.add(source: _source, folder: '/alias', kind: 'execute', type: RuleType.command, value: 'ls');
      expect(linked.conflict('/work/calc/sub', RuleType.path), isNotNull, reason: 'a link into the same tree');
      expect(linked.conflict('/work/elsewhere', RuleType.path), isNull);
      String? root(String folder) => folder.startsWith('/repo') ? '/repo' : null;
      final repo = PermissionRules(MemoryLedgerBackend(), real: _plain, repoRoot: root);
      repo.add(source: _source, folder: '/repo/a', kind: 'execute', type: RuleType.command, value: 'ls');
      expect(repo.conflict('/repo/b', RuleType.path), isNotNull, reason: 'the same repository, in another folder');
      expect(repo.conflict('/other/b', RuleType.path), isNull);
      final unresolvable = PermissionRules(MemoryLedgerBackend(), real: (_) => null);
      unresolvable.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls');
      expect(unresolvable.conflict('/anywhere', RuleType.path), isNotNull, reason: 'a folder that cannot be placed is treated as overlapping');
    });

    test('a file that holds both for one place applies neither', () async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":['
            '{"id":"a","source":"$_source","folder":"$_folder","tool":"execute","type":"command","value":"npm test","made":"2026-11-12T00:00:00Z","by":"canoa"},'
            '{"id":"b","source":"$_source","folder":"/work/calc/src","tool":"edit","type":"path","value":"/work/calc/src","made":"2026-11-12T00:00:00Z","by":"canoa"}]}';
      final rules = make(backend);
      await rules.load();
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
      expect(rules.match(source: _source, folder: '/work/calc/src', kind: 'edit', subject: _edit('/work/calc/src/a.js')!), isNull);
    });

    test('a file that stops parsing applies no rule at all, says so, is not written over, and applies again once it is right', () async {
      final backend = MemoryLedgerBackend();
      final rules = make(backend);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      await rules.settled();
      final good = backend.document!;
      backend.document = '{not json';
      await rules.refresh();
      expect(rules.rules, isEmpty);
      expect(rules.problem, contains('none apply'));
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
      expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'pwd'), isNull, reason: 'no rule is made over a file that cannot be read');
      await rules.settled();
      expect(backend.document, '{not json', reason: 'what it holds is left to be put right');
      backend.document = good;
      await rules.refresh();
      expect(rules.problem, isNull);
      expect(rules.rules.single.value, 'npm test');
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNotNull);
    });

    test('a file that cannot be read applies no rule either', () async {
      final backend = _FailingBackend()..fail = false;
      final rules = PermissionRules(backend);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      await rules.settled();
      backend.failReads = true;
      await rules.refresh();
      expect(rules.rules, isEmpty);
      expect(rules.problem, contains('could not read'));
    });

    test('a file that is not a list of rules leaves none and says so', () async {
      final backend = MemoryLedgerBackend()..document = '{"version":2}';
      final rules = make(backend);
      await rules.load();
      expect(rules.rules, isEmpty);
      expect(rules.problem, contains('none apply'));
    });

    test('an entry that is not a rule is dropped, the rest kept', () async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":[{"id":"x"},{"id":"a","source":"s","folder":"/f","tool":"execute","type":"command","value":"ls","made":"2026-11-12T00:00:00Z","fired":-3}]}';
      final rules = make(backend);
      await rules.load();
      expect(rules.rules.map((rule) => rule.id), ['a']);
      expect(rules.rules.single.fired, 0);
    });

    test('a rule the app did not write is flagged, and a rule added to the file meanwhile is kept by the next write', () async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":[{"id":"hand","source":"s","folder":"/f","tool":"execute","type":"command","value":"ls","made":"2026-11-12T00:00:00Z"}]}';
      final rules = make(backend);
      await rules.load();
      expect(rules.rules.single.external, isTrue);
      final mine = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      await rules.settled();
      backend.document = backend.document!.replaceFirst('"rules":[', '"rules":[{"id":"late","source":"s","folder":"/g","tool":"execute","type":"command","value":"pwd","made":"2026-11-12T00:00:00Z"},');
      rules.recordUse(mine, 'npm test');
      await rules.settled();
      final again = make(backend);
      await again.load();
      expect(again.rules.map((rule) => rule.id), containsAll(['hand', 'late', mine.id]));
      expect(again.rules.firstWhere((rule) => rule.id == 'late').external, isTrue);
      expect(again.rules.firstWhere((rule) => rule.id == mine.id).external, isFalse);
    });

    test('a revoke made by another instance wins: its rule stops applying at the next read and is not written back', () async {
      final backend = MemoryLedgerBackend();
      final first = make(backend);
      final kept = first.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls -la')!;
      final revoked = first.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      await first.settled();
      final second = make(backend);
      await second.load();
      expect(second.revoke(revoked.id), isTrue);
      await second.settled();
      expect(first.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNotNull, reason: 'not yet read');
      // A rule made here and not yet written is not mistaken for one revoked elsewhere.
      final fresh = first.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'pwd')!;
      await first.refresh();
      expect(first.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
      expect(first.rules.map((rule) => rule.id), containsAll([kept.id, fresh.id]));
      expect(first.rules.map((rule) => rule.id), isNot(contains(revoked.id)));
      await first.settled();
      final again = make(backend);
      await again.load();
      expect(again.rules.map((rule) => rule.id), unorderedEquals([kept.id, fresh.id]));
    });

    test('a revoked rule is not brought back by the file it was read from', () async {
      final backend = MemoryLedgerBackend();
      final rules = make(backend);
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      await rules.settled();
      final kept = backend.document;
      rules.revoke(rule.id);
      backend.document = kept;
      await rules.settled();
      expect(rules.rules, isEmpty);
    });

    test('a revoke whose write failed still holds: it is not matched, is shown as not yet saved, and is written at the next read', () async {
      final backend = _FailingBackend()..fail = false;
      final rules = PermissionRules(backend, real: _plain);
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      await rules.settled();
      expect(rules.unsaved, isFalse);
      backend.fail = true;
      rules.revoke(rule.id);
      await rules.settled();
      expect(rules.unsaved, isTrue);
      expect(rules.writeProblem, contains('could not write'));
      expect(rules.rules, isEmpty);
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull, reason: 'a revoke applies at once');
      await rules.refresh();
      expect(rules.rules, isEmpty, reason: 'the file still holds the rule, and reading it does not bring it back');
      expect(rules.unsaved, isTrue);
      backend.fail = false;
      await rules.refresh();
      expect(rules.unsaved, isFalse);
      expect(rules.writeProblem, isNull);
      expect(jsonDecode(backend.document!)['rules'], isEmpty, reason: 'the pending revoke is written once the disk allows');
    });

    test('rules come back when a file that failed to read reads again, with no restart', () async {
      final backend = _FailingBackend()..fail = false;
      final rules = PermissionRules(backend, real: _plain);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      await rules.settled();
      backend.failReads = true;
      await rules.refresh();
      expect(rules.rules, isEmpty);
      expect(rules.problem, isNotNull);
      backend.failReads = false;
      await rules.refresh();
      expect(rules.problem, isNull);
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNotNull);
    });

    test('a read that cannot get the lock in time applies no rule', () async {
      final backend = _StuckLockBackend();
      final rules = PermissionRules(backend, real: _plain, lockTimeout: const Duration(milliseconds: 50));
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      await rules.settled();
      await rules.refresh();
      expect(rules.rules, isEmpty);
      expect(rules.problem, contains('did not finish'));
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
    });

    test('a failed write is said, and a later one clears it', () async {
      final backend = _FailingBackend();
      final rules = PermissionRules(backend);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      await rules.settled();
      expect(rules.writeProblem, contains('could not write'));
      backend.fail = false;
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm run x');
      await rules.settled();
      expect(rules.writeProblem, isNull);
    });

    test('the log is appended to, and holds the program of a command and nothing after it, no hash and no assignment', () async {
      final log = MemoryRuleLog();
      final rules = make(MemoryLedgerBackend(), log);
      const secret = 'TOKEN=sk-live-123456 curl -H "Authorization: Bearer sk-live-123456" https://user:pw@api.example.test/x';
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: secret)!;
      rules.recordUse(rule, secret);
      rules.revoke(rule.id);
      await rules.settled();
      expect(log.lines, hasLength(3));
      for (final line in log.lines) {
        for (final leak in ['sk-live', 'Authorization', 'user:pw', 'TOKEN', 'api.example', 'sha256']) {
          expect(line, isNot(contains(leak)), reason: leak);
        }
        expect(line, contains('"what":"(command)"'));
        expect(RegExp('[0-9a-f]{64}').hasMatch(line), isFalse, reason: 'no hash of the text');
      }
      expect(log.lines[0], allOf(contains('"event":"made"'), contains('"rule":"${rule.id}"')));
      expect(log.lines[1], allOf(contains('"event":"used"'), contains('2026-11-12T08:00:00.000Z')));
      expect(log.lines[2], contains('"event":"revoked"'));
      final before = List.of(log.lines);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls -la');
      await rules.settled();
      expect(log.lines.take(3).toList(), before, reason: 'a line once written is not touched');
    });

    test('what a log line says of a command: a program when it begins with one, else (command); never an argument', () {
      expect(redacted(RuleType.command, 'ls'), 'ls');
      expect(redacted(RuleType.command, 'ls -la /secret'), 'ls …');
      expect(redacted(RuleType.command, r'C:\tools\run.exe --token x'), r'C:\tools\run.exe …');
      expect(redacted(RuleType.command, './deploy.sh now'), './deploy.sh …');
      for (final command in [
        'A=1 B=2 make test', r'$env:TOKEN=abc; ls', r'$env:TOKEN=abc', 'TOKEN=abc', r'$name=hunter2 ls', r'$password = "x y"', 'PASSWORD="hunter2 hunter3" ./deploy.sh',
        "PASSWORD='a b' ./deploy.sh", 'https://u:p@h.example/x', '"quoted prog" arg', r'$(cat secret)', '@echo off', 'user@host ls', '',
      ]) {
        expect(redacted(RuleType.command, command), '(command)', reason: command);
      }
      expect(redacted(RuleType.command, 'curl'), 'curl');
      expect(redacted(RuleType.command, 'x' * 100), '${'x' * 64}…');
    });

    test('a command that begins with a quoted assignment is logged as (command), whatever follows the space in its value', () async {
      final log = MemoryRuleLog();
      final rules = make(MemoryLedgerBackend(), log);
      const quoted = 'PASSWORD="hunter2 hunter3" ./deploy.sh';
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: quoted);
      await rules.settled();
      expect(log.lines.single, allOf(contains('"what":"(command)"'), isNot(contains('hunter')), isNot(contains('PASSWORD'))));
    });

    test('a path or a tool is logged as it is', () async {
      final log = MemoryRuleLog();
      final rules = make(MemoryLedgerBackend(), log);
      rules.add(source: _source, folder: _folder, kind: 'edit', type: RuleType.path, value: '/work/calc/src');
      await rules.settled();
      expect(log.lines.single, contains('"what":"/work/calc/src"'));
      expect(redacted(RuleType.command, 'ls'), 'ls');
    });

    test('keeps at most maxRules', () {
      final rules = make();
      for (var i = 0; i < PermissionRules.maxRules; i++) {
        expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'c$i'), isNotNull);
      }
      expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'one more'), isNull);
    });
  });
}

String? pathsProblem(RealPath? real) => folderProblem(_folder, real: real);

void fileBackendTests() {
  group('a linked worktree', () {
    test('counts as its main repository', () {
      final dir = Directory.systemTemp.createTempSync('canoa-repos-');
      addTearDown(() => dir.deleteSync(recursive: true));
      final main = Directory('${dir.path}/main/.git/worktrees/w')..createSync(recursive: true);
      final tree = Directory('${dir.path}/tree/sub')..createSync(recursive: true);
      File('${dir.path}/tree/.git').writeAsStringSync('gitdir: ${main.path}\n');
      final plain = Directory('${dir.path}/main/pkg')..createSync();
      final root = cleanPath('${dir.path}/main');
      expect(cleanPath(findRepoRoot(tree.path)!), root, reason: 'a worktree maps to the repository it was made from');
      expect(cleanPath(findRepoRoot(plain.path)!), root);
      File('${dir.path}/tree/.git').writeAsStringSync('gitdir: ../main/.git/worktrees/w\n');
      expect(cleanPath(findRepoRoot(tree.path)!), root, reason: 'a relative gitdir is read from the worktree');
      File('${dir.path}/tree/.git').writeAsStringSync('not a gitdir line');
      expect(cleanPath(findRepoRoot(tree.path)!), cleanPath('${dir.path}/tree'));
      expect(findRepoRoot('${dir.path}/nowhere'), anyOf(isNull, isNot(contains('nowhere'))));
    });
  });

  group('the rules file', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('canoa-rulesfile-'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('is replaced through a temp file named for that write alone, none left behind', () async {
      final backend = FileLedgerBackend('${dir.path}/permissions.json');
      for (var i = 0; i < 8; i++) {
        await backend.write('{"version":1,"rules":[],"n":$i}');
      }
      expect(await backend.read(), contains('"n":7'));
      final names = [for (final entity in dir.listSync()) entity.uri.pathSegments.where((part) => part.isNotEmpty).last];
      expect(names, ['permissions.json'], reason: 'no .new or .tmp file stays');
    });

    test('a rename another process\'s open file refuses is tried again, for a while and no longer', () async {
      var tries = 0;
      final backend = FileLedgerBackend('${dir.path}/permissions.json', renameFile: (from, to) async {
        tries++;
        if (tries < 4) throw FileSystemException('denied', to, const OSError('Access is denied.', 5));
        await from.rename(to);
      });
      await backend.write('{"version":1,"rules":[]}');
      expect(tries, 4);
      expect(await backend.read(), '{"version":1,"rules":[]}');
      final stuck = FileLedgerBackend('${dir.path}/stuck.json',
          retryFor: const Duration(milliseconds: 150),
          renameFile: (from, to) async => throw FileSystemException('denied', to, const OSError('Sharing violation', 32)));
      final started = DateTime.now();
      await expectLater(stuck.write('x'), throwsA(isA<FileSystemException>()));
      expect(DateTime.now().difference(started), lessThan(const Duration(seconds: 3)));
      expect(File('${dir.path}/stuck.json').existsSync(), isFalse);
      expect([for (final entity in dir.listSync()) entity.path.endsWith('.tmp')].where((tmp) => tmp), isEmpty, reason: 'no temp file stays');
      var other = 0;
      final broken = FileLedgerBackend('${dir.path}/other.json', renameFile: (from, to) async {
        other++;
        throw FileSystemException('no', to, const OSError('Not found', 2));
      });
      await expectLater(broken.write('x'), throwsA(isA<FileSystemException>()));
      expect(other, 1, reason: 'an error waiting cannot cure is not tried again');
    });

    test('a temp file a killed writer left is taken away by the next write once it is old, a fresh one is not', () async {
      final backend = FileLedgerBackend('${dir.path}/permissions.json');
      final old = File('${dir.path}/permissions.json.4242.abcd.tmp')..writeAsStringSync('half');
      old.setLastModifiedSync(DateTime.now().subtract(const Duration(minutes: 5)));
      final fresh = File('${dir.path}/permissions.json.4343.ef01.tmp')..writeAsStringSync('being written');
      final other = File('${dir.path}/notes.json.1.2.tmp')..writeAsStringSync('not ours');
      other.setLastModifiedSync(DateTime.now().subtract(const Duration(minutes: 5)));
      await backend.write('{}');
      expect(old.existsSync(), isFalse);
      expect(fresh.existsSync(), isTrue);
      expect(other.existsSync(), isTrue, reason: 'only this file\'s temps');
    });

    test('a lock that cannot be had in time fails the call and runs nothing', () async {
      final backend = FileLedgerBackend('${dir.path}/permissions.json');
      final hold = Completer<void>();
      final holder = backend.exclusively(() => hold.future);
      var ran = false;
      await expectLater(backend.exclusively(() async => ran = true, timeout: const Duration(milliseconds: 100)), throwsA(isA<TimeoutException>()));
      expect(ran, isFalse);
      hold.complete();
      await holder;
      await backend.exclusively(() async => ran = true);
      expect(ran, isTrue, reason: 'giving up did not leave the lock held');
    });

    test('is changed under a lock, one caller at a time', () async {
      final backend = FileLedgerBackend('${dir.path}/permissions.json');
      final order = <String>[];
      final one = backend.exclusively(() async {
        order.add('one in');
        await Future<void>.delayed(const Duration(milliseconds: 50));
        order.add('one out');
      });
      final two = backend.exclusively(() async {
        order.add('two in');
        order.add('two out');
      });
      await Future.wait([one, two]);
      expect(order, ['one in', 'one out', 'two in', 'two out']);
      expect(File('${dir.path}/permissions.json.lock').existsSync(), isTrue);
    });

    test('a list over it keeps a rule made by another list over the same file', () async {
      final path = '${dir.path}/permissions.json';
      final a = PermissionRules(FileLedgerBackend(path));
      a.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls');
      await a.settled();
      final b = PermissionRules(FileLedgerBackend(path));
      await b.load();
      b.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'pwd');
      await b.settled();
      await a.refresh();
      expect(a.rules.map((rule) => rule.value), unorderedEquals(['ls', 'pwd']));
    });
  });
}

/// A backend whose lock is never free.
final class _StuckLockBackend implements LedgerBackend, LockableBackend {
  @override
  String get location => 'a locked file';

  @override
  Future<String?> read() async => null;

  @override
  Future<void> write(String document) async {}

  @override
  Future<T> exclusively<T>(Future<T> Function() body, {Duration timeout = const Duration(seconds: 2)}) async {
    await Future<void>.delayed(timeout);
    throw TimeoutException('did not finish in ${timeout.inMilliseconds} ms', timeout);
  }
}

/// A backend whose writes fail while [fail] is set.
final class _FailingBackend implements LedgerBackend {
  bool fail = true;
  bool failReads = false;
  String? document;

  @override
  String get location => 'a file that cannot be written';

  @override
  Future<String?> read() async {
    if (failReads) throw const FileSystemException('unreadable');
    return document;
  }

  @override
  Future<void> write(String document) async {
    if (fail) throw const FileSystemException('disk full');
    this.document = document;
  }
}
