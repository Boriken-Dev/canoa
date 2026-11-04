// A git worktree per session: the branch a session gets, git on this computer against repositories in a temporary directory,
// the host's answers, the ledger's note of it, the list showing the branch, and what closing a session asks. A widget test
// cannot wait on git processes, so git runs in plain tests and the widgets are given its state.
import 'dart:io';
import 'dart:math';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/ledger/ledger_list.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/worktree/close_dialog.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/local_io.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// `git -C dir args`, which must succeed.
Future<String> _git(String dir, List<String> args) async {
  final result = await Process.run('git', [
    '-C', dir, '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid', '-c', 'commit.gpgsign=false', ...args,
  ]);
  expect(result.exitCode, 0, reason: 'git $args: ${result.stderr}');
  return '${result.stdout}';
}

/// A repository at [dir] with one commit.
Future<void> _repository(String dir) async {
  await Directory('$dir/sub').create(recursive: true);
  await _git(dir, ['init', '-q']);
  await File('$dir/README').writeAsString('hello\n');
  await File('$dir/sub/keep').writeAsString('');
  await _git(dir, ['add', '.']);
  await _git(dir, ['commit', '-q', '-m', 'first']);
}

Future<List<String>> _branches(String repo) async =>
    (await _git(repo, ['branch', '--format=%(refname:short)'])).split('\n').where((line) => line.isNotEmpty).toList();

final bool _hasGit = () {
  try {
    return Process.runSync('git', ['--version']).exitCode == 0;
  } on ProcessException {
    return false;
  }
}();

void main() {
  group('a branch', () {
    test('is derived from the session and always one Canoa takes', () {
      final random = Random(1);
      for (final label in ['Fix the tests', '', '--force', '../../etc', 'a..b', 'é ü', 'x' * 200, 'Run\x07\nnow', '@{1}', '-']) {
        final branch = worktreeBranch(label, random: random);
        expect(safeWorktreeBranch(branch), isTrue, reason: '$label gave $branch');
        expect(branch, isNot(contains('..')));
        expect(branch.length, lessThanOrEqualTo(maxWorktreeBranch));
      }
      expect(worktreeBranch('Fix the tests', random: Random(1)), startsWith('canoa/fix-the-tests-'));
      expect(worktreeBranch('', random: Random(1)), startsWith('canoa/session-'));
      expect(worktreeBranch('Same', random: Random(1)), isNot(worktreeBranch('Same', random: Random(2))));
    });

    test('anything else is refused, here and in what the ledger or a host gives back', () {
      for (final bad in ['', 'main', '-b', '--orphan', 'canoa/', 'canoa/-x', 'canoa/x-', 'canoa/a--b', 'canoa/a..b',
          'canoa/a/b', 'canoa/A', 'canoa/a b', 'canoa/a\x07', 'canoa/a.lock', 'canoa/${'a' * 70}']) {
        expect(safeWorktreeBranch(bad), isFalse, reason: bad);
        expect(SessionWorktree.fromJson({'path': '/w', 'branch': bad, 'repo': '/r'}), isNull, reason: bad);
      }
    });

    test('names where its worktree goes, beside the repository', () {
      expect(worktreeLocation('/src/proj', 'canoa/x-1'), '/src/proj.worktrees/x-1');
      expect(worktreeLocation(r'C:\src\proj\', 'canoa/x-1'), 'C:/src/proj.worktrees/x-1');
    });
  });

  group('git on this computer', () {
    late Directory scratch;
    late String repo;
    final worktrees = LocalWorktrees();

    setUp(() async {
      scratch = await Directory.systemTemp.createTemp('canoa-worktree-test');
      repo = '${scratch.path}/src/proj';
      if (_hasGit) await _repository(repo);
    });
    tearDown(() => scratch.delete(recursive: true));

    final skip = _hasGit ? null : 'no git on PATH';

    test('two sessions on one repository each work in a worktree of their own', () async {
      final first = await worktrees.create(repo, 'canoa/first-a1b2c3');
      final second = await worktrees.create('$repo/sub', 'canoa/second-d4e5f6');
      expect(first.path, isNot(second.path));
      expect(first.path.replaceAll(r'\', '/'), endsWith('/src/proj.worktrees/first-a1b2c3'));
      expect(second.path.replaceAll(r'\', '/'), endsWith('/src/proj.worktrees/second-d4e5f6'));
      expect(File('${first.path}/README').existsSync() && File('${second.path}/README').existsSync(), isTrue);
      expect(await _branches(repo), containsAll(['canoa/first-a1b2c3', 'canoa/second-d4e5f6']));
      expect('worktree '.allMatches(await _git(repo, ['worktree', 'list', '--porcelain'])).length, 3);

      // Work in each stays apart.
      await File('${first.path}/draft').writeAsString('unsaved\n');
      await File('${second.path}/done').writeAsString('saved\n');
      await _git(second.path, ['add', 'done']);
      await _git(second.path, ['commit', '-q', '-m', 'work']);
      final firstState = await worktrees.status(first), secondState = await worktrees.status(second);
      expect((firstState.exists, firstState.branch, firstState.changed, firstState.commits), (true, first.branch, 1, 0));
      expect((secondState.exists, secondState.branch, secondState.changed, secondState.commits), (true, second.branch, 0, 1));
      expect(File('${second.path}/draft').existsSync(), isFalse);

      // A change is not deleted without discard; a commit of its own keeps the branch.
      await expectLater(worktrees.remove(first), throwsA(isA<WorktreeFailure>()));
      expect(File('${first.path}/draft').existsSync(), isTrue);
      final discarded = await worktrees.remove(first, discard: true);
      expect((discarded.branch, discarded.branchKept), (first.branch, false));
      expect(Directory(first.path).existsSync(), isFalse);
      final removed = await worktrees.remove(second);
      expect((removed.branchKept, removed.commits), (true, 1));
      expect(await _branches(repo), isNot(contains(first.branch)));
      expect(await _branches(repo), contains(second.branch));

      // A reopened session finds its worktree again, or finds it gone.
      expect((await worktrees.status(first)).exists, isFalse);
    }, skip: skip);

    test('what is not Canoa\'s is neither made over nor removed', () async {
      await expectLater(worktrees.create(repo, '--orphan'), throwsA(isA<WorktreeFailure>()));
      await expectLater(worktrees.create('${scratch.path}/none', 'canoa/x'), throwsA(isA<WorktreeFailure>()));
      await Directory('${scratch.path}/plain').create();
      await expectLater(worktrees.create('${scratch.path}/plain', 'canoa/x'), throwsA(isA<WorktreeFailure>()));
      await _git(repo, ['worktree', 'add', '-q', '-b', 'mine', '../mine']);
      await _git(repo, ['worktree', 'add', '-q', '-b', 'canoa/hand-made', '../hand-made']);
      for (final (path, branch) in [(repo, 'canoa/x'), ('${scratch.path}/src/mine', 'canoa/mine'),
          ('${scratch.path}/src/hand-made', 'canoa/hand-made')]) {
        final worktree = SessionWorktree(path: path, branch: branch, repo: repo);
        expect((await worktrees.status(worktree)).exists, isFalse, reason: path);
        await expectLater(worktrees.remove(worktree, discard: true), throwsA(isA<WorktreeFailure>()), reason: path);
        expect(Directory(path).existsSync(), isTrue, reason: path);
      }
    }, skip: skip);

    test('a .worktrees directory that is a link is not used', () async {
      final elsewhere = await Directory('${scratch.path}/elsewhere').create();
      final link = '${scratch.path}/src/proj.worktrees';
      if (Platform.isWindows) {
        final made = await Process.run('cmd', ['/C', 'mklink', '/J', link.replaceAll('/', r'\'), elsewhere.path.replaceAll('/', r'\')]);
        expect(made.exitCode, 0, reason: '${made.stderr}');
      } else {
        await Link(link).create(elsewhere.path);
      }
      await expectLater(worktrees.create(repo, 'canoa/x'), throwsA(isA<WorktreeFailure>()));
      expect(elsewhere.listSync(), isEmpty);
      expect(await _branches(repo), isNot(contains('canoa/x')));
      if (Platform.isWindows) await Directory(link).delete();
    }, skip: skip);
  });

  group('the host', () {
    test('answers are read as the host gave them, and an answer for another branch is refused', () async {
      final calls = <(String, Map<String, Object?>)>[];
      Map<String, Object?> answer = {};
      final host = HostWorktrees((method, params) async {
        calls.add((method, params));
        return answer;
      });
      answer = {'path': '/src/proj.worktrees/x-1', 'branch': 'canoa/x-1', 'repo': '/src/proj'};
      final made = await host.create('/src/proj', 'canoa/x-1');
      expect((made.path, made.branch, made.repo), ('/src/proj.worktrees/x-1', 'canoa/x-1', '/src/proj'));
      expect(calls.last.$1, '_canoa/worktree/new');
      expect(calls.last.$2, {'cwd': '/src/proj', 'branch': 'canoa/x-1'});
      await expectLater(host.create('/src/proj', 'canoa/y-1'), throwsA(isA<WorktreeFailure>()));
      answer = {'exists': true, 'branch': 'canoa/x-1', 'changed': 2, 'commits': 1};
      final state = await host.status(made);
      expect((state.exists, state.changed, state.commits), (true, 2, 1));
      expect(calls.last.$1, '_canoa/worktree/status');
      expect(calls.last.$2, {'path': '/src/proj.worktrees/x-1'});
      answer = {'exists': false};
      expect((await host.status(made)).exists, isFalse);
      answer = {'removed': true, 'branch': 'canoa/x-1', 'branchKept': true, 'commits': 1};
      expect((await host.remove(made, discard: true)).branchKept, isTrue);
      expect(calls.last.$1, '_canoa/worktree/remove');
      expect(calls.last.$2, {'path': '/src/proj.worktrees/x-1', 'discard': true});
    });
  });

  const worktree = SessionWorktree(path: '/src/proj.worktrees/fix-a1b2c3', branch: 'canoa/fix-a1b2c3', repo: '/src/proj');

  group('the ledger and the list', () {
    test('a session\'s worktree is noted and read back', () async {
      final backend = MemoryLedgerBackend();
      final ledger = SessionLedger(backend);
      ledger.record(peerId: 'local:Claude', sessionId: 's1', entry: 'Claude', place: SessionPlace.local, cwd: worktree.path,
          reopenBy: 'session/load', worktree: worktree);
      await ledger.flush();
      final again = SessionLedger(backend);
      await again.load();
      final read = again.entries.single.worktree!;
      expect((read.path, read.branch, read.repo), (worktree.path, worktree.branch, worktree.repo));
    });

    testWidgets('the list shows each session\'s branch', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.record(peerId: 'local:Claude', sessionId: 's1', entry: 'Claude', place: SessionPlace.local, cwd: worktree.path,
          reopenBy: 'session/load', worktree: worktree, title: 'Fix');
      final store = SessionStore(includePreview: false, ledger: ledger);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: LedgerList(ledger: ledger, store: store))));
      expect(find.textContaining('branch canoa/fix-a1b2c3'), findsOneWidget);
      await tester.pump(SessionLedger.saveDelay);
    });
  });

  group('closing a session', () {
    Future<WorktreeChoice?> ask(WidgetTester tester, WorktreeState? state, {String? problem}) async {
      WorktreeChoice? chosen;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async => chosen = await showDialog<WorktreeChoice>(
          context: context,
          builder: (_) => WorktreeCloseDialog(worktree: worktree, state: state, problem: problem),
        ),
        child: const Text('close'),
      ))));
      await tester.tap(find.text('close'));
      await tester.pumpAndSettle();
      return chosen;
    }

    testWidgets('changes not committed are named, and deleting them is a button that says so', (tester) async {
      await ask(tester, const WorktreeState(branch: 'canoa/fix-a1b2c3', changed: 2, commits: 0));
      expect(find.byKey(const ValueKey('worktree changes')), findsOneWidget);
      expect(find.textContaining('2 files there have changes not committed'), findsOneWidget);
      expect(find.text('Remove worktree'), findsNothing);
      expect(find.text('Delete 2 files and remove'), findsOneWidget);
      await tester.tap(find.text('Keep worktree'));
      await tester.pumpAndSettle();
      expect(find.byType(WorktreeCloseDialog), findsNothing);
    });

    testWidgets('a clean worktree is removed by its own button, and a branch with commits stays', (tester) async {
      WorktreeChoice? chosen;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async => chosen = await showDialog<WorktreeChoice>(
          context: context,
          builder: (_) => const WorktreeCloseDialog(
            worktree: worktree, state: WorktreeState(branch: 'canoa/fix-a1b2c3', changed: 0, commits: 3),
          ),
        ),
        child: const Text('close'),
      ))));
      await tester.tap(find.text('close'));
      await tester.pumpAndSettle();
      expect(find.textContaining('holds 3 commits no other branch has, so it stays'), findsOneWidget);
      expect(find.textContaining('Delete'), findsNothing);
      await tester.tap(find.text('Remove worktree'));
      await tester.pumpAndSettle();
      expect(chosen, WorktreeChoice.remove);
    });

    testWidgets('a state that could not be read offers only keeping it', (tester) async {
      await ask(tester, null, problem: 'the host did not answer');
      expect(find.textContaining('the host did not answer'), findsOneWidget);
      expect(find.text('Remove worktree'), findsNothing);
      expect(find.textContaining('Delete'), findsNothing);
      expect(find.text('Keep worktree'), findsOneWidget);
    });
  });

  testWidgets('the connect dialog starts a session in a worktree when asked', (tester) async {
    const scripted = AgentCommand(label: 'Scripted', program: 'node');
    final asked = <bool>[];
    await tester.binding.setSurfaceSize(const Size(1280, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () => showCanoaConnectDialog(
        context,
        (_) async {},
        onRunLocalAgent: (command, {required cwd, required title, required worktree}) async => asked.add(worktree),
        localAgents: const [scripted],
        checkInstalled: (_) async => true,
        methodUnavailable: (method) async => 'no key here',
        localPeerId: (method) async => throw UnsupportedError('no key'),
        inBrowser: false,
      ),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    final box = find.byKey(const ValueKey('worktree'));
    expect(tester.widget<CheckboxListTile>(box).value, isFalse, reason: 'off unless asked');
    await tester.tap(box);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Run Scripted'));
    await tester.pumpAndSettle();
    expect(asked, [true]);
  });
}
