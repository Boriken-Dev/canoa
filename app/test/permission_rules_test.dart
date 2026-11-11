// What a remembered rule covers and what it never does: the matcher is the one place a request is allowed without the person.
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:flutter_test/flutter_test.dart';

const _source = 'Scripted';
const _folder = '/work/calc';

PermissionRule _rule(RuleType type, String value, {String tool = 'execute', String source = _source, String folder = _folder}) =>
    PermissionRule(id: 'r1', source: source, folder: folder, tool: tool, type: type, value: value, made: DateTime.utc(2026, 11, 12));

RuleSubject _run(String command, [Map<String, Object?> more = const {}]) =>
    subjectOf(kind: 'execute', rawInput: {'command': command, ...more})!;

bool _allows(PermissionRule rule, RuleSubject? subject, {String source = _source, String folder = _folder, String? kind}) =>
    subject != null && rule.matches(source: source, folder: folder, kind: kind ?? rule.tool, subject: subject);

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

    test('another folder, another source and another kind of tool still ask', () {
      expect(_allows(rule, _run('npm test'), folder: '/work/other'), isFalse);
      expect(_allows(rule, _run('npm test'), folder: '/work/calc/sub'), isFalse, reason: 'here is this folder, not what is under it');
      expect(_allows(rule, _run('npm test'), source: 'Other'), isFalse);
      expect(_allows(rule, _run('npm test'), kind: 'edit'), isFalse);
      expect(rule.matches(source: _source, folder: _folder, kind: null, subject: _run('npm test')), isFalse);
    });

    test('input that says more than a command is never covered', () {
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test', 'run_in_background': true})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test', 'cwd': '/'})), isFalse);
      expect(_allows(rule, _run('npm test', {'description': 'Run the tests', 'timeout': 5000})), isTrue,
          reason: 'a description and a timeout change nothing about what runs');
    });

    test('a command written another way is not the same command', () {
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm\u00a0test'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'execute', rawInput: {'command': 'npm test\n'})), isFalse);
    });
  });

  group('a rule for a prefix', () {
    final rule = _rule(RuleType.prefix, 'npm test');

    test('covers the prefix with arguments after it', () {
      for (final command in ['npm test', 'npm test -- a.js', 'npm test --silent "a b"']) {
        expect(_allows(rule, _run(command)), isTrue, reason: command);
      }
    });

    test('never matches past its edited prefix', () {
      for (final command in ['npm testing', 'npm tes', 'npm', 'xnpm test', ' npm test', 'echo npm test', 'npmtest']) {
        expect(_allows(rule, _run(command)), isFalse, reason: command);
      }
    });

    test('never covers a second command, a substitution, a redirection or a hidden character', () {
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
      ]) {
        expect(_allows(rule, _run(command)), isFalse, reason: command);
      }
    });

    test('a prefix that is itself a compound command or empty is not a prefix', () {
      for (final bad in ['', ' ', 'npm test;', 'a && b', 'npm test ', ' npm', 'rm\n', r'$(x)']) {
        expect(prefixProblem(bad), isNotNull, reason: '"$bad"');
        expect(prefixCovers(bad, 'npm test'), isFalse);
      }
      expect(prefixProblem('git status'), isNull);
    });

    test('does not cover another folder or source', () {
      expect(_allows(rule, _run('npm test'), folder: '/work/other'), isFalse);
      expect(_allows(rule, _run('npm test'), source: 'Other'), isFalse);
    });
  });

  group('a rule for changes under a path', () {
    final rule = _rule(RuleType.path, '/work/calc/src', tool: 'edit');
    RuleSubject? edit(String path, [Map<String, Object?> more = const {}]) =>
        subjectOf(kind: 'edit', rawInput: {'file_path': path, 'old_string': 'a', 'new_string': 'b', ...more});

    test('covers the file and what is under it, and a relative path is read from the folder', () {
      expect(_allows(rule, edit('/work/calc/src')), isTrue);
      expect(_allows(rule, edit('/work/calc/src/a.js')), isTrue);
      expect(_allows(rule, edit('/work/calc/src/deep/er/a.js')), isTrue);
      expect(_allows(rule, edit('src/a.js')), isTrue);
      expect(_allows(rule, edit('./src/../src/a.js')), isTrue);
    });

    test('never covers outside it, by a sibling name or by climbing out', () {
      for (final path in ['/work/calc/srcx/a.js', '/work/calc/a.js', '/work/calc/src/../a.js', '../src/a.js', '/etc/passwd',
        '/work/calc/src/../../other/a.js', 'src/../../a.js', '', '/work/calc/src\u0000/../../x']) {
        expect(_allows(rule, edit(path)), isFalse, reason: path);
      }
    });

    test('never covers a change that names a second place or anything it does not know', () {
      expect(_allows(rule, edit('/work/calc/src/a.js', {'new_path': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js', 'path': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js', 'destination': '/etc/x'})), isFalse);
      expect(_allows(rule, subjectOf(kind: 'edit', rawInput: {'paths': ['/work/calc/src/a.js']})), isFalse);
    });

    test('does not cover another folder, source or kind of tool', () {
      expect(_allows(rule, edit('/work/calc/src/a.js'), folder: '/work/other'), isFalse);
      expect(_allows(rule, edit('/work/calc/src/a.js'), source: 'Other'), isFalse);
      expect(_allows(rule, edit('/work/calc/src/a.js'), kind: 'delete'), isFalse);
    });

    test('a rule whose path lies outside its folder covers nothing', () {
      final stray = _rule(RuleType.path, '/etc', tool: 'edit');
      expect(_allows(stray, edit('/etc/passwd')), isFalse);
      final climbing = _rule(RuleType.path, '../other', tool: 'edit');
      expect(_allows(climbing, edit('/work/other/a.js')), isFalse);
    });

    test('a Windows path has no case and either slash', () {
      final win = _rule(RuleType.path, r'C:\Work\Calc\src', tool: 'edit', folder: r'C:\Work\Calc');
      expect(_allows(win, edit(r'c:/work/calc/SRC/a.js'), folder: r'c:\work\calc'), isTrue);
      expect(_allows(win, edit(r'C:\Work\Other\a.js'), folder: r'C:\Work\Calc'), isFalse);
      expect(_allows(win, edit(r'D:\Work\Calc\src\a.js'), folder: r'C:\Work\Calc'), isFalse);
    });
  });

  group('a rule for an MCP tool', () {
    final rule = _rule(RuleType.mcp, 'mcp__marker__mark', tool: 'other');

    test('allows that tool on that server with any input, and no other', () {
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mcp__marker__mark', rawInput: {'any': 1})), isTrue);
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mcp__marker__mark2')), isFalse);
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mcp__other__mark')), isFalse);
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mcp__marker__mark', toolCall: {'_meta': {'claudeCode': {'toolName': 'mcp__marker__erase'}}})), isFalse,
          reason: 'the tool the agent names beats the title it shows');
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mark')), isFalse);
      expect(_allows(rule, subjectOf(kind: 'other', title: 'mcp__marker__mark'), source: 'Other'), isFalse);
    });

    test('is described by tool and server', () {
      expect(rule.describe, 'Use the tool mark of marker, with any input');
    });
  });

  group('what a rule can be made of', () {
    test('a tool of another kind, or with no command, has no subject', () {
      expect(subjectOf(kind: 'read', rawInput: {'file_path': 'a'}), isNull);
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
      final file = subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/src/a.js'})!;
      expect(scopeProblem(RuleType.path, '/work/calc/src', file, _folder), isNull);
      expect(scopeProblem(RuleType.path, 'src/a.js', file, _folder), isNull);
      expect(scopeProblem(RuleType.path, '/work', file, _folder), isNotNull, reason: 'wider than the folder');
      expect(scopeProblem(RuleType.path, '/work/calc/lib', file, _folder), isNotNull);
    });
  });

  group('the list of rules', () {
    PermissionRules make([MemoryLedgerBackend? backend]) => PermissionRules(backend ?? MemoryLedgerBackend(), clock: () => DateTime.utc(2026, 11, 12, 8));

    test('keeps what is made, counts uses, and revoking stops one matching', () async {
      final backend = MemoryLedgerBackend();
      final rules = make(backend);
      final rule = rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test')!;
      expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'npm test'), same(rule),
          reason: 'the same rule is not made twice');
      final subject = _run('npm test');
      expect(rules.match(source: _source, folder: _folder, kind: 'execute', subject: subject), same(rule));
      rules.recordUse(rule);
      rules.recordUse(rule);
      await rules.flush();
      final reread = make(backend);
      await reread.load();
      expect(reread.rules.single.fired, 2);
      expect(reread.rules.single.value, 'npm test');
      expect(reread.rules.single.made, DateTime.utc(2026, 11, 12, 8));
      expect(reread.revoke(rule.id), isTrue);
      expect(reread.revoke(rule.id), isFalse);
      expect(reread.match(source: _source, folder: _folder, kind: 'execute', subject: subject), isNull);
      await reread.flush();
      final again = make(backend);
      await again.load();
      expect(again.rules, isEmpty, reason: 'the revoke was written');
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

    test('keeps at most maxRules', () {
      final rules = make();
      for (var i = 0; i < PermissionRules.maxRules; i++) {
        expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'c$i'), isNotNull);
      }
      expect(rules.add(source: _source, folder: _folder, kind: 'execute', type: RuleType.command, value: 'one more'), isNull);
    });
  });
}
