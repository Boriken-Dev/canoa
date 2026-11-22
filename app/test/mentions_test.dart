// An @ in the composer names files in the session's folder, which go to the agent as links; the placeholder names the inputs, the
// / list has the agent's commands and Canoa's own actions in two groups, and + offers Slash commands.
import 'dart:io';

import 'package:canoa/acp/local_agent.dart' show InsideFile;
import 'package:canoa/chat/agent_events.dart' show AgentCommandInfo;
import 'package:canoa/chat/mentions.dart' show rankMentions;
import 'package:canoa/profile/report.dart' show ItemState, ProfileItem, ProfileReport;
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/session_setup.dart';
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

/// What the host lists for an `@`: the files under [folder] as paths relative to it, a folder's ending in a slash.
Future<List<String>> _list(String folder) async {
  final root = Directory(folder);
  int depth(String entry) => '/'.allMatches(entry.endsWith('/') ? entry.substring(0, entry.length - 1) : entry).length;
  return [
    await for (final entry in root.list(recursive: true, followLinks: false))
      '${entry.path.substring(root.path.length + 1).replaceAll(Platform.pathSeparator, '/')}${entry is Directory ? '/' : ''}',
  ]..sort((a, b) => depth(a) != depth(b) ? depth(a).compareTo(depth(b)) : a.compareTo(b));
}

/// What the host answers for a name picked: the file or folder under [folder] it names.
Future<InsideFile?> _resolve(String folder, String relative) async {
  if (relative.split('/').contains('..')) return null;
  final path = '$folder${Platform.pathSeparator}${relative.replaceAll('/', Platform.pathSeparator)}';
  if (await Directory(path).exists()) return (path: path, folder: true, size: 0);
  final file = File(path);
  return await file.exists() ? (path: path, folder: false, size: await file.length()) : null;
}

Future<(SessionStore, OptionsAgent, _Tree)> _started(WidgetTester tester, {Size size = const Size(1280, 800), bool files = true}) async {
  final tree = _Tree();
  addTearDown(() => tree.root.deleteSync(recursive: true));
  final agent = OptionsAgent(files: files);
  final store = SessionStore();
  await pumpApp(tester, store, linked: agent.openLink, size: size, pickFolder: () async => tree.folder.path, listFolder: _list, resolveInside: _resolve, openIn: false);
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
  });

  testWidgets('the placeholder names the agent and the send keys, and the inputs are in the + menu', (tester) async {
    final (store, _, _) = await _started(tester);
    expect(_hint(tester), 'Message ${store.selected!.agent} · Enter to send');
    expect(_hint(tester), isNot(contains('@')), reason: 'the placeholder says nothing of what the field takes');
    await tester.tap(find.byKey(const Key('composer-plus')));
    await frames(tester);
    expect(find.byKey(const Key('plus-mention')), findsOneWidget);
    expect(find.byKey(const Key('plus-slash')), findsOneWidget);
    await tester.tap(find.byKey(const Key('plus-mention')));
    await frames(tester);
    expect(tester.widget<TextField>(_field).controller!.text, '@', reason: 'the entry starts a mention');
    await finish(tester);
  });

  testWidgets('on a phone the placeholder is one line: the agent alone, and a draft names its source', (tester) async {
    final (store, _, _) = await _started(tester, size: const Size(390, 844));
    final hint = _hint(tester)!;
    expect(hint, 'Message ${store.selected!.agent}');
    expect(tester.getSize(find.text(hint)).height, lessThan(30));
    await newSession(tester);
    final draft = _hint(tester)!;
    expect(draft, 'Message Scripted');
    expect(tester.getSize(find.text(draft)).height, lessThan(30));
    await tester.tap(find.byKey(const Key('composer-plus')));
    await frames(tester);
    expect(find.text('Mention a file'), findsNothing, reason: 'no folder is chosen yet, so no file to mention');
    expect(find.text('Mention a conversation'), findsOneWidget, reason: 'the conversation before it can be');
    expect(find.byKey(const Key('plus-slash')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('an @ lists the folder\'s files, a pick writes the mention and, to an agent that takes no files, goes as a link with no contents', (tester) async {
    final (store, agent, tree) = await _started(tester, files: false);
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
    // The conversation takes what was sent from the composer when it has handed it over, a frame after the agent has it.
    await settle(tester);
    expect(store.selected!.attachments, isEmpty);
    await finish(tester);
  });

  testWidgets('to an agent that takes files, a picked text file goes as its contents and a picked folder as a link', (tester) async {
    final (_, agent, tree) = await _started(tester);
    await _type(tester, 'read @ma');
    await until(tester, () => _mention('src/main.dart').evaluate().isNotEmpty, 'the list');
    await tester.tap(_mention('src/main.dart'));
    await settle(tester);
    await _type(tester, 'read @src/main.dart and @sr');
    await until(tester, () => _mention('src/').evaluate().isNotEmpty, 'the list');
    await tester.tap(_mention('src/'));
    await settle(tester);
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'the agent to have it');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource', 'resource_link']);
    expect(prompt[1]['resource']['text'], 'void main() {}');
    expect(prompt[2]['uri'], Uri.directory('${tree.folder.path}${Platform.pathSeparator}src').toString());
    await finish(tester);
  });
  testWidgets('an @ also lists the other conversations under their own heading, and a pick goes as that conversation\'s messages as text', (tester) async {
    final (store, agent, _) = await _started(tester);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await send(tester, 'Second one');
    await _type(tester, 'continue from @Say');
    await until(tester, () => find.byKey(const Key('mention-group Conversations')).evaluate().isNotEmpty, 'the list');
    expect(find.byKey(const ValueKey('mention Conversations Say hello')), findsOneWidget);
    expect(find.byKey(const ValueKey('mention Conversations Second one')), findsNothing, reason: 'not the conversation it is typed in');
    await tester.tap(find.byKey(const ValueKey('mention Conversations Say hello')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'continue from @Say hello ');
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'the agent to have it');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource']);
    expect(prompt[1]['resource']['text'], allOf(startsWith('# Say hello'), contains('User: Say hello')));
    expect(store.selected!.title, isNot('Say hello'));
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
    store.selected!.offerCommands(const [AgentCommandInfo('compact', 'Make room')]);
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

  testWidgets('/ lists every command the agent reports, not a first few', (tester) async {
    final (store, _, _) = await _started(tester);
    store.selected!.offerCommands([for (var i = 0; i < 30; i++) AgentCommandInfo('skill$i', 'Skill number $i')]);
    store.markViewed(store.selected!, 'x', null);
    await tester.pump();
    await _type(tester, '/');
    expect(find.byKey(const ValueKey('command skill29')), findsOneWidget);
    expect(find.byKey(const ValueKey('command skill0')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('/ lists the skills and MCP servers Canoa gave the session under their own headings, once each, and a pick writes a request', (tester) async {
    final (store, agent, _) = await _started(tester);
    store.selected!
      ..offerCommands(const [AgentCommandInfo('review', 'The agent lists this skill itself')])
      ..profile = const ProfileReport('claude-code', [
        ProfileItem('skills.review', 'overlay:team', ItemState.applied, 'a skill folder'),
        ProfileItem('skills.deploy', 'overlay:team', ItemState.applied, 'a skill folder'),
        ProfileItem('skills.refused', 'overlay:team', ItemState.refused, 'not allowed'),
        ProfileItem('mcp.github', 'overlay:team', ItemState.applied, 'mcpServers'),
        ProfileItem('model', 'yours', ItemState.applied, 'set'),
      ]);
    store.markViewed(store.selected!, 'x', null);
    await tester.pump();
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await _type(tester, '/');
    expect(find.text('Skills given to this session'), findsOneWidget);
    expect(find.text('MCP servers given to this session'), findsOneWidget);
    expect(find.byKey(const ValueKey('action Skills given to this session deploy')), findsOneWidget);
    expect(find.byKey(const ValueKey('action Skills given to this session review')), findsNothing, reason: 'the agent lists it as a command');
    expect(find.byKey(const ValueKey('action Skills given to this session refused')), findsNothing, reason: 'a skill the agent was not given');
    expect(find.byKey(const ValueKey('command review')), findsOneWidget);
    await tester.ensureVisible(find.byKey(const ValueKey('action MCP servers given to this session github')));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('action MCP servers given to this session github')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'Use the github MCP server to ');
    expect(agent.methods.where((method) => method == 'session/prompt').length, before, reason: 'nothing is sent until the person sends it');
    await finish(tester);
  });
  testWidgets('+ has Slash commands, which opens the list', (tester) async {
    final (store, _, _) = await _started(tester);
    store.selected!.offerCommands(const [AgentCommandInfo('compact', 'Make room')]);
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

  testWidgets('/ lists MCP tools under their server from stand-in host answer, and picking writes the prompt without sending', (tester) async {
    final (store, agent, _) = await _started(tester);
    agent.onMcpList = (params) => switch (params['cursor']) {
      null => {
        'tools': [
          {'server': 'probe', 'name': 'marker', 'description': 'Returns the marker word.'},
          {'server': 'probe', 'name': 'probe', 'description': 'Checks probe settings.'},
        ],
        'resources': [
          {'server': 'probe', 'uri': 'canoa://probe/readme', 'name': 'readme', 'description': 'Probe readme.', 'mimeType': 'text/plain'},
        ],
        'servers': [
          {'name': 'probe', 'status': 'ok', 'tools': 2, 'resources': 1},
          {'name': 'docs', 'status': 'ok', 'tools': 1, 'resources': 0},
        ],
        'nextCursor': 'page2',
      },
      'page2' => {
        'tools': [
          {'server': 'docs', 'name': 'search_docs', 'description': 'Search docs.'},
        ],
        'resources': <Object?>[],
      },
      _ => <String, Object?>{},
    };

    final loaded = await store.refreshMcpInventory(store.selected!, force: true);
    expect(loaded?.tools.map((t) => '${t.server}:${t.name}'), ['probe:marker', 'probe:probe', 'docs:search_docs']);
    expect(loaded?.resources.single.uri, 'canoa://probe/readme');
    await tester.pump();

    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await _type(tester, '/');
    expect(find.byKey(const Key('group Tools · probe')), findsOneWidget);
    expect(find.byKey(const Key('group Tools · docs')), findsOneWidget);
    expect(find.byKey(const ValueKey('action Tools · probe marker')), findsOneWidget);
    expect(find.byKey(const ValueKey('action Tools · docs search_docs')), findsOneWidget);

    await tester.ensureVisible(find.byKey(const ValueKey('action Tools · probe marker')));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('action Tools · probe marker')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'Use the marker tool of probe to ');
    expect(agent.methods.where((method) => method == 'session/prompt').length, before);
    await finish(tester);
  });

  testWidgets('@ lists MCP resources (sent as resource_link) and session subagents (naming the task)', (tester) async {
    final (store, agent, _) = await _started(tester);
    final session = store.selected!
      ..mcpInventory = const McpInventory(
        tools: [McpTool(server: 'probe', name: 'marker')],
        resources: [McpResource(server: 'probe', uri: 'canoa://probe/readme', name: 'readme', mimeType: 'text/plain')],
        servers: [McpServerStatus(name: 'probe', ok: true, tools: 1, resources: 1)],
      );
    session.tasks.apply({
      'sessionId': 'o1',
      'taskId': 't1',
      'event': {
        'task': {
          'id': 't1',
          'status': {'state': 'TASK_STATE_WORKING'},
          'metadata': {'title': 'Audit tests', 'source': 'Codex'},
        },
      },
    });
    store.markViewed(session, 'x', null);
    await tester.pump();

    await _type(tester, 'check @');
    await until(tester, () => find.byKey(const Key('mention-group Resources')).evaluate().isNotEmpty, 'resources group');
    expect(find.byKey(const Key('mention-group Subagents')), findsOneWidget);
    expect(find.byKey(const ValueKey('mention Resources readme')), findsOneWidget);
    expect(find.byKey(const ValueKey('mention Subagents Audit tests')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('mention Resources readme')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'check @readme ');
    expect(find.byKey(const ValueKey('attachment readme')), findsOneWidget);

    await _type(tester, 'check @readme with @Audit');
    await until(tester, () => find.byKey(const ValueKey('mention Subagents Audit tests')).evaluate().isNotEmpty, 'subagent item');
    await tester.tap(find.byKey(const ValueKey('mention Subagents Audit tests')));
    await settle(tester);
    expect(tester.widget<TextField>(_field).controller!.text, 'check @readme with @Audit tests ');

    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'prompt sent');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource_link']);
    expect(prompt[1]['uri'], 'canoa://probe/readme');
    expect(prompt[1]['name'], 'readme');
    expect(prompt[1]['mimeType'], 'text/plain');
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

    testWidgets('the / list with the skills and servers Canoa gave draws at $label width', (tester) async {
      final (store, _, _) = await _started(tester, size: size);
      store.selected!
        ..offerCommands(const [AgentCommandInfo('compact', 'Make room')])
        ..profile = const ProfileReport('claude-code', [
          ProfileItem('skills.deploy', 'overlay:team', ItemState.applied, 'a skill folder'),
          ProfileItem('skills.release-notes', 'overlay:team', ItemState.applied, 'a skill folder'),
          ProfileItem('mcp.github', 'overlay:team', ItemState.applied, 'mcpServers'),
        ]);
      store.markViewed(store.selected!, 'x', null);
      await tester.pump();
      await _type(tester, '/');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/slash_given_$label.png'));
      await finish(tester);
    });

    testWidgets('the / list with MCP server tools draws at $label width', (tester) async {
      final (store, _, _) = await _started(tester, size: size);
      store.selected!
        ..offerCommands(const [AgentCommandInfo('compact', 'Make room')])
        ..mcpInventory = const McpInventory(
          tools: [
            McpTool(server: 'probe', name: 'marker', description: 'Returns the marker word.'),
            McpTool(server: 'github', name: 'list_prs', description: 'List open pull requests.'),
          ],
          servers: [
            McpServerStatus(name: 'probe', ok: true, tools: 1, resources: 0),
            McpServerStatus(name: 'github', ok: true, tools: 1, resources: 0),
          ],
        );
      store.markViewed(store.selected!, 'x', null);
      await tester.pump();
      await _type(tester, '/');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/slash_mcp_tools_$label.png'));
      await finish(tester);
    });

    testWidgets('the @ list with resources, subagents and conversations draws at $label width', (tester) async {
      final (store, _, _) = await _started(tester, size: size);
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await openFolder(tester);
      await send(tester, 'Second one');
      final session = store.selected!
        ..mcpInventory = const McpInventory(
          resources: [McpResource(server: 'probe', uri: 'canoa://probe/readme', name: 'readme', mimeType: 'text/plain')],
        );
      session.tasks.apply({
        'sessionId': 'o1',
        'taskId': 't1',
        'event': {
          'task': {
            'id': 't1',
            'status': {'state': 'TASK_STATE_WORKING'},
            'metadata': {'title': 'Audit tests', 'source': 'Codex'},
          },
        },
      });
      await _type(tester, 'see @');
      await until(tester, () => find.byKey(const Key('mention-group Resources')).evaluate().isNotEmpty, 'resources list');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mention_mcp_and_subagents_$label.png'));
      await finish(tester);
    });

    testWidgets('the @ list with files and conversations draws at $label width', (tester) async {
      await _started(tester, size: size);
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await openFolder(tester);
      await send(tester, 'Second one');
      await _type(tester, 'see @');
      await until(tester, () => find.byKey(const Key('mention-group Conversations')).evaluate().isNotEmpty, 'the list');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/mention_conversations_$label.png'));
      await finish(tester);
    });
    testWidgets('the / list in two groups draws at $label width', (tester) async {
      final (store, _, _) = await _started(tester, size: size);
      store.selected!.offerCommands(const [AgentCommandInfo('compact', 'Make room'), AgentCommandInfo('review', 'Review the changes')]);
      store.markViewed(store.selected!, 'x', null);
      await tester.pump();
      await _type(tester, '/');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/slash_groups_$label.png'));
      await finish(tester);
    });
  }
}
