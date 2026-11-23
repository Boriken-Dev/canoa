// Commit, push and pull request from the Changes pane: the pane drives a publisher the host stands behind, and shows git's own
// words when it refuses. The publisher here is a fake that records what it was asked.
import 'dart:convert' show jsonEncode;

import 'package:canoa/chat/git_changes.dart';
import 'package:canoa/chat/git_publish.dart';
import 'package:canoa/store/links.dart' as links;
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/task_agent.dart';

const _desktop = Size(1280, 800);

/// A fake host that commits, pushes and opens pull requests as a test says, and what the disk holds after a commit.
final class FakePublishingGit extends FakeGit implements GitPublisher {
  final commits = <(String, String, List<String>?)>[];
  final pushes = <(String, String?, bool)>[];
  final requests = <(String, String, String, String?, bool)>[];

  /// Fails the next call of that name with these words.
  final failures = <String, String>{};

  /// What a pull request answers.
  PullRequestOpened opened = const PullRequestOpened(viaGh: true, url: 'https://forge.invalid/acme/widgets/pull/7', branch: 'main');

  void _maybeFail(String name) {
    final words = failures.remove(name);
    if (words != null) throw WorktreeFailure(words);
  }

  @override
  Future<GitCommitted> commit(String folder, String message, {List<String>? paths}) async {
    commits.add((folder, message, paths));
    _maybeFail('commit');
    final left = [for (final file in uncommitted?.files ?? const <GitFile>[]) if (paths != null && !paths.contains(file.path)) (file.path, file.added, file.removed)];
    current = uncommitted = gitState(left);
    return const GitCommitted(commit: 'abcdef0123456789', summary: 'the subject', branch: 'main');
  }

  @override
  Future<void> push(String folder, {String? remote, bool setUpstream = false}) async {
    pushes.add((folder, remote, setUpstream));
    _maybeFail('push');
  }

  @override
  Future<PullRequestOpened> pullRequest(String folder, {required String title, String body = '', String? base, bool draft = false}) async {
    requests.add((folder, title, body, base, draft));
    _maybeFail('pull request');
    return opened;
  }
}

/// The Changes pane on the disk against HEAD, with [files] there.
Future<FakePublishingGit> _open(WidgetTester tester, List<(String, int, int)> files) async {
  final git = FakePublishingGit()
    ..current = gitState(files)
    ..uncommitted = gitState(files);
  await showTask(tester, _desktop, git: git);
  await tester.tap(find.byTooltip('Session details'));
  await tester.pumpAndSettle();
  await settle(tester);
  await tester.tap(find.byKey(const ValueKey('scope Uncommitted')));
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(const ValueKey('open diff')));
  await tester.pumpAndSettle();
  return git;
}

String _result(WidgetTester tester) => tester.widget<SelectableText>(find.byKey(const ValueKey('publish result'))).data!;

void main() {
  testWidgets('the files chosen are committed with the message, and the pane reads the repository again', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0), ('b.txt', 2, 1)]);
    expect(find.byKey(const ValueKey('publish chosen')), findsOneWidget);
    expect(find.text('2 of 2 files to commit'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('publish files')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('publish file b.txt')));
    await tester.pumpAndSettle();
    expect(find.text('1 of 2 files to commit'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const ValueKey('publish commit'))).onPressed, isNull, reason: 'no message yet');
    await tester.enterText(find.byKey(const ValueKey('publish message')), 'Fix the sum');
    await tester.pump();
    final reads = git.reads.length;
    await tester.tap(find.byKey(const ValueKey('publish commit')));
    await tester.pumpAndSettle();
    expect(git.commits.single.$1, '/work/proj');
    expect(git.commits.single.$2, 'Fix the sum');
    expect(git.commits.single.$3, ['a.txt']);
    expect(_result(tester), 'Committed abcdef0 the subject');
    expect(git.reads.length, greaterThan(reads), reason: 'the repository is read again');
    expect(find.text('1 of 1 file to commit'), findsOneWidget);
    expect(find.text('b.txt'), findsWidgets);
    expect(tester.widget<TextField>(find.byKey(const ValueKey('publish message'))).controller!.text, isEmpty);
  });

  testWidgets('with every file chosen no list of files is sent, so the host takes all that changed', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0), ('b.txt', 2, 1)]);
    await tester.enterText(find.byKey(const ValueKey('publish message')), 'All of it');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('publish commit')));
    await tester.pumpAndSettle();
    expect(git.commits.single.$2, 'All of it');
    expect(git.commits.single.$3, isNull);
  });

  testWidgets('a refused commit shows git\'s own words and leaves the message', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0)]);
    git.failures['commit'] = 'git failed: Author identity unknown';
    await tester.enterText(find.byKey(const ValueKey('publish message')), 'Keep me');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('publish commit')));
    await tester.pumpAndSettle();
    expect(_result(tester), 'git failed: Author identity unknown');
    expect(tester.widget<TextField>(find.byKey(const ValueKey('publish message'))).controller!.text, 'Keep me');
  });

  testWidgets('a push says what git says, and can set the upstream on a remote of the person\'s choosing', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0)]);
    git.failures['push'] = 'git failed: fatal: The current branch main has no upstream branch.';
    await tester.tap(find.byKey(const ValueKey('publish push')));
    await tester.pumpAndSettle();
    expect(_result(tester), contains('has no upstream branch'));
    expect(git.pushes, [('/work/proj', null, false)]);
    await tester.tap(find.byKey(const ValueKey('publish set upstream')));
    await tester.enterText(find.byKey(const ValueKey('publish remote')), 'fork');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('publish push')));
    await tester.pumpAndSettle();
    expect(git.pushes.last, ('/work/proj', 'fork', true));
    expect(_result(tester), 'Pushed main to fork and set it as the upstream.');
  });

  testWidgets('a pull request made by gh shows its address', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0)]);
    await tester.tap(find.byKey(const ValueKey('publish pull request')));
    await tester.pumpAndSettle();
    expect(tester.widget<FilledButton>(find.byKey(const ValueKey('pr create'))).onPressed, isNull, reason: 'a title is needed');
    await tester.enterText(find.byKey(const ValueKey('pr title')), 'Fix the sum');
    await tester.enterText(find.byKey(const ValueKey('pr body')), 'Because.');
    await tester.enterText(find.byKey(const ValueKey('pr base')), 'develop');
    await tester.tap(find.byKey(const ValueKey('pr draft')));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('pr create')));
    await tester.pumpAndSettle();
    expect(git.requests, [('/work/proj', 'Fix the sum', 'Because.', 'develop', true)]);
    expect(_result(tester), 'Pull request opened: https://forge.invalid/acme/widgets/pull/7');
  });

  testWidgets('with no gh the forge\'s page is opened in the browser, and gh\'s refusal is shown when it is there', (tester) async {
    final git = await _open(tester, [('a.txt', 1, 0)]);
    final before = links.openLink;
    addTearDown(() => links.openLink = before);
    final browsed = <Uri>[];
    links.openLink = (uri) async {
      browsed.add(uri);
      return true;
    };
    git.opened = const PullRequestOpened(viaGh: false, url: 'https://github.com/acme/widgets/compare/main?expand=1', branch: 'main');
    await tester.tap(find.byKey(const ValueKey('publish pull request')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const ValueKey('pr title')), 'T');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('pr create')));
    await tester.pumpAndSettle();
    expect(browsed, [Uri.parse('https://github.com/acme/widgets/compare/main?expand=1')]);
    expect(_result(tester), startsWith('Opened the forge\'s page'));
    git.failures['pull request'] = 'gh failed: not logged in';
    await tester.tap(find.byKey(const ValueKey('publish pull request')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const ValueKey('pr title')), 'T');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('pr create')));
    await tester.pumpAndSettle();
    expect(_result(tester), 'gh failed: not logged in');
    expect(browsed, hasLength(1));
  });

  testWidgets('a host that only reads the repository shows no publishing', (tester) async {
    final git = FakeGit()..current = gitState([('a.txt', 1, 0)]);
    await showTask(tester, _desktop, git: git);
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('scope Uncommitted')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('open diff')));
    await tester.pumpAndSettle();
    expect(find.byType(GitPublishBar), findsNothing);
  });

  group('the host\'s control link', () {
    test('commit, push and pull request are the host\'s methods, with only what was given', () async {
      final asked = <(String, Map<String, Object?>)>[];
      final git = ControlGit((method, params) async {
        asked.add((method, params));
        return switch (method) {
          '_canoa/worktree/commit' => {'commit': 'c' * 40, 'summary': 'subject', 'branch': 'main'},
          '_canoa/worktree/push' => {'pushed': true, 'remote': 'origin', 'branch': 'main'},
          _ => {'via': 'web', 'url': 'https://x.invalid/compare/main', 'branch': 'main'},
        };
      });
      final made = await git.commit('/w', 'msg', paths: ['a', 'b']);
      expect((made.commit, made.summary, made.branch), ('c' * 40, 'subject', 'main'));
      await git.commit('/w', 'msg');
      await git.push('/w');
      await git.push('/w', remote: 'fork', setUpstream: true);
      final opened = await git.pullRequest('/w', title: 'T');
      expect((opened.viaGh, opened.url), (false, 'https://x.invalid/compare/main'));
      await git.pullRequest('/w', title: 'T', body: 'B', base: 'dev', draft: true);
      expect([for (final (method, params) in asked) '${method.split('/').last} ${jsonEncode(params)}'], [
        'commit {"path":"/w","message":"msg","paths":["a","b"]}',
        'commit {"path":"/w","message":"msg"}',
        'push {"path":"/w"}',
        'push {"path":"/w","remote":"fork","setUpstream":true}',
        'pull_request {"path":"/w","title":"T","body":""}',
        'pull_request {"path":"/w","title":"T","body":"B","base":"dev","draft":true}',
      ]);
    });

    test('an answer that is not the host\'s is a failure, not a crash', () async {
      final git = ControlGit((method, params) async => {'unexpected': true});
      await expectLater(git.commit('/w', 'm'), throwsA(isA<WorktreeFailure>()));
      await expectLater(git.push('/w'), throwsA(isA<WorktreeFailure>()));
      await expectLater(git.pullRequest('/w', title: 'T'), throwsA(isA<WorktreeFailure>()));
    });
  });
}
