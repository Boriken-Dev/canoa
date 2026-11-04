// Every file the session's tools changed, by file, from what the agent sent: the shapes are the coding task's, Claude's v2 diff
// as test/fixtures/coding_task_updates.jsonl records it and the scripted stand-in's v1 diffs.
import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory, File;

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/chat/changes.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/foundation.dart' show Uint8List;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Tolerant extends LocalFileComparator {
  _Tolerant(super.testFile);

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final result = await GoldenFileComparator.compareLists(imageBytes, await getGoldenBytes(golden));
    if (!result.passed && result.diffPercent > 0.015) fail(await generateFailureOutput(result, golden, basedir));
    result.dispose();
    return true;
  }
}

final class _Agent {
  final incoming = StreamController<String>();

  Future<ChatSession> start(SessionStore store) async {
    final client = VerifiedAgent.fromChild(
      label: 'Coder',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
          'initialize' => {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}},
          'session/new' => {'sessionId': 's1'},
          _ => <String, Object?>{},
        }}));
      },
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Fix the failing test', agent: 'Coder', peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    return session;
  }

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body},
  }));

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

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump();
}

Future<(_Agent, ChatSession, SessionStore)> _shown(WidgetTester tester, Size size) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore(includePreview: false);
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store));
  agent.task();
  await _settle(tester);
  return (agent, session, store);
}

void main() {
  setUpAll(() => goldenFileComparator = _Tolerant(Uri.file('${Directory.current.path}/test/changes_test.dart')));

  test('the fixture records the v2 diff shape these tests send', () {
    final shapes = File('test/fixtures/coding_task_updates.jsonl').readAsLinesSync().where((line) => line.contains('"patch"'));
    expect(shapes.single, contains('"changes":[{"operation":"string","path":"string","fileType":"string"}]'));
  });

  testWidgets('each file once, at its latest, from the first text the agent showed to the last; what failed is left out', (tester) async {
    final (_, session, _) = await _shown(tester, const Size(1280, 800));
    final files = changesOf(session);
    expect([for (final file in files) file.path], ['src/calc.js', 'README.md', 'src/report.js']);
    final calc = files.first;
    expect(calc.before, contains('a - b'));
    expect(calc.after, contains('function sum'));
    expect((calc.added, calc.removed), (2, 2));
    expect(files[1].before, isNull, reason: 'a new file');
    expect(files[2].patches, hasLength(1));
    expect((files[2].added, files[2].removed), (1, 1));
  });

  testWidgets('a file marked viewed stays so until it changes again', (tester) async {
    final (agent, session, store) = await _shown(tester, const Size(1280, 800));
    await tester.tap(find.byTooltip('Changes'));
    await tester.pumpAndSettle();
    expect(find.byType(ChangesPane), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('viewed README.md')));
    await tester.pump();
    expect(isViewed(session, changesOf(session)[1]), isTrue);
    expect(find.text('1 of 3 viewed'), findsOneWidget);
    agent.tool('readme2', 'completed', [{'type': 'diff', 'path': 'README.md', 'oldText': '# tiny\n', 'newText': '# small\n'}]);
    await _settle(tester);
    expect(isViewed(session, changesOf(session)[1]), isFalse);
    expect(store.selected, session);
  });

  testWidgets('a comment on a line goes to the composer with the file and the line', (tester) async {
    await _shown(tester, const Size(1280, 800));
    await tester.tap(find.byTooltip('Changes'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('line src/calc.js new 1')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('change-comment')), 'Keep the old name as an alias.');
    await tester.tap(find.text('Add to message'));
    await tester.pumpAndSettle();
    final composer = tester.widget<TextField>(find.byWidgetPredicate((widget) => widget is TextField && widget.decoration?.hintText == 'Message agent'));
    expect(composer.controller!.text, 'src/calc.js:1: `export function sum(a, b) {`\nKeep the old name as an alias.\n');
  });

  testWidgets('a change cut short says so', (tester) async {
    final (agent, session, _) = await _shown(tester, const Size(1280, 800));
    agent.tool('big', 'completed', [{'type': 'diff', 'path': 'big.txt', 'newText': 'x' * (maxToolText + 10)}]);
    await _settle(tester);
    expect(changesOf(session).last.cut, isTrue);
    await tester.tap(find.byTooltip('Changes'));
    await tester.pumpAndSettle();
    expect(find.textContaining('cut short'), findsOneWidget);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the pane draws at $label width with nothing overflowing', (tester) async {
      await _shown(tester, size);
      if (label == 'phone') {
        await tester.tap(find.byTooltip('More'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Changes'));
      } else {
        await tester.tap(find.byTooltip('Changes'));
      }
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      expect(find.byType(ChangesPane), findsOneWidget);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/changes_$label.png'));
    });
  }

}
