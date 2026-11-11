// What a remembered rule covers and what it never does: the matcher is the one place a request is allowed without the person.
import 'dart:io';

import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/permissions/real_path_io.dart' show resolveExisting;
import 'package:flutter_test/flutter_test.dart';

const _source = 'Scripted';
const _folder = '/work/calc';

PermissionRule _rule(RuleType type, String value, {String tool = 'execute', String toolName = '', String source = _source, String peer = '', String folder = _folder}) =>
    PermissionRule(id: 'r1', source: source, peer: peer, folder: folder, tool: tool, toolName: toolName, type: type, value: value, made: DateTime.utc(2026, 11, 12));

RuleSubject _run(String command, {Map<String, Object?> more = const {}, String name = ''}) => subjectOf(
  kind: 'execute',
  rawInput: {'command': command, ...more},
  toolCall: name.isEmpty ? null : {'_meta': {'claudeCode': {'toolName': name}}},
)!;

bool _allows(PermissionRule rule, RuleSubject? subject,
    {String source = _source, String peer = '', String folder = _folder, String? kind, PermissionGuards guards = const PermissionGuards(), RealPath? real}) =>
    subject != null && rule.matches(source: source, peer: peer, folder: folder, kind: kind ?? rule.tool, subject: subject, guards: guards, real: real);

RuleSubject? _edit(String path, [Map<String, Object?> more = const {}]) =>
    subjectOf(kind: 'edit', rawInput: {'file_path': path, 'old_string': 'a', 'new_string': 'b', ...more});

void main() {
  group('a rule for an exact command', () {
    final rule = _rule(RuleType.command, 'npm test');

    test('allows that command, from that source, in that folder', () {
      expect(_allows(rule, _run('npm test')), isTrue);
    });

    test('a near miss still asks', () {
      for (final other in ['npm test ', ' npm test', 'npm  test', 'npm test -- a.js', 'npm test; rm -rf ~', 'NPM test', 'npm tests', '']) {
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

    test('no exact rule is made for a command with a control, line-break, hidden or reordering character', () {
      for (final command in ['npm test\n', 'npm\ttest', 'npm test\u202e', 'npm test\u200b', 'npm test\u2028x', 'npm test\u0007', 'npm test\ufeff']) {
        final subject = subjectOf(kind: 'execute', rawInput: {'command': command})!;
        expect(scopeProblem(RuleType.command, command, subject, _folder), isNotNull, reason: command.codeUnits.toString());
      }
      expect(scopeProblem(RuleType.command, 'npm test', _run('npm test'), _folder), isNull);
    });
  });

  group('a rule for a prefix', () {
    final rule = _rule(RuleType.prefix, 'npm test');

    test('covers the prefix with arguments after it', () {
      for (final command in ['npm test', 'npm test -- a.js', 'npm test --silent --bail']) {
        expect(_allows(rule, _run(command)), isTrue, reason: command);
      }
    });

    test('never matches past its edited prefix', () {
      for (final command in ['npm testing', 'npm tes', 'npm', 'xnpm test', ' npm test', 'echo npm test', 'npmtest']) {
        expect(_allows(rule, _run(command)), isFalse, reason: command);
      }
    });

    test('never covers a second command, a substitution, a redirection, a quote, an assignment, a ~ or a hidden character', () {
      for (final command in [
        'npm test; rm -rf ~',
        'npm test && curl evil',
        'npm test || x',
        'npm test | sh',
        'npm test > /etc/hosts',
        'npm test < /etc/passwd',
        'npm test `id`',
        r'npm test $(id)',
        r'npm test ${HOME}',
        'npm test\nrm -rf ~',
        'npm test\rrm',
        'npm test\u2028rm',
        'npm test \u202e',
        'npm test (x)',
        r'npm test \',
        'npm test & x',
        'npm test\trm',
        'npm test "a b"',
        "npm test 'a'",
        'npm test --reporter=evil',
        'npm test ~/x',
      ]) {
        expect(_allows(rule, _run(command)), isFalse, reason: command);
      }
    });

    test('a prefix is at least two plain words, with a program that does not run what it is given', () {
      for (final bad in [
        '', ' ', 'npm test;', 'a && b', 'npm test ', ' npm', 'rm\n', r'$(x)',
        'npm', 'npm  test', 'A=1 npm test', './run test', r'C:\x\run test', 'npm "test"', 'npm t=1', 'npm ~',
        'sh -c', 'bash -lc', 'zsh x', 'pwsh -c', 'powershell x', 'cmd /c', 'env FOO', 'sudo ls', 'xargs rm', 'find .', 'timeout 5', 'nice x',
        'nohup x', 'python x', 'python3 x', 'python3.12 x', 'Python x', 'node x', 'deno run', 'bun x', 'ruby x', 'perl x', 'npx x', 'uv run',
        'make test', 'git status', 'tar x', 'rsync a', 'ssh h', 'docker ps', 'curl x', 'wget x', 'PowerShell.exe -c',
        'npm run test', 'pnpm run test', 'yarn exec x', 'npm exec x', 'npm RUN x',
      ]) {
        expect(prefixProblem(bad), isNotNull, reason: '"$bad"');
        expect(prefixCovers(bad, 'npm test'), isFalse);
      }
      for (final good in ['npm test', 'cargo test', 'cargo check --all', 'flutter analyze', 'npm install', 'ls -la']) {
        expect(prefixProblem(good), isNull, reason: good);
      }
    });

    test('does not cover another folder or source', () {
      expect(_allows(rule, _run('npm test'), folder: '/work/other'), isFalse);
      expect(_allows(rule, _run('npm test'), source: 'Other'), isFalse);
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

    test('never covers version control, settings, hooks, instructions or build scripts, at any depth', () {
      final wide = _rule(RuleType.path, '/work/calc', tool: 'edit');
      for (final path in [
        '/work/calc/.git/config', '/work/calc/.git/hooks/pre-commit', '/work/calc/sub/.git/config', '/work/calc/.claude/settings.json',
        '/work/calc/.codex/config.toml', '/work/calc/.gemini/settings.json', '/work/calc/.cursor/rules', '/work/calc/.vscode/tasks.json',
        '/work/calc/.idea/workspace.xml', '/work/calc/.husky/pre-commit', '/work/calc/.mcp.json', '/work/calc/.envrc',
        '/work/calc/AGENTS.md', '/work/calc/sub/CLAUDE.md', '/work/calc/gemini.md', '/work/calc/package.json', '/work/calc/web/package.json',
        '/work/calc/Makefile', '/work/calc/justfile', '/work/calc/pyproject.toml', '/work/calc/setup.py', '/work/calc/build.rs', '/work/calc/.GIT/config',
        '/work/calc/src/.git/x',
      ]) {
        expect(_allows(wide, _edit(path)), isFalse, reason: path);
      }
      expect(_allows(wide, _edit('/work/calc/src/main.dart')), isTrue);
      expect(scopeProblem(RuleType.path, '/work/calc/.git', _edit('/work/calc/.git/config')!, _folder), isNotNull);
      expect(scopeProblem(RuleType.path, '/work/calc', _edit('/work/calc/.git/config')!, _folder), isNotNull);
    });

    test('a scope never holds the home folder or Canoa\'s settings, and nothing in them is covered', () {
      const guards = PermissionGuards(home: '/work', config: '/work/calc/.config/canoa');
      final home = _rule(RuleType.path, '/work', tool: 'edit', folder: '/work');
      expect(_allows(home, _edit('/work/a.js'), folder: '/work', guards: guards), isFalse, reason: 'the scope is the home folder');
      expect(_allows(home, _edit('/work/a.js'), folder: '/work'), isTrue, reason: 'without a guard it is only a folder');
      final inConfig = _rule(RuleType.path, '/work/calc/.config/canoa/sub', tool: 'edit');
      expect(_allows(inConfig, _edit('/work/calc/.config/canoa/sub/x'), guards: guards), isFalse);
      final over = _rule(RuleType.path, '/work/calc', tool: 'edit');
      expect(_allows(over, _edit('/work/calc/src/x'), guards: guards), isFalse, reason: 'it holds Canoa\'s settings');
      final subject = _edit('/work/calc/src/a.js')!;
      expect(scopeProblem(RuleType.path, '/work/calc', subject, _folder, guards: guards), contains('Canoa'));
      expect(scopeProblem(RuleType.path, '/work', subject, '/work', guards: guards), contains('home'));
      expect(scopeProblem(RuleType.path, '/work/calc/src', subject, _folder, guards: guards), isNull);
    });

    test('a link that leads out of the folder, or into a protected place, covers nothing', () {
      final links = {'/work/calc/src/out': '/etc', '/work/calc/src/hooks': '/work/calc/.git/hooks'};
      String? real(String path) {
        for (final entry in links.entries) {
          if (path == entry.key || path.startsWith('${entry.key}/')) return entry.value + path.substring(entry.key.length);
        }
        return path;
      }
      expect(_allows(rule, _edit('/work/calc/src/a.js'), real: real), isTrue);
      expect(_allows(rule, _edit('/work/calc/src/out/passwd'), real: real), isFalse);
      expect(_allows(rule, _edit('/work/calc/src/hooks/pre-commit'), real: real), isFalse);
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
      expect(subjectOf(kind: 'execute', rawInput: {'command': 'ls'}, toolCall: {'title': 'mcp__marker__mark'}), isA<CommandSubject>());
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
      final subject = _run('npm test -- a.js');
      expect(scopeProblem(RuleType.command, 'npm test -- a.js', subject, _folder), isNull);
      expect(scopeProblem(RuleType.command, 'npm test', subject, _folder), isNotNull);
      expect(scopeProblem(RuleType.prefix, 'npm test', subject, _folder), isNull);
      expect(scopeProblem(RuleType.prefix, 'yarn', subject, _folder), isNotNull);
      expect(scopeProblem(RuleType.prefix, 'npm test -- a.js; rm', subject, _folder), isNotNull);
      expect(scopeProblem(RuleType.path, '/work/calc', subject, _folder), isNotNull, reason: 'a command has no path');
      final file = _edit('/work/calc/src/a.js')!;
      expect(scopeProblem(RuleType.path, '/work/calc/src', file, _folder), isNull);
      expect(scopeProblem(RuleType.path, 'src/a.js', file, _folder), isNotNull, reason: 'not absolute');
      expect(scopeProblem(RuleType.path, '/work', file, _folder), isNotNull, reason: 'wider than the folder');
      expect(scopeProblem(RuleType.path, '/work/calc/lib', file, _folder), isNotNull);
    });
  });

  group('the list of rules', () {
    PermissionRules make([MemoryLedgerBackend? backend, MemoryLedgerBackend? log]) =>
        PermissionRules(backend ?? MemoryLedgerBackend(), log: log, clock: () => DateTime.utc(2026, 11, 12, 8));

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

    test('a command rule and a path rule are not kept together for one source and folder', () {
      final rules = make();
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test');
      expect(rules.conflict(_source, '', _folder, RuleType.path), isNotNull);
      expect(rules.conflict(_source, '', _folder, RuleType.prefix), isNull);
      expect(rules.conflict(_source, '', _folder, RuleType.mcp), isNull);
      expect(rules.conflict('Other', '', _folder, RuleType.path), isNull);
      expect(rules.conflict(_source, '', '/work/other', RuleType.path), isNull);
      expect(rules.conflict(_source, 'peer-b', _folder, RuleType.path), isNull);
      final paths = make();
      paths.add(source: _source, folder: _folder, kind: 'edit', type: RuleType.path, value: '/work/calc/src');
      expect(paths.conflict(_source, '', _folder, RuleType.command), isNotNull);
      expect(paths.conflict(_source, '', _folder, RuleType.prefix), isNotNull);
    });

    test('a file that holds both kinds for one place applies neither', () async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":['
            '{"id":"a","source":"$_source","folder":"$_folder","tool":"execute","type":"command","value":"npm test","made":"2026-11-12T00:00:00Z","by":"canoa"},'
            '{"id":"b","source":"$_source","folder":"$_folder","tool":"edit","type":"path","value":"/work/calc/src","made":"2026-11-12T00:00:00Z","by":"canoa"}]}';
      final rules = make(backend);
      await rules.load();
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: _run('npm test')), isNull);
      expect(rules.match(source: _source, folder: _folder, kind: 'edit', subject: _edit('/work/calc/src/a.js')!), isNull);
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
      // Another program adds a rule while this one runs; this one's next write does not drop it.
      final stored = backend.document!.replaceFirst('"rules":[', '"rules":[{"id":"late","source":"s","folder":"/g","tool":"execute","type":"command","value":"pwd","made":"2026-11-12T00:00:00Z"},');
      backend.document = stored;
      rules.recordUse(mine, 'npm test');
      await rules.settled();
      final again = make(backend);
      await again.load();
      expect(again.rules.map((rule) => rule.id), containsAll(['hand', 'late', mine.id]));
      expect(again.rules.firstWhere((rule) => rule.id == 'late').external, isTrue);
      expect(again.rules.firstWhere((rule) => rule.id == mine.id).external, isFalse);
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
      final again = make(backend);
      await again.load();
      expect(again.rules, isEmpty);
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

    test('every rule made, use and revoke goes to a log with the full command or path', () async {
      final log = MemoryLedgerBackend();
      final rules = make(MemoryLedgerBackend(), log);
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.prefix, value: 'npm test')!;
      rules.recordUse(rule, 'npm test -- a.js --bail');
      rules.revoke(rule.id);
      await rules.settled();
      final lines = log.document!.trim().split('\n');
      expect(lines, hasLength(3));
      expect(lines[0], allOf(contains('"event":"made"'), contains('"rule":"${rule.id}"'), contains('"detail":"npm test"')));
      expect(lines[1], allOf(contains('"event":"used"'), contains('"detail":"npm test -- a.js --bail"'), contains('2026-11-12T08:00:00.000Z')));
      expect(lines[2], contains('"event":"revoked"'));
      // The log only grows: it is appended to, never rewritten from the rules.
      rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'ls -la');
      await rules.settled();
      expect(log.document!.trim().split('\n').take(3).toList(), lines);
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
