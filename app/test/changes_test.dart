// Every file the session's tools changed, by file, from what the agent sent: the shapes are the coding task's, Claude's v2 diff
// as test/fixtures/coding_task_updates.jsonl records it and the scripted stand-in's v1 diffs.
import 'dart:io' show Directory, File;

import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:canoa/chat/session_views.dart';
import 'fixtures/task_agent.dart';

Future<void> _settle(WidgetTester tester) => settle(tester);

Future<(TaskAgent, ChatSession, SessionStore)> _shown(WidgetTester tester, Size size) async {
  final shown = await showTask(tester, size);
  shown.$1.task();
  await settle(tester);
  return shown;
}

/// The changes pane the inspector opens from the header's chip.
Future<void> _openChanges(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Changes'));
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(const ValueKey('open diff')));
  await tester.pumpAndSettle();
}

bool isViewed(ChatSession session, FileChange file) => session.viewedChanges[file.path] == file.version;

void main() {
  setUpAll(() => goldenFileComparator = TolerantComparator(Uri.file('${Directory.current.path}/test/changes_test.dart')));

  test('the fixture records the v2 diff shape these tests send', () {
    final shapes = File('test/fixtures/coding_task_updates.jsonl').readAsLinesSync().where((line) => line.contains('"patch"'));
    expect(shapes.single, contains('"changes":[{"operation":"string","path":"string","fileType":"string"}]'));
  });

  testWidgets('each file once, at its latest, from the first text the agent showed to the last; what failed is left out', (tester) async {
    final (_, session, _) = await _shown(tester, const Size(1280, 800));
    final files = sessionChanges(session);
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
    await _openChanges(tester);
    expect(find.byType(ChangesPane), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('viewed README.md')));
    await tester.pump();
    expect(isViewed(session, sessionChanges(session)[1]), isTrue);
    expect(find.text('1 of 3 viewed'), findsOneWidget);
    agent.tool('readme2', 'completed', [{'type': 'diff', 'path': 'README.md', 'oldText': '# tiny\n', 'newText': '# small\n'}]);
    await _settle(tester);
    expect(isViewed(session, sessionChanges(session)[1]), isFalse);
    expect(store.selected, session);
  });

  testWidgets('a comment on a line goes to the composer with the file and the line', (tester) async {
    await _shown(tester, const Size(1280, 800));
    await _openChanges(tester);
    await tester.tap(find.byKey(const ValueKey('line src/calc.js new 1')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('change-comment')), 'Keep the old name as an alias.');
    await tester.tap(find.text('Add to message'));
    await tester.pumpAndSettle();
    final composer = tester.widget<TextField>(find.byWidgetPredicate((widget) => widget is TextField && (widget.decoration?.hintText ?? '').startsWith('Message agent')));
    expect(composer.controller!.text, 'src/calc.js:1: `export function sum(a, b) {`\nKeep the old name as an alias.\n');
  });

  testWidgets('a change cut short says so', (tester) async {
    final (agent, session, _) = await _shown(tester, const Size(1280, 800));
    agent.tool('big', 'completed', [{'type': 'diff', 'path': 'big.txt', 'newText': 'x' * (chatLimits.toolText + 10)}]);
    await _settle(tester);
    expect(sessionChanges(session).last.cut, isTrue);
    await _openChanges(tester);
    expect(find.textContaining('cut short'), findsOneWidget);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the pane draws at $label width with nothing overflowing', (tester) async {
      await _shown(tester, size);
      await _openChanges(tester);
      expect(tester.takeException(), isNull);
      expect(find.byType(ChangesPane), findsOneWidget);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/changes_$label.png'));
    });
  }

}
