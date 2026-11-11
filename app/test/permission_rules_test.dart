// What a remembered rule covers and what it never does: the matcher is the one place a request is allowed without the person.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/permissions/real_path_io.dart' show resolveExisting;
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

    test('no rule is made for a command with a control, line-break, hidden or reordering character', () {
      for (final command in ['npm test\n', 'npm\ttest', 'npm test\u202e', 'npm test\u200b', 'npm test\u2028x', 'npm test\u0007', 'npm test\ufeff']) {
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
      expect(_allows(rule, _edit('/work/calc/./src/../src/a.js')), isTrue);
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
      expect(_allows(wide, _edit('/work/calc/a.config.json')), isTrue);
      final hidden = _rule(RuleType.path, '/work/.agents/calc', tool: 'edit', folder: '/work/.agents/calc');
      expect(_allows(hidden, _edit('/work/.agents/calc/a.js'), folder: '/work/.agents/calc'), isTrue);
      expect(_allows(hidden, _edit('/work/.agents/calc/.x/a.js'), folder: '/work/.agents/calc'), isFalse);
    });

    test('a scope that is one of them is refused with the reason', () {
      expect(scopeProblem(RuleType.path, '/work/calc/.git', _edit('/work/calc/.git/config')!, _folder, real: _plain), contains('Hidden'));
      expect(scopeProblem(RuleType.path, '/work/calc', _edit('/work/calc/.claude./settings.json')!, _folder, real: _plain), contains('Hidden'));
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
    final rule = _rule(RuleType.mcp, 'mcp__marker__mark', tool: 'other', toolName: 'mcp__marker__mark');
    RuleSubject? mcp(String name, {String kind = 'other'}) =>
        subjectOf(kind: kind, rawInput: {'x': 1}, toolCall: {'_meta': {'claudeCode': {'toolName': name}}});

    test('allows that tool on that server with any input, and no other', () {
      expect(_allows(rule, mcp('mcp__marker__mark')), isTrue);
      expect(_allows(rule, mcp('mcp__marker__mark2')), isFalse);
      expect(_allows(rule, mcp('mcp__other__mark')), isFalse);
      expect(_allows(rule, mcp('mcp__marker__mark'), source: 'Other'), isFalse);
    });

    test('is made only from the structured tool name, never from a title', () {
      expect(subjectOf(kind: 'other', rawInput: {'x': 1}), isNull);
      expect(subjectOf(kind: 'other', rawInput: {'x': 1}, toolCall: {'title': 'mcp__marker__mark'}), isNull);
      expect(mcp('mcp__a__b__c'), isA<McpSubject>().having((s) => s.server, 'server', 'a'));
      expect(mcp('mcp____x'), isNull, reason: 'a server named by nothing');
      expect(mcp('mark'), isNull);
    });

    test('is described by tool and server', () {
      expect(rule.describe, 'Use the tool mark of marker, with any input');
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
      expect(rules.conflict(_source, '', _folder, RuleType.path), isNull, reason: 'an inactive rule holds nothing back');
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

    test('commands and the rest are not kept together for one source: file changes and tools count, in nested folders too', () {
      final rules = make();
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      for (final type in [RuleType.path, RuleType.mcp]) {
        expect(rules.conflict(_source, '', _folder, type), isNotNull, reason: type.name);
        expect(rules.conflict(_source, '', '/work/calc/sub', type), isNotNull, reason: 'a folder inside it');
        expect(rules.conflict(_source, '', '/work', type), isNotNull, reason: 'a folder that holds it');
        expect(rules.conflict(_source, '', '/work/other', type), isNull);
        expect(rules.conflict('Other', '', _folder, type), isNull);
        expect(rules.conflict(_source, 'peer-b', _folder, type), isNull);
      }
      expect(rules.conflict(_source, '', '/work/calc/sub', RuleType.command), isNull);
      final others = make();
      others.add(source: _source, folder: _folder, kind: 'other', toolName: 'mcp__a__b', type: RuleType.mcp, value: 'mcp__a__b');
      expect(others.conflict(_source, '', _folder, RuleType.command), isNotNull, reason: 'a file system tool can write what a command runs');
      expect(others.conflict(_source, '', '/work/calc/sub', RuleType.command), isNotNull);
      final paths = make();
      paths.add(source: _source, folder: _folder, kind: 'edit', type: RuleType.path, value: '/work/calc/src');
      expect(paths.conflict(_source, '', '/work', RuleType.command), isNotNull);
      expect(paths.conflict(_source, '', _folder, RuleType.mcp), isNull);
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

    test('the log is appended to, and holds a hash and a redacted form of a command, never the command', () async {
      final log = MemoryRuleLog();
      final rules = make(MemoryLedgerBackend(), log);
      const secret = 'curl -H "Authorization: Bearer sk-live-123456" https://api.example.test/x';
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: secret)!;
      rules.recordUse(rule, secret);
      rules.revoke(rule.id);
      await rules.settled();
      expect(log.lines, hasLength(3));
      for (final line in log.lines) {
        expect(line, isNot(contains('sk-live')));
        expect(line, isNot(contains('Authorization')));
        expect(line, contains(sha256Of(secret)));
        expect(line, contains('"what":"curl … (5 arguments)"'));
      }
      expect(log.lines[0], allOf(contains('"event":"made"'), contains('"rule":"${rule.id}"')));
      expect(log.lines[1], allOf(contains('"event":"used"'), contains('2026-11-12T08:00:00.000Z')));
      expect(log.lines[2], contains('"event":"revoked"'));
      final before = List.of(log.lines);
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls -la');
      await rules.settled();
      expect(log.lines.take(3).toList(), before, reason: 'a line once written is not touched');
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

/// A backend whose writes fail while [fail] is set.
final class _FailingBackend implements LedgerBackend {
  bool fail = true;
  String? document;

  @override
  String get location => 'a file that cannot be written';

  @override
  Future<String?> read() async => document;

  @override
  Future<void> write(String document) async {
    if (fail) throw const FileSystemException('disk full');
    this.document = document;
  }
}
