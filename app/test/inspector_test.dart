// The inspector: a session's environment, its context and its changes in one place, and the git-backed views of the changes. The
// agent here is a stand-in the test drives, and git is a fake that says what the repository holds.
import 'dart:convert' show base64Encode, utf8;
import 'dart:io' show Directory;

import 'package:canoa/acp/delegate.dart' show TaskPhase, delegateStopMethod;
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/chat/agent_events.dart' show Choice, ConfigOption;
import 'package:canoa/chat/background_tasks.dart' show BackgroundCommand;
import 'package:canoa/chat/git_changes.dart';
import 'package:canoa/chat/git_changes_pane.dart';
import 'package:canoa/chat/inspector.dart' show SessionInspector;
import 'package:canoa/chat/outputs.dart';
import 'package:canoa/chat/session_views.dart' show sessionChanges;
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/workspace/open_in.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegation_scene.dart';
import 'fixtures/open_host.dart';
import 'fixtures/session_setup.dart';
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
      session.offer([
        ConfigOption(id: 'mode', label: 'Mode', current: 'accept_edits', category: 'mode', choices: [Choice('manual', 'Manual'), Choice('accept_edits', 'Accept edits')]),
      ]);
      await tester.pump();
      await _openInspector(tester);
      expect(_factText(tester, 'Machine'), 'Machine|This computer');
      expect(_factText(tester, 'Workspace'), 'Workspace|/work/proj');
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
      expect(_factText(tester, 'Workspace'), 'Workspace|/work/proj');
      expect(_factText(tester, 'Worktree'), 'Worktree|/work/proj.worktrees/fix-a1b2c3');
      expect(_factText(tester, 'Branch'), 'Branch|canoa/fix-a1b2c3');
    });

    testWidgets('a hosted session names its host, not this computer', (tester) async {
      await showTask(tester, _desktop, place: SessionPlace.peer, endpoint: Uri.parse('wss://build-box.example:8443/acp'));
      await _openInspector(tester);
      expect(_factText(tester, 'Host'), 'Host|build-box.example');
      expect(_fact('Machine'), findsNothing);
      expect(_factText(tester, 'Workspace'), 'Workspace|/work/proj');
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
      expect(find.text('20% used'), findsOneWidget);
      expect(find.text('200 / 1k'), findsOneWidget);
      agent.usage(640, 1000);
      await settle(tester);
      expect(find.text('64% used'), findsOneWidget);
      expect(find.text('20% used'), findsNothing);
    });

    String rowText(WidgetTester tester, String key) => tester
        .widgetList<Text>(find.descendant(of: find.byKey(ValueKey(key)), matching: find.byType(Text)))
        .map((text) => text.data)
        .whereType<String>()
        .join('|');

    testWidgets('as Claude reports it today, is the window split into used and free, and says what is not reported', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      // The shape a real Claude turn sent: used, size and cost, and the model in `_meta`.
      agent.update({
        'sessionUpdate': 'usage_update',
        'used': 400000,
        'size': 1000000,
        'cost': {'amount': 0.39, 'currency': 'USD'},
        '_meta': {'_claude/model': 'claude-opus'},
      });
      await settle(tester);
      expect(find.text('40% used'), findsOneWidget);
      expect(find.text('400k / 1M'), findsOneWidget);
      expect(rowText(tester, 'context used'), 'Used|400k|40.0%');
      expect(rowText(tester, 'context free'), 'Free space|600k|60.0%');
      expect(find.byKey(const ValueKey('context unreported')), findsOneWidget);
      expect(find.byKey(const ValueKey('context kind Messages')), findsNothing, reason: 'a kind nobody named is not made up');
    });

    testWidgets('as Codex reports it, with only its figures and a token split of the turn, is the same split and invents no kind', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      agent.update({'sessionUpdate': 'usage_update', 'used': 25000, 'size': 200000});
      agent.update({
        'sessionUpdate': 'state_update',
        'state': 'idle',
        'stopReason': 'end_turn',
        'usage': {'inputTokens': 9000, 'outputTokens': 700, 'cachedReadTokens': 0, 'cachedWriteTokens': 0, 'totalTokens': 9700},
        '_meta': {
          'quota': {
            'token_count': {'totalTokens': 9700, 'inputTokens': 9000, 'cachedInputTokens': 0, 'cachedWriteTokens': 0, 'outputTokens': 700, 'reasoningOutputTokens': 200},
          },
        },
      });
      await settle(tester);
      expect(find.text('13% used'), findsOneWidget);
      expect(rowText(tester, 'context free'), 'Free space|175k|87.5%');
      expect(find.byKey(const ValueKey('context unreported')), findsOneWidget);
    });

    testWidgets('shows a row for each kind the agent names, with its tokens and share of the window, and deferred kinds apart', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      agent.update({
        'sessionUpdate': 'usage_update',
        'used': 400000,
        'size': 1000000,
        '_meta': {
          '_claude/contextUsage': {
            'categories': [
              {'name': 'Messages', 'tokens': 322600},
              {'name': 'System tools', 'tokens': 27800},
              {'name': 'Memory files', 'tokens': 16600},
              {'name': 'Free space', 'tokens': 600000},
              {'name': 'MCP tools (deferred)', 'tokens': 20800, 'isDeferred': true},
              {'name': 'Broken', 'tokens': 'many'},
            ],
          },
        },
      });
      await settle(tester);
      expect(rowText(tester, 'context kind Messages'), 'Messages|323k|32.3%');
      expect(rowText(tester, 'context kind System tools'), 'System tools|27.8k|2.8%');
      expect(rowText(tester, 'context kind Memory files'), 'Memory files|16.6k|1.7%');
      expect(rowText(tester, 'context free'), 'Free space|600k|60.0%');
      expect(rowText(tester, 'context kind MCP tools (deferred)'), 'MCP tools (deferred)|20.8k|—');
      expect(find.byKey(const ValueKey('context kind Broken')), findsNothing);
      expect(find.byKey(const ValueKey('context unreported')), findsNothing);
      expect(find.byKey(const ValueKey('context used')), findsNothing, reason: 'the kinds replace the single used row');
      final segments = find.descendant(of: find.byKey(const ValueKey('context bar')), matching: find.byType(Expanded));
      expect(tester.widgetList<Expanded>(segments).map((each) => each.flex).toList(), [322600, 27800, 16600, 600000], reason: 'one segment for each kind that is in the window, then the free space');
    });

    testWidgets('looks the window by kind, with the bar split and deferred kinds apart', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      agent.update({
        'sessionUpdate': 'usage_update',
        'used': 408100,
        'size': 1000000,
        '_meta': {
          '_claude/contextUsage': {
            'categories': [
              {'name': 'Messages', 'tokens': 322600},
              {'name': 'System tools', 'tokens': 27800},
              {'name': 'Memory files', 'tokens': 16500},
              {'name': 'Skills', 'tokens': 6200},
              {'name': 'Autocompact buffer', 'tokens': 33000},
              {'name': 'Free space', 'tokens': 558900},
              {'name': 'MCP tools (deferred)', 'tokens': 20800, 'isDeferred': true},
            ],
          },
        },
      });
      await settle(tester);
      await tester.pumpAndSettle();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/inspector_context_kinds.png'));
    });

    testWidgets('a report that names no kinds replaces the ones before it', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      agent.update({
        'sessionUpdate': 'usage_update',
        'used': 100,
        'size': 1000,
        '_meta': {
          '_claude/contextUsage': {
            'categories': [
              {'name': 'Messages', 'tokens': 100},
            ],
          },
        },
      });
      await settle(tester);
      expect(find.byKey(const ValueKey('context kind Messages')), findsOneWidget);
      agent.usage(300, 1000);
      await settle(tester);
      expect(find.byKey(const ValueKey('context kind Messages')), findsNothing);
      expect(find.byKey(const ValueKey('context unreported')), findsOneWidget);
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
      final composer = tester.widget<TextField>(find.byWidgetPredicate((widget) => widget is TextField && (widget.decoration?.hintText ?? '').startsWith('Message')));
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

  group('the background tasks', () {
    Future<void> details(WidgetTester tester) async {
      if (find.byTooltip('Session details').evaluate().isEmpty) {
        await tester.tap(find.byTooltip('More'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Session details'));
      } else {
        await tester.tap(find.byTooltip('Session details'));
      }
      await tester.pumpAndSettle();
    }

    Map<String, Object?> terminal(String id, {String? command, int? exit}) => {
      'jsonrpc': '2.0',
      'method': 'session/update',
      'params': {
        'sessionId': 's1',
        'update': {
          'sessionUpdate': 'terminal_update',
          'terminalId': id,
          'command': ?command,
          'output': {'data': base64Encode(utf8.encode('listening\n'))},
          if (exit != null) 'exitStatus': {'exitCode': exit},
        },
      },
    };

    Finder inRow(String id, String text) => find.descendant(of: find.byKey(ValueKey('lineage $id')), matching: find.text(text));

    testWidgets('each child is a row with its state and elapsed time, and the heading counts those running', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await scene.fourChildren();
      await details(tester);
      expect(find.text('Background tasks · 4 running'), findsOneWidget);
      for (final title in childTitles) {
        expect(find.text(title), findsOneWidget);
      }
      expect(inRow('c0', 'Working'), findsOneWidget);
      expect(inRow('c0', '25s'), findsOneWidget);
      await scene.end('c0', 'COMPLETED', at: 18);
      await scene.end('c1', 'FAILED', at: 21, note: 'No login for it.');
      expect(find.text('Background tasks · 2 running'), findsOneWidget);
      expect(find.text('2 finished'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('tasks finished')));
      await tester.pump();
      expect(inRow('c0', 'Completed'), findsOneWidget);
      expect(inRow('c1', 'Failed'), findsOneWidget);
      expect(inRow('c0', '18s'), findsOneWidget);
    });

    testWidgets('a session that handed nothing out has no lineage', (tester) async {
      await openScene(tester, size: const Size(1160, 900));
      await details(tester);
      expect(find.textContaining('Background tasks'), findsNothing);
      expect(find.text('Environment'), findsOneWidget);
    });

    testWidgets('a child appears while the inspector is open, and a tap opens its thread', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await details(tester);
      expect(find.textContaining('Background tasks'), findsNothing);
      await scene.fourChildren();
      expect(find.text('Background tasks · 4 running'), findsOneWidget);
      expect(scene.session.openChild, isNull);
      await tester.tap(find.byKey(const ValueKey('lineage c2')));
      await tester.pump(const Duration(milliseconds: 500));
      expect(scene.session.openChild, 'c2');
    });

    testWidgets('a command still running is listed with its time, and folds under "finished" when it ends', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await details(tester);
      expect(find.textContaining('Background tasks'), findsNothing);
      await scene.say(terminal('t1', command: 'npm run dev'));
      await scene.say(terminal('t2', command: 'cargo test', exit: 0));
      expect(find.text('Background tasks · 1 running'), findsOneWidget);
      Finder inCommand(String id, String text) => find.descendant(of: find.byKey(ValueKey('command $id')), matching: find.text(text));
      expect(inCommand('t1', 'npm run dev'), findsOneWidget);
      expect(inCommand('t1', 'under a second'), findsOneWidget);
      scene.now = scene.now.add(const Duration(seconds: 75));
      await tester.pump(const Duration(seconds: 1));
      expect(inCommand('t1', '1m 15s'), findsOneWidget, reason: 'the time keeps ticking while it runs');
      expect(find.byKey(const ValueKey('command t2')), findsNothing, reason: 'a finished one is folded');
      expect(find.text('1 finished'), findsOneWidget);
      await scene.say(terminal('t1', exit: 130));
      expect(find.text('Background tasks'), findsOneWidget);
      expect(find.text('2 finished'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('tasks finished')));
      await tester.pump();
      expect(inCommand('t2', 'Done'), findsOneWidget);
      expect(inCommand('t1', 'Exit 130'), findsOneWidget);
      expect(inCommand('t1', '1m 15s'), findsOneWidget, reason: 'a finished command keeps how long it ran');
    });

    testWidgets('a command has no Stop: the agent runs it in its own terminal', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await scene.say(terminal('t1', command: 'npm run dev'));
      await details(tester);
      expect(find.byKey(const ValueKey('command t1')), findsOneWidget);
      expect(find.byIcon(Icons.stop_circle_outlined), findsNothing);
    });

    testWidgets('a running subagent has a Stop that asks the host to stop that one task, and it leaves the running list', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await scene.fourChildren();
      await details(tester);
      await tester.tap(find.byKey(const ValueKey('stop c1')));
      await tester.pump();
      await scene.settle();
      expect(scene.host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1', 'taskId': 'c1'});
      expect(scene.session.tasks['c1']!.phase, TaskPhase.canceled);
      expect(find.text('Background tasks · 3 running'), findsOneWidget);
      expect(find.byKey(const ValueKey('stop c1')), findsNothing);
      expect(find.byKey(const ValueKey('lineage c1')), findsNothing, reason: 'folded under finished');
      expect(find.text('1 finished'), findsOneWidget);
    });

    testWidgets('a subagent started by the conversation itself has no Stop', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await scene.nativeChildren('subagents');
      await details(tester);
      expect(find.byKey(const ValueKey('lineage panel')), findsOneWidget);
      expect(find.byIcon(Icons.stop_circle_outlined), findsNothing);
    });

    testWidgets('past four running the rest wait behind Show more, and finished ones behind their own fold', (tester) async {
      final scene = await openScene(tester, size: const Size(1160, 900));
      await scene.fourChildren();
      await scene.say(terminal('t1', command: 'npm run dev'));
      await scene.say(terminal('t2', command: 'cargo test', exit: 0));
      await details(tester);
      expect(find.text('Background tasks · 5 running'), findsOneWidget);
      expect(find.byKey(const ValueKey('command t1')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('tasks more')));
      await tester.pump();
      expect(find.byKey(const ValueKey('command t1')), findsOneWidget);
      expect(find.text('Show less'), findsOneWidget);
      expect(find.byKey(const ValueKey('command t2')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('tasks finished')));
      await tester.pump();
      expect(find.byKey(const ValueKey('command t2')), findsOneWidget);
    });

    test('a command whose start this app did not see has no time', () {
      const unseen = BackgroundCommand(id: 't', command: 'make', running: true);
      expect(unseen.elapsedAt(DateTime.utc(2026)), isNull);
      final seen = BackgroundCommand(id: 't', command: 'make', running: false, started: DateTime.utc(2026), ended: DateTime.utc(2026, 1, 1, 0, 2));
      expect(seen.elapsedAt(DateTime.utc(2027)), const Duration(minutes: 2));
    });

    testWidgets('on a phone the panel holds the lineage across the window, a child closes it onto its thread, and the panel opens again', (tester) async {
      final scene = await openScene(tester, size: const Size(390, 844));
      await scene.fourChildren();
      await details(tester);
      expect(find.byType(WorkPanel), findsOneWidget);
      expect(find.text('Background tasks · 4 running'), findsOneWidget);
      await tester.tap(find.byTooltip('Back to the conversation'));
      await tester.pumpAndSettle();
      expect(find.byType(WorkPanel), findsNothing);
      await details(tester);
      expect(find.byType(WorkPanel), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('lineage c1')));
      await tester.pump(const Duration(milliseconds: 500));
      await tester.pump(const Duration(milliseconds: 500));
      expect(find.byType(WorkPanel), findsNothing);
      expect(scene.session.openChild, 'c1');
      expect(scene.session.tasks['c1']!.phase, TaskPhase.working);
    });

    for (final (label, size) in [('phone', const Size(390, 900)), ('desktop', const Size(1280, 800))]) {
      testWidgets('the inspector with children and a running command, $label', (tester) async {
        final scene = await openScene(tester, size: size);
        await scene.fourChildren();
        await scene.end('c0', 'COMPLETED', at: 18);
        await scene.end('c3', 'FAILED', at: 21, note: 'No login for it.');
        await scene.say(terminal('t1', command: 'npm run dev -- --port 5173'));
        await details(tester);
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('tasks finished')));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/inspector_lineage_$label.png'));
      });
    }
  });

  group('looks', () {
    for (final (label, size) in [('phone', _phone), ('desktop', _desktop)]) {
      testWidgets('the inspector at $label width, in a worktree with changes from git', (tester) async {
        const worktree = SessionWorktree(path: '/work/proj.worktrees/fix-a1b2c3', branch: 'canoa/fix-a1b2c3', repo: '/work/proj');
        final git = FakeGit()..current = _disk([('src/calc.js', 4, 1), ('README.md', 3, 0), ('test/calc_test.js', 12, 2)], branch: 'canoa/fix-a1b2c3');
        final (agent, session, store) = await showTask(tester, size, git: git, cwd: '/work/proj.worktrees/fix-a1b2c3', worktree: worktree);
        session.offer([
          ConfigOption(id: 'mode', label: 'Mode', current: 'accept_edits', category: 'mode', choices: [Choice('manual', 'Manual'), Choice('accept_edits', 'Accept edits')]),
        ]);
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
        expect(find.text('31% used'), findsOneWidget);
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

    testWidgets('on a phone the inspector is a panel across the window, with a way back', (tester) async {
      final (agent, _, _) = await showTask(tester, _phone);
      agent.task();
      await settle(tester);
      await tester.tap(find.byTooltip('Changes'));
      await tester.pumpAndSettle();
      expect(find.byType(WorkPanel), findsOneWidget);
      expect(find.text('Environment'), findsOneWidget);
      await tester.tap(find.byTooltip('Back to the conversation'));
      await tester.pumpAndSettle();
      expect(find.byType(WorkPanel), findsNothing);
    });
  });

  group('the outputs', () {
    /// A tool call that deleted [path]: Claude names the file in the call's locations, not in a diff.
    void deleted(TaskAgent agent, String id, String path) =>
        agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': id, 'title': 'Delete', 'kind': 'delete', 'status': 'completed', 'locations': [{'path': path}]});

    void wrote(TaskAgent agent, String id, String path) => agent.tool(id, 'completed', [{'type': 'diff', 'path': path, 'newText': 'text\n'}]);

    Future<void> inspector(WidgetTester tester, SessionStore store, ChatSession session, {ValueChanged<String>? onOpenInFiles, WorkspaceOpener? opener}) async {
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: SessionInspector(
            store: store,
            session: session,
            scopes: const [ChangeScope.reported],
            scope: ChangeScope.reported,
            onScope: (_) {},
            onOpenDiff: () {},
            onClose: () {},
            onOpenInFiles: onOpenInFiles,
            opener: opener,
          ),
        ),
      ));
      await tester.pumpAndSettle();
    }

    double top(WidgetTester tester, String path) => tester.getTopLeft(find.byKey(ValueKey('output $path'))).dy;

    testWidgets('lists each file the tools wrote or changed once, the one touched last first, and leaves out a refused edit', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      agent.task();
      await settle(tester);
      await _openInspector(tester);
      expect(find.text('Outputs · 3'), findsOneWidget);
      final rows = ['/work/proj/src/report.js', '/work/proj/src/calc.js', '/work/proj/README.md'];
      for (final path in rows) {
        expect(find.byKey(ValueKey('output $path')), findsOneWidget, reason: path);
      }
      expect(top(tester, rows[0]), lessThan(top(tester, rows[1])));
      expect(top(tester, rows[1]), lessThan(top(tester, rows[2])), reason: 'calc.js was edited after README.md, so it is listed above it');
      expect(find.byKey(const ValueKey('output /work/proj/obsolete.txt')), findsNothing, reason: 'a refused edit changed nothing');
      expect(find.descendant(of: find.byKey(const ValueKey('output /work/proj/src/report.js')), matching: find.text('src/report.js')), findsOneWidget, reason: 'a path is shown relative to the workspace');
    });

    testWidgets('a path written two ways is one output, and a session that wrote nothing has no list', (tester) async {
      final (agent, session, _) = await showTask(tester, _desktop);
      await _openInspector(tester);
      expect(find.textContaining('Outputs'), findsNothing);
      wrote(agent, 'a', 'src/a.js');
      wrote(agent, 'b', '/work/proj/src/a.js');
      wrote(agent, 'c', './src/b.js');
      await settle(tester);
      expect([for (final output in outputsOf(session, '/work/proj')) output.path], ['/work/proj/src/b.js', '/work/proj/src/a.js']);
      expect(find.text('Outputs · 2'), findsOneWidget);
    });

    testWidgets('a deleted file is shown as deleted and opens nothing, until a tool writes it again', (tester) async {
      final (agent, session, store) = await showTask(tester, _desktop);
      wrote(agent, 'a', '/work/proj/old.txt');
      deleted(agent, 'rm', '/work/proj/old.txt:1');
      await settle(tester);
      final opened = <String>[];
      await inspector(tester, store, session, onOpenInFiles: opened.add);
      expect(find.byKey(const ValueKey('output deleted /work/proj/old.txt')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('output /work/proj/old.txt')));
      await tester.pumpAndSettle();
      expect(opened, isEmpty);
      expect(find.byKey(const ValueKey('output menu /work/proj/old.txt')), findsNothing);
      wrote(agent, 'again', '/work/proj/old.txt');
      await settle(tester);
      await inspector(tester, store, session, onOpenInFiles: opened.add);
      expect(find.byKey(const ValueKey('output deleted /work/proj/old.txt')), findsNothing);
    });

    testWidgets('past five files the rest wait behind Show more', (tester) async {
      final (agent, _, _) = await showTask(tester, _desktop);
      for (var at = 0; at < 7; at++) {
        wrote(agent, 'w$at', '/work/proj/f$at.txt');
      }
      await settle(tester);
      await _openInspector(tester);
      expect(find.text('Outputs · 7'), findsOneWidget);
      expect(find.byKey(const ValueKey('output /work/proj/f6.txt')), findsOneWidget);
      expect(find.byKey(const ValueKey('output /work/proj/f2.txt')), findsOneWidget);
      expect(find.byKey(const ValueKey('output /work/proj/f1.txt')), findsNothing);
      await tester.ensureVisible(find.byKey(const ValueKey('outputs more')));
      await tester.tap(find.byKey(const ValueKey('outputs more')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('output /work/proj/f0.txt')), findsOneWidget);
      expect(find.text('Show fewer'), findsOneWidget);
    });

    testWidgets('a tap opens the file in Files, and the menu offers it with the file manager where this computer can start one', (tester) async {
      final (agent, session, store) = await showTask(tester, _desktop);
      wrote(agent, 'a', '/work/proj/src/a.js');
      await settle(tester);
      final host = OpenHost(ids: const ['file_manager', 'reveal']);
      final opened = <String>[];
      await inspector(tester, store, session, onOpenInFiles: opened.add, opener: await host.opener());
      await tester.tap(find.byKey(const ValueKey('output /work/proj/src/a.js')));
      await tester.pumpAndSettle();
      expect(opened, ['/work/proj/src/a.js']);

      await tester.tap(find.byKey(const ValueKey('output menu /work/proj/src/a.js')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('output action files')), findsOneWidget);
      expect(find.byKey(const ValueKey('output action reveal')), findsOneWidget);
      expect(find.byKey(const ValueKey('output action vscode')), findsNothing, reason: 'VS Code is not on this stand-in host');
      await tester.tap(find.byKey(const ValueKey('output action reveal')));
      await tester.pumpAndSettle();
      expect(host.started.single, ('/work/proj/src/a.js', 'reveal'));
    });

    testWidgets('where the files are on another host the menu has Open in Files alone, and with no Files there is no menu', (tester) async {
      final (agent, session, store) = await showTask(tester, _desktop);
      wrote(agent, 'a', '/work/proj/a.js');
      await settle(tester);
      await inspector(tester, store, session, onOpenInFiles: (_) {});
      await tester.tap(find.byKey(const ValueKey('output menu /work/proj/a.js')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('output action files')), findsOneWidget);
      expect(find.byKey(const ValueKey('output action reveal')), findsNothing);
      await tester.tapAt(Offset.zero);
      await tester.pumpAndSettle();
      await inspector(tester, store, session);
      expect(find.byKey(const ValueKey('output /work/proj/a.js')), findsOneWidget);
      expect(find.byKey(const ValueKey('output menu /work/proj/a.js')), findsNothing);
    });

    for (final (label, size, menu) in [('desktop', _desktop, false), ('desktop_menu', _desktop, true), ('phone', _phone, false)]) {
      testWidgets('the outputs, $label', (tester) async {
        final (agent, session, store) = await showTask(tester, size);
        agent.task();
        wrote(agent, 'notes', '/work/proj/docs/notes.md');
        wrote(agent, 'gone', '/work/proj/obsolete.txt');
        deleted(agent, 'rm', '/work/proj/obsolete.txt');
        agent.usage(62000, 200000);
        await settle(tester);
        await inspector(tester, store, session, onOpenInFiles: (_) {}, opener: await OpenHost(ids: const ['file_manager', 'reveal']).opener());
        if (menu) {
          await tester.tap(find.byKey(const ValueKey('output menu /work/proj/src/report.js')));
          await tester.pumpAndSettle();
        }
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/inspector_outputs_$label.png'));
      });
    }
  });
}
