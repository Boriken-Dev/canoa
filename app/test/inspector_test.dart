// The inspector: a session's environment, its context and its changes in one place, and the git-backed views of the changes. The
// agent here is a stand-in the test drives, and git is a fake that says what the repository holds.
import 'dart:io' show Directory;

import 'package:canoa/chat/agent_events.dart' show Choice, ConfigOption;
import 'package:canoa/chat/git_changes.dart';
import 'package:canoa/chat/git_changes_pane.dart';
import 'package:canoa/chat/session_views.dart' show sessionChanges;
import 'package:canoa/session_store.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/task_agent.dart';

const _desktop = Size(1280, 800);
const _phone = Size(390, 844);

Finder _fact(String label) => find.byKey(ValueKey('fact $label'));

String _factText(WidgetTester tester, String label) =>
    tester.widgetList<Text>(find.descendant(of: _fact(label), matching: find.byType(Text))).map((text) => text.data).whereType<String>().join('|');

Future<void> _openInspector(WidgetTester tester, {String tooltip = 'Session details'}) async {
  await tester.tap(find.byTooltip(tooltip));
  await tester.pumpAndSettle();
}

/// The three states a repository goes through in a session: the commit it began on, and the disk after each turn.
GitState _disk(List<(String, int, int)> files, {int revision = 0, String? branch = 'main'}) => gitState(files, revision: revision, branch: branch);

void main() {
  setUpAll(() => goldenFileComparator = TolerantComparator(Uri.file('${Directory.current.path}/test/inspector_test.dart')));

  group('the environment', () {
    testWidgets('a local session shows this computer, its folder and its mode, and no worktree', (tester) async {
      final (_, session, _) = await showTask(tester, _desktop);
      session.options = const [
        ConfigOption(id: 'mode', label: 'Mode', current: 'accept_edits', category: 'mode', choices: [Choice('manual', 'Manual'), Choice('accept_edits', 'Accept edits')]),
      ];
      await tester.pump();
      await _openInspector(tester);
      expect(_factText(tester, 'Machine'), 'Machine|This computer');
      expect(_factText(tester, 'Folder'), 'Folder|/work/proj');
      expect(_factText(tester, 'Mode'), 'Mode|Accept edits');
      expect(_fact('Worktree'), findsNothing);
      expect(_fact('Branch'), findsNothing, reason: 'no branch is known until git has been read');
      expect(_fact('Host'), findsNothing);
    });

    testWidgets('a local session in a repository shows the branch git reports', (tester) async {
      final git = FakeGit()..current = _disk([('a.txt', 1, 0)], branch: 'feature/login');
      await showTask(tester, _desktop, git: git);
      await _openInspector(tester);
      await settle(tester);
      expect(_factText(tester, 'Branch'), 'Branch|feature/login');
    });

    testWidgets('a worktree session shows the repository it was made from, its own folder and its branch', (tester) async {
      const worktree = SessionWorktree(path: '/work/proj.worktrees/fix-a1b2c3', branch: 'canoa/fix-a1b2c3', repo: '/work/proj');
      await showTask(tester, _desktop, cwd: '/work/proj.worktrees/fix-a1b2c3', worktree: worktree);
      await _openInspector(tester);
      expect(_factText(tester, 'Machine'), 'Machine|This computer');
      expect(_factText(tester, 'Folder'), 'Folder|/work/proj');
      expect(_factText(tester, 'Worktree'), 'Worktree|/work/proj.worktrees/fix-a1b2c3');
      expect(_factText(tester, 'Branch'), 'Branch|canoa/fix-a1b2c3');
    });

    testWidgets('a hosted session names its host, not this computer', (tester) async {
      await showTask(tester, _desktop, place: SessionPlace.peer, endpoint: Uri.parse('wss://build-box.example:8443/acp'));
      await _openInspector(tester);
      expect(_factText(tester, 'Host'), 'Host|build-box.example');
      expect(_fact('Machine'), findsNothing);
      expect(_factText(tester, 'Folder'), 'Folder|/work/proj');
    });

    testWidgets('the header names the agent and nothing the inspector shows', (tester) async {
      const worktree = SessionWorktree(path: '/work/proj.worktrees/fix-a1b2c3', branch: 'canoa/fix-a1b2c3', repo: '/work/proj');
      await showTask(tester, _desktop, cwd: '/work/proj.worktrees/fix-a1b2c3', worktree: worktree);
      expect(find.text('Coder'), findsOneWidget);
      expect(find.textContaining('worktree canoa/'), findsNothing);
      expect(find.textContaining('/work/proj'), findsNothing, reason: 'the folder is in the inspector, which is closed');
    });
  });

  group('the context', () {
    testWidgets('is the fraction the agent last reported, and follows the next report', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      expect(find.byKey(const ValueKey('context none')), findsOneWidget);
      agent.usage(200, 1000);
      await settle(tester);
      expect(find.text('20% of context'), findsOneWidget);
      expect(find.text('200 of 1k tokens'), findsOneWidget);
      agent.usage(640, 1000);
      await settle(tester);
      expect(find.text('64% of context'), findsOneWidget);
      expect(find.text('20% of context'), findsNothing);
    });

    testWidgets('is no longer in the usage line under the conversation', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      agent.update({'sessionUpdate': 'usage_update', 'used': 500, 'size': 1000, 'cost': {'amount': 0.25, 'currency': 'USD'}});
      await settle(tester);
      expect(find.text('0.25 USD'), findsOneWidget);
      expect(find.textContaining('of context'), findsNothing, reason: 'the inspector is closed, and the line left it');
    });
  });

  group('the changes', () {
    testWidgets('the header chip adds up what the agent reported and opens the inspector at the changes', (tester) async {
      final (agent, session, _) = await showTask(tester, _desktop);
      expect(find.byKey(const ValueKey('changes chip')), findsNothing, reason: 'nothing has changed yet');
      agent.task();
      await settle(tester);
      final (added, removed) = totalOf(sessionChanges(session));
      expect((added, removed), (6, 3));
      expect(find.descendant(of: find.byKey(const ValueKey('changes chip')), matching: find.text('+6')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const ValueKey('changes chip')), matching: find.text('−3')), findsOneWidget);
      await tester.tap(find.byTooltip('Changes'));
      await tester.pumpAndSettle();
      expect(find.text('Environment'), findsOneWidget);
      expect(find.byKey(const ValueKey('changes added')), findsOneWidget);
      expect(find.text('3 files'), findsOneWidget);
    });

    testWidgets('the section opens today\'s diff, and back returns to the session', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      agent.task();
      await settle(tester);
      await _openInspector(tester, tooltip: 'Changes');
      await tester.tap(find.byKey(const ValueKey('open diff')));
      await tester.pumpAndSettle();
      expect(find.byType(ChangesPane), findsOneWidget);
      expect(find.text('As the agent sent them, not as the files are on disk now. Choose a line to comment on it.'), findsOneWidget);
      await tester.tap(find.byTooltip('Back to the session'));
      await tester.pumpAndSettle();
      expect(find.byType(ChangesPane), findsNothing);
      expect(find.text('Environment'), findsOneWidget);
    });

    testWidgets('with no git for the folder only the agent\'s view is offered', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      agent.task();
      await settle(tester);
      await _openInspector(tester);
      expect(find.byKey(const ValueKey('scope Changes')), findsOneWidget);
      expect(find.byKey(const ValueKey('scope Uncommitted')), findsNothing);
      expect(find.byKey(const ValueKey('scope Last turn')), findsNothing);
      expect(find.byKey(const ValueKey('scope Whole session')), findsNothing);
    });

    testWidgets('a reader that fails leaves the agent\'s view and shows nothing broken', (tester) async {
      final git = FakeGit()..failure = const WorktreeFailure('The host did not answer.');
      final (agent, _, _) = await showTask(tester, _desktop, git: git);
      agent.task();
      await settle(tester);
      await _openInspector(tester);
      expect(tester.takeException(), isNull);
      expect(find.byKey(const ValueKey('scope Changes')), findsOneWidget);
      expect(find.byKey(const ValueKey('scope Whole session')), findsNothing);
      expect(find.byKey(const ValueKey('changes chip')), findsOneWidget);
    });

    testWidgets('a session with neither says so', (tester) async {
      await showTask(tester, _desktop);
      await _openInspector(tester);
      expect(find.byKey(const ValueKey('changes none')), findsOneWidget);
      expect(find.byKey(const ValueKey('open diff')), findsNothing);
    });
  });

  group('the views from git', () {
    testWidgets('the chip counts the disk, not the agent\'s report, and the four views are named', (tester) async {
      final git = FakeGit()..current = _disk([('src/calc.js', 4, 1), ('README.md', 3, 0)]);
      final (agent, _, _) = await showTask(tester, _desktop, git: git);
      agent.task();
      await _openInspector(tester);
      await settle(tester);
      expect(find.descendant(of: find.byKey(const ValueKey('changes chip')), matching: find.text('+7')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const ValueKey('changes chip')), matching: find.text('−1')), findsOneWidget);
      for (final name in ['Changes', 'Uncommitted', 'Last turn', 'Whole session']) {
        expect(find.byKey(ValueKey('scope $name')), findsOneWidget, reason: name);
      }
    });

    testWidgets('Whole session opens the files on disk, labelled as the disk, and a line comments into the message', (tester) async {
      final git = FakeGit()..current = _disk([('src/calc.js', 2, 1)]);
      await showTask(tester, _desktop, git: git);
      await _openInspector(tester);
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('scope Whole session')));
      await tester.pumpAndSettle();
      expect(find.text('+2'), findsWidgets);
      await tester.tap(find.byKey(const ValueKey('open diff')));
      await tester.pumpAndSettle();
      expect(find.byType(ChangesPane), findsNothing, reason: 'the agent\'s pane says the files are not as on disk');
      expect(find.byType(GitChangesPane), findsOneWidget);
      expect(find.textContaining('against where this session began'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('git line src/calc.js new 1')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('change-comment')), 'Keep the old name.');
      await tester.tap(find.text('Add to message'));
      await tester.pumpAndSettle();
      final composer = tester.widget<TextField>(find.byWidgetPredicate((widget) => widget is TextField && (widget.decoration?.hintText ?? '').startsWith('Message agent')));
      expect(composer.controller!.text, 'src/calc.js:1: `new src/calc.js 0`\nKeep the old name.\n');
    });

    testWidgets('Uncommitted is the disk against HEAD, which differs from the session once something was committed', (tester) async {
      final git = FakeGit()..current = gitState([], head: 'aaaa', base: 'aaaa');
      final (_, session, store) = await showTask(tester, _desktop, git: git);
      await tester.runAsync(() => store.refreshGit(session));
      // The agent commits one file's work: HEAD moves on, and the session is still measured from where it began.
      git
        ..current = gitState([('a.txt', 5, 0), ('b.txt', 1, 1)], head: 'bbbb', base: 'aaaa')
        ..uncommitted = gitState([('b.txt', 1, 1)], head: 'bbbb', base: 'bbbb');
      await _openInspector(tester);
      await settle(tester);
      expect(session.git.base, 'aaaa');
      expect([for (final file in session.git.filesOf(ChangeScope.wholeSession)) file.path], ['a.txt', 'b.txt']);
      expect([for (final file in session.git.filesOf(ChangeScope.uncommitted)) file.path], ['b.txt']);
      await tester.tap(find.byKey(const ValueKey('scope Uncommitted')));
      await tester.pumpAndSettle();
      expect(find.text('1 file'), findsOneWidget);
      expect(find.text('b.txt'), findsOneWidget);
      expect(find.text('a.txt'), findsNothing);
      expect(git.reads.where((read) => read.$2 == null), isNotEmpty, reason: 'HEAD is read with no base');
      expect(git.reads.where((read) => read.$2 == 'aaaa'), isNotEmpty, reason: 'the session is read from its first commit');
    });

    testWidgets('Last turn is the files the turn changed, measured from the disk as the turn began', (tester) async {
      final git = FakeGit()..current = _disk([('old.txt', 3, 0), ('edited.txt', 1, 0)]);
      final (agent, session, store) = await showTask(tester, _desktop, git: git, holdTurns: true);
      await tester.runAsync(() => store.refreshGit(session));
      expect(session.git.lastTurn, isEmpty, reason: 'no turn has begun');
      await tester.runAsync(() async {
        final sent = store.sendText('go');
        await Future<void>.delayed(const Duration(milliseconds: 60));
        // The turn edits one file, adds one and leaves another as it was.
        git.current = _disk([('old.txt', 3, 0), ('edited.txt', 4, 1), ('fresh.txt', 2, 0)], revision: 1);
        agent.finishTurn();
        await sent;
        await Future<void>.delayed(const Duration(milliseconds: 60));
      });
      await tester.pump();
      expect([for (final file in session.git.lastTurn) file.path], ['edited.txt', 'fresh.txt']);
      expect([for (final file in session.git.filesOf(ChangeScope.wholeSession)) file.path], ['old.txt', 'edited.txt', 'fresh.txt']);
      await _openInspector(tester);
      await tester.tap(find.byKey(const ValueKey('scope Last turn')));
      await tester.pumpAndSettle();
      expect(find.text('2 files'), findsOneWidget);
      expect(find.text('old.txt'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('open diff')));
      await tester.pumpAndSettle();
      expect(find.text('fresh.txt'), findsOneWidget);
      expect(find.text('edited.txt'), findsOneWidget);
      expect(find.text('old.txt'), findsNothing);
    });

    testWidgets('a binary or too large file is listed with its counts and no lines', (tester) async {
      final git = FakeGit()..current = const GitState(head: 'aaaa', branch: 'main', base: 'aaaa', added: 0, removed: 0, more: false, files: [
        GitFile(path: 'logo.png', status: 'A', added: 0, removed: 0, binary: true, patch: null, cut: false, print: 'p1'),
        GitFile(path: 'huge.json', status: 'M', added: 90000, removed: 3, binary: false, patch: null, cut: false, print: 'p2'),
      ]);
      await showTask(tester, _desktop, git: git);
      await _openInspector(tester);
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('scope Whole session')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('open diff')));
      await tester.pumpAndSettle();
      expect(find.text('A binary file: no lines to show.'), findsOneWidget);
      expect(find.text('Too large to show here.'), findsOneWidget);
    });

    testWidgets('what git returns is data: markup and bidi text in a path are drawn as text', (tester) async {
      final git = FakeGit()..current = gitState([('<b>x</b>\u202Eexe.txt', 1, 0)]);
      await showTask(tester, _desktop, git: git);
      await _openInspector(tester);
      await settle(tester);
      expect(tester.takeException(), isNull);
      expect(find.textContaining('<b>x</b>'), findsWidgets);
    });
  });

  group('the answer from a host', () {
    test('is read into files, a record at a time, and anything less is dropped', () {
      final state = GitState.fromJson({
        'exists': true, 'head': 'a' * 40, 'branch': null, 'base': 'b' * 40, 'added': 3, 'removed': 1, 'more': true,
        'files': [
          {'path': 'a.txt', 'status': 'M', 'added': 3, 'removed': 1, 'binary': false, 'patch': '@@ -1 +1,3 @@\n-x\n+y\n+z\n+w\n', 'cut': false, 'print': 'p'},
          {'path': 'broken'},
          7,
        ],
      })!;
      expect(state.branch, isNull);
      expect(state.more, isTrue);
      expect([for (final file in state.files) file.path], ['a.txt']);
      expect((state.files.single.change.added, state.files.single.change.removed), (3, 1));
      expect(GitState.fromJson({'exists': false}), isNull);
      expect(GitState.fromJson({'exists': true, 'files': 'x'}), isNull);
    });

    test('is asked for by path and base, and a folder that is no repository is no state', () async {
      final asked = <(String, Map<String, Object?>)>[];
      final git = HostGit((method, params) async {
        asked.add((method, params));
        return {'exists': false};
      });
      expect(await git.read('/work/proj'), isNull);
      expect(await git.read('/work/proj', base: 'c' * 40), isNull);
      expect([for (final (method, _) in asked) method], ['_canoa/worktree/diff', '_canoa/worktree/diff']);
      expect([for (final (_, params) in asked) params], [
        {'path': '/work/proj'},
        {'path': '/work/proj', 'base': 'c' * 40},
      ]);
    });

    test('a session that was never read has no last turn, and a repeated read does not move its first commit', () async {
      final fake = FakeGit()..current = gitState([('a', 1, 0)], head: 'h1', base: 'h1');
      final session = SessionGit();
      expect(session.lastTurn, isEmpty);
      await session.refresh(fake, '/f');
      expect(session.base, 'h1');
      fake.current = gitState([('a', 1, 0)], head: 'h2', base: 'h1');
      await session.refresh(fake, '/f');
      expect(session.base, 'h1');
    });
  });

  group('looks', () {
    for (final (label, size) in [('phone', _phone), ('desktop', _desktop)]) {
      testWidgets('the inspector at $label width, in a worktree with changes from git', (tester) async {
        const worktree = SessionWorktree(path: '/work/proj.worktrees/fix-a1b2c3', branch: 'canoa/fix-a1b2c3', repo: '/work/proj');
        final git = FakeGit()..current = _disk([('src/calc.js', 4, 1), ('README.md', 3, 0), ('test/calc_test.js', 12, 2)], branch: 'canoa/fix-a1b2c3');
        final (agent, session, store) = await showTask(tester, size, git: git, cwd: '/work/proj.worktrees/fix-a1b2c3', worktree: worktree);
        session.options = const [
          ConfigOption(id: 'mode', label: 'Mode', current: 'accept_edits', category: 'mode', choices: [Choice('manual', 'Manual'), Choice('accept_edits', 'Accept edits')]),
        ];
        agent.usage(62000, 200000);
        await tester.runAsync(() => store.refreshGit(session));
        await settle(tester);
        if (size == _phone) {
          await tester.tap(find.byTooltip('Changes'));
        } else {
          await tester.tap(find.byTooltip('Session details'));
        }
        await tester.pumpAndSettle();
        await settle(tester);
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        expect(find.text('31% of context'), findsOneWidget);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/inspector_$label.png'));
      });
    }

    for (final (label, size) in [('phone', _phone), ('desktop', _desktop)]) {
      testWidgets('the whole session from git at $label width', (tester) async {
        final git = FakeGit()..current = _disk([('src/calc.js', 3, 1), ('README.md', 2, 0)]);
        final (_, session, store) = await showTask(tester, size, git: git);
        await tester.runAsync(() => store.refreshGit(session));
        await settle(tester);
        await tester.tap(find.byTooltip(size == _phone ? 'Changes' : 'Session details'));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('scope Whole session')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('open diff')));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        expect(find.byType(GitChangesPane), findsOneWidget);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/git_changes_$label.png'));
      });
    }

    testWidgets('on a phone the inspector is a sheet, which closes', (tester) async {
      final (agent, _, _) = await showTask(tester, _phone);
      agent.task();
      await settle(tester);
      await tester.tap(find.byTooltip('Changes'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.text('Environment'), findsOneWidget);
      await tester.tap(find.byTooltip('Close details'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
    });
  });
}
