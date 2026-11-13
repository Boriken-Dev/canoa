// A stand-in agent a test drives by hand and a session on it, for the changes pane and the inspector: it edits files when told,
// reports usage, and can hold a turn open until the test ends it.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/git_changes.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:flutter/foundation.dart' show Uint8List;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A golden that tolerates the antialiasing between machines, and nothing more.
final class TolerantComparator extends LocalFileComparator {
  TolerantComparator(super.testFile);

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final result = await GoldenFileComparator.compareLists(imageBytes, await getGoldenBytes(golden));
    if (!result.passed && result.diffPercent > 0.015) fail(await generateFailureOutput(result, golden, basedir));
    result.dispose();
    return true;
  }
}

/// Git as a test says it is: [read] answers with whatever [current] holds, per base.
final class FakeGit implements GitReader {
  /// What the disk holds against the session's first commit.
  GitState? current;

  /// What it holds against HEAD, when that is not [current].
  GitState? uncommitted;

  /// Fails every read with this.
  Object? failure;

  final reads = <(String, String?)>[];

  @override
  Future<GitState?> read(String folder, {String? base}) async {
    reads.add((folder, base));
    if (failure != null) throw failure!;
    return base == null && uncommitted != null ? uncommitted : current;
  }
}

/// A state of the repository: [files] as (path, added, removed), each with a print that changes with its counts and [revision].
GitState gitState(List<(String, int, int)> files, {String? branch = 'main', String head = 'aaaa', String base = 'aaaa', int revision = 0}) {
  final listed = [
    for (final (path, added, removed) in files)
      GitFile(
        path: path,
        status: 'M',
        added: added,
        removed: removed,
        binary: false,
        patch: '@@ -1,1 +1,${added + 1} @@\n-old $path\n${List.generate(added, (index) => '+new $path $index').join('\n')}\n',
        cut: false,
        print: '$path $added $removed',
      ),
  ];
  return GitState(
    head: head,
    branch: branch,
    base: base,
    added: [for (final file in listed) file.added].fold(0, (a, b) => a + b),
    removed: [for (final file in listed) file.removed].fold(0, (a, b) => a + b),
    files: listed,
    more: false,
  );
}

final class TaskAgent {
  final incoming = StreamController<String>();

  /// A turn's answer waits for [finishTurn] while this is set.
  bool holdTurns = false;
  final _held = <Object>[];

  Future<ChatSession> start(SessionStore store, {String cwd = '/work/proj', SessionWorktree? worktree, SessionPlace? place, Uri? endpoint}) async {
    final client = VerifiedAgent.fromChild(
      label: 'Coder',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        if (message['method'] == 'session/prompt' && holdTurns) {
          _held.add(id as Object);
          return;
        }
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
          'initialize' => {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}},
          'session/new' => {'sessionId': 's1'},
          'session/prompt' => {'stopReason': 'end_turn'},
          _ => <String, Object?>{},
        }}));
      },
    );
    await client.initialize();
    await client.openSession(cwd: cwd);
    final session = store.addProviderSession(
      title: 'Fix the failing test', agent: 'Coder', peerId: client.peerId, remoteSessionId: 's1', cwd: cwd, worktree: worktree,
    );
    if (place != null) session.place = place;
    session.endpoint = endpoint;
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    return session;
  }

  /// Ends the turn a held prompt began.
  void finishTurn() {
    for (final id in _held) {
      incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': {'stopReason': 'end_turn'}}));
    }
    _held.clear();
  }

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body},
  }));

  void usage(int used, int size) => update({'sessionUpdate': 'usage_update', 'used': used, 'size': size});

  void tool(String id, String status, List<Object> content) =>
      update({'sessionUpdate': 'tool_call_update', 'toolCallId': id, 'title': 'Edit', 'kind': 'edit', 'status': status, 'content': content});

  /// The coding task's edits: calc.js twice, a new README line, an edit refused, and a v2 patch of report.js.
  void task() {
    tool('fix', 'completed', [
      {'type': 'diff', 'path': 'src/calc.js', 'oldText': 'export function addNumbers(a, b) {\n  return a - b;\n}\n',
        'newText': 'export function addNumbers(a, b) {\n  return a + b;\n}\n'},
    ]);
    tool('readme', 'completed', [{'type': 'diff', 'path': 'README.md', 'newText': '# tiny\nMaintainer: Ana Rivera\n'}]);
    tool('refused', 'failed', [{'type': 'diff', 'path': 'obsolete.txt', 'oldText': 'kept\n', 'newText': ''}]);
    tool('rename', 'completed', [
      {'type': 'diff', 'path': 'src/calc.js', 'oldText': 'export function addNumbers(a, b) {\n  return a + b;\n}\n',
        'newText': 'export function sum(a, b) {\n  return a + b;\n}\n'},
    ]);
    // Claude's v2 shape, as the fixture records it.
    tool('report', 'completed', [
      {'type': 'diff', 'changes': [{'operation': 'update', 'path': 'src/report.js', 'fileType': 'file'}],
        'patch': {'format': 'unified', 'text': '@@ -1,4 +4,4 @@\n export function total(values) {\n-  return values.reduce((s, v) => addNumbers(s, v), 0);\n+  return values.reduce((s, v) => sum(s, v), 0);\n }'}},
    ]);
  }
}

Future<void> settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  await tester.pump();
}

/// The app showing a session on a [TaskAgent] at [size], with [git] reading its repository when given.
Future<(TaskAgent, ChatSession, SessionStore)> showTask(
  WidgetTester tester,
  Size size, {
  FakeGit? git,
  String cwd = '/work/proj',
  SessionWorktree? worktree,
  SessionPlace? place,
  Uri? endpoint,
  bool holdTurns = false,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  final agent = TaskAgent()..holdTurns = holdTurns;
  final session = await tester.runAsync(() => agent.start(store, cwd: cwd, worktree: worktree, place: place, endpoint: endpoint)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store, gitReader: git == null ? null : (_) => git));
  await settle(tester);
  return (agent, session, store);
}
