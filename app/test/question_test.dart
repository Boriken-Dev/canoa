// An agent's question as a form: built from its schema, answered, declined or cancelled, its text shown as data, and needing the
// person like a permission request does. In memory, shaped as Claude's adapter asks, and against the scripted stand-in.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/chat/question_card.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  Map<String, dynamic>? initialized;

  Future<ChatSession> start(SessionStore store) async {
    final client = VerifiedAgent.fromChild(
      label: 'Claude',
      messages: incoming.stream,
      decider: store.askPermission,
      elicitor: store.askQuestion,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        if (message['method'] == 'initialize') initialized = message['params'] as Map<String, dynamic>;
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
          'initialize' => {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}},
          'session/new' => {'sessionId': 's1'},
          _ => <String, Object?>{},
        }}));
      },
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Plan', agent: 'Claude', peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    return session;
  }

  void ask(int id, Map<String, Object?> properties, {String message = 'Which runner should the tests use?', List<String> required = const []}) =>
      incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': id, 'method': 'elicitation/create',
        'params': {
          'mode': 'form', 'sessionId': 's1', 'toolCallId': 'ask', 'message': message,
          'requestedSchema': {'type': 'object', 'properties': properties, 'required': required},
        },
      }));

  /// As Claude's adapter shapes AskUserQuestion: a choice with a header, and "Other" beside it.
  void askRunner(int id) => ask(id, {
    'q0': {'type': 'string', 'title': 'Runner', 'oneOf': [
      {'const': 'node', 'title': 'node --test', 'description': 'The runner package.json names'},
      {'const': 'npm', 'title': 'npm test'},
    ]},
    'q0_custom': {'type': 'string', 'title': 'Other', 'description': 'Type your own answer (optional).'},
  });

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump();
}

Future<(_Agent, ChatSession, SessionStore)> _shown(WidgetTester tester, {Size size = const Size(1100, 900)}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore(includePreview: false);
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store));
  await tester.pump();
  return (agent, session, store);
}

void main() {
  testWidgets('a question shaped as Claude asks one is a form, and the answer chosen reaches the agent', (tester) async {
    final (agent, session, store) = await _shown(tester);
    expect((agent.initialized?['capabilities'] as Map)['elicitation'], {'form': <String, Object?>{}});
    final asked = <AttentionKind>[];
    store.attention.listen((needs) => asked.add(needs.$2));
    agent.askRunner(3);
    await _settle(tester);
    expect(find.byType(QuestionCard), findsOneWidget);
    expect(find.text('Which runner should the tests use?'), findsOneWidget);
    expect(asked, [AttentionKind.question]);
    expect(session.needsYou, isTrue);
    await tester.tap(find.text('npm test'));
    await tester.pump();
    await tester.enterText(find.byKey(const ValueKey('question-q0_custom')), 'with --watch off');
    await tester.pump();
    await tester.tap(find.text('Send answer'));
    await _settle(tester);
    expect(agent.answerTo(3)?['result'], {'action': 'accept', 'content': {'q0': 'npm', 'q0_custom': 'with --watch off'}});
    expect(find.byType(QuestionCard), findsNothing);
  });

  for (final (button, action) in [('Decline', 'decline'), ('Cancel', 'cancel')]) {
    testWidgets('$button answers $action', (tester) async {
      final (agent, _, _) = await _shown(tester);
      agent.askRunner(4);
      await _settle(tester);
      await tester.tap(find.descendant(of: find.byType(QuestionCard), matching: find.text(button)));
      await _settle(tester);
      expect(agent.answerTo(4)?['result'], {'action': action});
    });
  }

  testWidgets('every kind of field draws, and an answer that does not fit cannot be sent', (tester) async {
    final (agent, _, _) = await _shown(tester, size: const Size(390, 1400));
    agent.ask(5, {
      'name': {'type': 'string', 'title': 'Name', 'minLength': 2},
      'count': {'type': 'integer', 'title': 'How many', 'minimum': 1, 'maximum': 3, 'default': 2},
      'ok': {'type': 'boolean', 'title': 'Go ahead', 'default': true},
      'tags': {'type': 'array', 'title': 'Tags', 'maxItems': 1, 'items': {'anyOf': [{'const': 'a', 'title': 'A'}, {'const': 'b', 'title': 'B'}]}},
    }, required: ['name']);
    await _settle(tester);
    expect(tester.takeException(), isNull);
    FilledButton send() => tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Send answer'));
    expect(send().onPressed, isNull, reason: 'the name is required');
    await tester.enterText(find.byKey(const ValueKey('question-name')), 'A');
    await tester.pump();
    expect(send().onPressed, isNull, reason: 'it needs two characters');
    await tester.enterText(find.byKey(const ValueKey('question-name')), 'Ana');
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pump();
    await tester.ensureVisible(find.text('A'));
    await tester.tap(find.text('A'));
    await tester.tap(find.text('B'));
    await tester.pump();
    expect(send().onPressed, isNull, reason: 'one tag at most');
    await tester.ensureVisible(find.text('A'));
    await tester.tap(find.text('A'));
    await tester.pump();
    await tester.ensureVisible(find.text('Send answer'));
    await tester.pump();
    await tester.tap(find.text('Send answer'));
    await _settle(tester);
    expect(agent.answerTo(5)?['result'], {'action': 'accept', 'content': {'name': 'Ana', 'count': 2, 'ok': true, 'tags': ['b']}});
  });

  testWidgets('a required question of a kind Canoa cannot show leaves only Decline and Cancel', (tester) async {
    final (agent, _, _) = await _shown(tester);
    agent.ask(6, {'map': {'type': '_vendor_map', 'title': 'Where'}}, required: ['map']);
    await _settle(tester);
    expect(find.textContaining('cannot show'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Send answer')).onPressed, isNull);
  });

  testWidgets('what the agent wrote is shown as text, with nothing that could disguise it', (tester) async {
    final (agent, _, _) = await _shown(tester);
    agent.ask(7, {'q': {'type': 'boolean', 'title': 'Sure?'}}, message: 'Run [this](https://evil.example) \u202Eexe.txt now');
    await _settle(tester);
    expect(find.text('Run [this](https://evil.example)  exe.txt now'), findsOneWidget);
  });

  testWidgets('Stop answers a waiting question cancelled', (tester) async {
    final (agent, session, store) = await _shown(tester);
    agent.incoming.add(jsonEncode({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': {'sessionUpdate': 'state_update', 'state': 'running'}}}));
    agent.askRunner(8);
    await _settle(tester);
    await tester.runAsync(store.cancelActiveSession);
    await _settle(tester);
    expect(agent.answerTo(8)?['result'], {'action': 'cancel'});
    expect(session.pendingQuestions, isEmpty);
  });

  final node = Platform.environment['CANOA_TEST_NODE'] ?? 'node';
  bool runs() {
    try {
      return Process.runSync(node, ['--version'], runInShell: true).exitCode == 0;
    } on ProcessException {
      return false;
    }
  }

  test('the scripted agent\'s question, answered through the store as the form answers it', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-ask-');
    addTearDown(() async {
      for (var tries = 0; ; tries++) {
        try {
          return dir.deleteSync(recursive: true);
        } on FileSystemException {
          if (tries > 100) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
      }
    });
    final store = SessionStore(includePreview: false);
    final client = await startLocalAgent(
      AgentCommand(label: 'Scripted', program: node, args: ['${Directory.current.path}/integration_test/scripted_agent.mjs', '--state=${dir.path}/s.json']),
      decider: store.askPermission,
      elicitor: store.askQuestion,
      workingDirectory: dir.path,
    );
    addTearDown(client.close);
    final opened = await client.openSession(cwd: dir.path);
    final session = store.addProviderSession(title: 'S', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    Future<void> until(bool Function() done) async {
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out');
        await Future<void>.delayed(const Duration(milliseconds: 20));
      }
    }

    final first = store.sendText('Ask Which runner?');
    await until(() => session.pendingQuestions.isNotEmpty);
    final request = session.pendingQuestions.single.request;
    expect(request.message, 'Which runner?');
    expect([for (final field in request.fields) field.name], ['q0', 'q0_custom']);
    store.answerQuestion(session, const AcpElicitAccept({'q0': 'node'}));
    await first;
    expect(session.messages.last.text, 'You chose node.');

    final second = store.sendText('Ask Which runner, again?');
    await until(() => session.pendingQuestions.isNotEmpty);
    store.answerQuestion(session, const AcpElicitDecline());
    await second;
    expect(session.messages.last.text, 'You declined to answer.');
  }, skip: runs() ? false : 'needs node', timeout: const Timeout(Duration(minutes: 1)));
}
