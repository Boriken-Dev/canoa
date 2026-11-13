// A git worktree per session: the branch a session gets, the host's answers, the ledger's note of it, the list showing the
// branch, and what closing a session asks. A widget test cannot wait on a host, so the widgets are given its state.
import 'dart:math';

import 'package:canoa/auth/peer_proof.dart' show PeerMethod;
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/ledger/ledger_list.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/worktree/close_dialog.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

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

    test('a noted worktree is taken only where its repository and branch put it', () {
      expect(SessionWorktree.fromJson({'path': '/src/proj.worktrees/x-1', 'branch': 'canoa/x-1', 'repo': '/src/proj'}),
          isNotNull);
      expect(SessionWorktree.fromJson({'path': r'C:\src\proj.worktrees\x-1', 'branch': 'canoa/x-1', 'repo': r'C:\src\proj'}),
          isNotNull);
      for (final path in ['/home/me/Downloads/evil.worktrees/x-1', '/src/proj', '/src/proj.worktrees/y-1', '/']) {
        expect(SessionWorktree.fromJson({'path': path, 'branch': 'canoa/x-1', 'repo': '/src/proj'}), isNull, reason: path);
      }
    });

    test('names where its worktree goes, beside the repository', () {
      expect(worktreeLocation('/src/proj', 'canoa/x-1'), '/src/proj.worktrees/x-1');
      expect(worktreeLocation(r'C:\src\proj\', 'canoa/x-1'), 'C:/src/proj.worktrees/x-1');
    });
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
      answer = {'exists': true, 'branch': 'canoa/x-1', 'changed': 2, 'commits': 1, 'detached': false, 'listing': 'a1'};
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
      final store = SessionStore(ledger: ledger);
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
      expect(find.textContaining('2 files there are in no commit'), findsOneWidget);
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

    testWidgets('a detached HEAD holding commits of its own offers only keeping it', (tester) async {
      await ask(tester, const WorktreeState(branch: null, changed: 0, commits: 2, detached: true));
      expect(find.textContaining('detached'), findsOneWidget);
      expect(find.text('Remove worktree'), findsNothing);
      expect(find.textContaining('Delete'), findsNothing);
      expect(find.text('Keep worktree'), findsOneWidget);
    });

    testWidgets('a state that could not be read offers only keeping it', (tester) async {
      await ask(tester, null, problem: 'the host did not answer');
      expect(find.textContaining('the host did not answer'), findsOneWidget);
      expect(find.text('Remove worktree'), findsNothing);
      expect(find.textContaining('Delete'), findsNothing);
      expect(find.text('Keep worktree'), findsOneWidget);
    });
  });

  testWidgets('the connect dialog asks for a worktree for an agent elsewhere when asked', (tester) async {
    final asked = <bool>[];
    await tester.binding.setSurfaceSize(const Size(1280, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () => showCanoaConnectDialog(
        context,
        (target) async => asked.add(target.worktree),
        methodUnavailable: (method) async => null,
        localPeerId: (method) async => 'sha256:own-${method.id}',
        method: PeerMethod.webcrypto,
      ),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    final box = find.byKey(const ValueKey('worktree'));
    expect(tester.widget<CheckboxListTile>(box).value, isFalse, reason: 'off unless asked');
    await tester.ensureVisible(box);
    await tester.tap(box);
    await tester.pumpAndSettle();
    await tester.enterText(find.widgetWithText(TextField, 'WebSocket WSS endpoint'), 'wss://agent.example/acp');
    await tester.enterText(find.widgetWithText(TextField, 'Pinned server peer ID'), 'sha256:${'a' * 64}');
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(asked, [true]);
  });
}
