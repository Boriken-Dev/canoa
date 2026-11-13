// An @ in the composer names files in the session's folder, which go to the agent as links; the placeholder names the inputs, the
// / list has the agent's commands and Canoa's own actions in two groups, and + offers Slash commands.
import 'dart:io';

import 'package:canoa/acp/local_agent.dart' show listFolderEntries, resolveInside;
import 'package:canoa/chat/agent_events.dart' show AgentCommandInfo;
import 'package:canoa/chat/mentions.dart' show rankMentions;
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A folder on disk with files in it, and one file beside it that is not in it.
final class _Tree {
  _Tree() : root = Directory.systemTemp.createTempSync('canoa-mentions') {
    folder = Directory('${root.path}${Platform.pathSeparator}work')..createSync();
    write('work/notes.txt', 'hello notes');
    write('work/src/main.dart', 'void main() {}');
    write('work/src/util.dart', 'int one() => 1;');
    write('outside.txt', 'not in the folder');
  }
  final Directory root;
  late final Directory folder;

  void write(String relative, String text) {
    final file = File('${root.path}${Platform.pathSeparator}${relative.replaceAll('/', Platform.pathSeparator)}');
    file.parent.createSync(recursive: true);
    file.writeAsStringSync(text);
  }

  String get outside => '${root.path}${Platform.pathSeparator}outside.txt';
}

Future<(SessionStore, OptionsAgent, _Tree)> _started(WidgetTester tester, {Size size = const Size(1280, 800)}) async {
  final tree = _Tree();
  addTearDown(() => tree.root.deleteSync(recursive: true));
  final agent = OptionsAgent();
  final store = SessionStore();
  await pumpApp(tester, store, linked: agent.openLink, size: size, pickFolder: () async => tree.folder.path);
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
  await send(tester, 'Say hello');
  return (store, agent, tree);
}

Finder get _field => find.byType(TextField).last;
String? _hint(WidgetTester tester) => tester.widget<TextField>(_field).decoration?.hintText;
Finder _mention(String label) => find.byKey(ValueKey('mention $label'));

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(_field, text);
  await settle(tester);
}

List<dynamic> _lastPrompt(OptionsAgent agent) => agent.last('session/prompt')['params']['prompt'] as List<dynamic>;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/mentions_test.dart')));

  group('what @ offers', () {
    test('entries are ranked by name first, then path, then letters in order, and none is outside the query', () {
      const entries = ['README.md', 'src/', 'src/main.dart', 'src/domain.dart', 'docs/guide/main.md'];
      expect(rankMentions(entries, 'main', limit: 5), ['src/main.dart', 'docs/guide/main.md', 'src/domain.dart'], reason: 'a name that holds it comes last');
      expect(rankMentions(entries, 'sr', limit: 5).first, 'src/');
      expect(rankMentions(entries, 'smd', limit: 5), contains('src/main.dart'));
      expect(rankMentions(entries, 'zzz', limit: 5), isEmpty);
      expect(rankMentions(entries, '', limit: 2), ['README.md', 'src/'], reason: 'with nothing typed, the top level');
      expect(rankMentions(entries, '../x', limit: 5), isEmpty);
    });

    test('outside a repository the folder is walked, with no list of names left out and a bound on count and depth', () async {
      final tree = _Tree();
      addTearDown(() => tree.root.deleteSync(recursive: true));
      tree.write('work/node_modules/dep/index.js', 'x');
      tree.write('work/.hidden/note.txt', 'x');
      tree.write('work/a/b/c/d/e/deep.txt', 'x');
      final entries = await listFolderEntries(tree.folder.path);
      expect(entries, containsAll(['notes.txt', 'src/', 'src/main.dart', 'src/util.dart', 'node_modules/dep/index.js', '.hidden/note.txt']));
      expect(entries.where((entry) => entry.contains('outside')), isEmpty);
      final shallow = await listFolderEntries(tree.folder.path, maxDepth: 2);
      expect(shallow, contains('a/b/c/'));
      expect(shallow, isNot(contains('a/b/c/d/e/')), reason: 'deeper than the bound');
      expect((await listFolderEntries(tree.folder.path, maxEntries: 4)).length, lessThanOrEqualTo(4));
    });

    test('in a git repository the files are git\'s: tracked and untracked, never what an ignore rule covers', () async {
      final tree = _Tree();
      addTearDown(() => tree.root.deleteSync(recursive: true));
      final folder = tree.folder.path;
      Future<bool> git(List<String> args) async {
        try {
          return (await Process.run('git', ['-C', folder, ...args])).exitCode == 0;
        } on ProcessException {
          return false;
        }
      }

      if (!await git(['init', '-q'])) {
        markTestSkipped('git is not here');
        return;
      }
      tree.write('work/.gitignore', 'dist/\n*.log\n');
      tree.write('work/node_modules/dep/index.js', 'ignored by name only if the repository says so');
      tree.write('work/dist/out.js', 'x');
      tree.write('work/build.log', 'x');
      tree.write('work/.github/workflows/ci.yml', 'x');
      await git(['add', 'notes.txt']);
      final entries = await listFolderEntries(folder);
      expect(entries, containsAll(['notes.txt', 'src/main.dart', 'src/', '.gitignore', '.github/workflows/ci.yml', 'node_modules/dep/index.js']),
          reason: 'tracked and untracked, a dot name included, and a folder no rule ignores');
      expect(entries.where((entry) => entry.startsWith('dist') || entry.endsWith('.log') || entry.startsWith('.git/')), isEmpty);
      tree.write('work/.gitignore', 'dist/\n*.log\nnode_modules/\n');
      expect((await listFolderEntries(folder)).where((entry) => entry.contains('node_modules')), isEmpty, reason: 'the repository\'s rule, read from it');
    });

    test('a path outside the folder is refused, however it is spelled', () async {
      final tree = _Tree();
      addTearDown(() => tree.root.deleteSync(recursive: true));
      final folder = tree.folder.path;
      expect(resolveInside(folder, 'src/main.dart'), isNotNull);
      expect(resolveInside(folder, '../outside.txt'), isNull);
      expect(resolveInside(folder, 'src/../../outside.txt'), isNull);
      expect(resolveInside(folder, tree.outside), isNull, reason: 'an absolute path');
      expect(resolveInside(folder, 'missing.txt'), isNull);
      expect(resolveInside(folder, ''), isNull);
    });

    test('a link in the folder to a file outside it is refused', () async {
      final tree = _Tree();
      addTearDown(() => tree.root.deleteSync(recursive: true));
      final link = Link('${tree.folder.path}${Platform.pathSeparator}escape.txt');
      try {
        link.createSync(tree.outside);
      } on FileSystemException {
        markTestSkipped('this machine does not let a test make a link');
        return;
      }
      expect(resolveInside(tree.folder.path, 'escape.txt'), isNull);
      expect(await listFolderEntries(tree.folder.path), isNot(contains('escape.txt')));
    });
  });

  testWidgets('the placeholder names the inputs', (tester) async {
    await _started(tester);
    expect(_hint(tester), 'Message agent: @ files, / commands');
    await finish(tester);
  });

  testWidgets('on a phone the placeholder is one line, and a draft\'s names its inputs too', (tester) async {
    await _started(tester, size: const Size(390, 844));
    final hint = _hint(tester)!;
    expect(hint, 'Message agent: @ files, / commands');
    expect(tester.getSize(find.text(hint)).height, lessThan(30));
    await newSession(tester);
    final draft = _hint(tester)!;
    expect(draft, 'Message Scripted: / commands', reason: 'no folder is chosen yet, so no @');
    expect(draft, isNot(contains('it starts when you send')), reason: 'the short form');
    expect(tester.getSize(find.text(draft)).height, lessThan(30));
    await finish(tester);
  });

  testWidgets('an @ lists the folder\'s files, a pick writes the mention and goes as a link with no contents', (tester) async {
    final (store, agent, tree) = await _started(tester);
    await _type(tester, 'read @ma');
    await until(tester, () => _mention('src/main.dart').evaluate().isNotEmpty, 'the list');
    expect(_mention('src/util.dart'), findsNothing);
    expect(find.textContaining('.git'), findsNothing);
    await tester.tap(_mention('src/main.dart'));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'read @src/main.dart ');
    expect(find.byKey(const ValueKey('attachment src/main.dart')), findsOneWidget);
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'the agent to have it');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource_link']);
    expect(prompt[1]['uri'], Uri.file('${tree.folder.path}${Platform.pathSeparator}src${Platform.pathSeparator}main.dart').toString());
    expect(prompt[1]['name'], 'src/main.dart');
    expect(store.selected!.attachments, isEmpty);
    await finish(tester);
  });

  testWidgets('a mention whose text was deleted does not go', (tester) async {
    final (_, agent, _) = await _started(tester);
    await _type(tester, '@notes');
    await until(tester, () => _mention('notes.txt').evaluate().isNotEmpty, 'the list');
    await tester.tap(_mention('notes.txt'));
    await settle(tester);
    await _type(tester, 'never mind');
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'the agent to have it');
    expect(_lastPrompt(agent).map((block) => block['type']), ['text']);
    await finish(tester);
  });

  testWidgets('typing a path out of the folder lists nothing, and picking one that is not in it is refused', (tester) async {
    final (store, _, _) = await _started(tester);
    await _type(tester, '@../outside');
    await settle(tester, 100);
    expect(find.byKey(const Key('mention-suggestions')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('mention-suggestions')), matching: find.byType(ListTile)), findsNothing);
    expect(store.selected!.attachments, isEmpty);
    await finish(tester);
  });

  testWidgets('/ lists the agent\'s commands and Canoa\'s actions in two groups, and an action sends nothing', (tester) async {
    final (store, agent, _) = await _started(tester);
    store.selected!.commands = const [AgentCommandInfo('compact', 'Make room')];
    store.markViewed(store.selected!, 'x', null);
    await tester.pump();
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await _type(tester, '/');
    expect(find.text('Agent commands'), findsOneWidget);
    expect(find.text('Canoa actions'), findsOneWidget);
    expect(find.text('/compact'), findsOneWidget);
    expect(find.byKey(const ValueKey('action mode')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('action mode')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, isEmpty);
    expect(agent.methods.where((method) => method == 'session/prompt').length, before);
    await finish(tester);
  });

  testWidgets('+ has Slash commands, which opens the list', (tester) async {
    final (store, _, _) = await _started(tester);
    store.selected!.commands = const [AgentCommandInfo('compact', 'Make room')];
    store.markViewed(store.selected!, 'x', null);
    await tester.pump();
    await tester.tap(find.byKey(const Key('composer-plus')));
    await frames(tester);
    await tester.tap(find.text('Slash commands'));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, '/');
    expect(find.text('/compact'), findsOneWidget);
    expect(find.text('Canoa actions'), findsOneWidget);
    await finish(tester);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the @ list draws at $label width', (tester) async {
      await _started(tester, size: size);
      await _type(tester, 'read @');
      await until(tester, () => _mention('notes.txt').evaluate().isNotEmpty, 'the list');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mentions_$label.png'));
      await finish(tester);
    });

    testWidgets('the / list in two groups draws at $label width', (tester) async {
      final (store, _, _) = await _started(tester, size: size);
      store.selected!.commands = const [AgentCommandInfo('compact', 'Make room'), AgentCommandInfo('review', 'Review the changes')];
      store.markViewed(store.selected!, 'x', null);
      await tester.pump();
      await _type(tester, '/');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/slash_groups_$label.png'));
      await finish(tester);
    });
  }
}
