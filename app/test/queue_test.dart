// Messages written while a turn runs: queued, shown, editable and removable, sent as each turn ends, and held, not dropped,
// when the turn is stopped. Against the scripted stand-in of the coding-task test, as a real child, and the panel in memory.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/chat/queue_panel.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final _node = Platform.environment['CANOA_TEST_NODE'] ?? 'node';

bool _runs(String program) {
  try {
    return Process.runSync(program, ['--version'], runInShell: true).exitCode == 0;
  } on ProcessException {
    return false;
  }
}

/// The scripted agent as a child, with a session open in [store].
Future<(VerifiedAgent, ChatSession)> _scripted(SessionStore store) async {
  final dir = Directory.systemTemp.createTempSync('canoa-queue-');
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
  final client = await startLocalAgent(
    AgentCommand(label: 'Scripted', program: _node, args: ['${Directory.current.path}/integration_test/scripted_agent.mjs', '--state=${dir.path}/s.json']),
    decider: store.askPermission,
    workingDirectory: dir.path,
  );
  addTearDown(client.close);
  final opened = await client.openSession(cwd: dir.path);
  final session = store.addProviderSession(title: 'S', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
  store.attachVerifiedClient(session.id, client, record: false);
  store.select(session.id);
  return (client, session);
}

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

bool _said(ChatSession session, String text) =>
    session.messages.any((message) => message.role == MessageRole.agent && message.text.contains(text));

void _allowAll(SessionStore store, ChatSession session) {
  void allow() {
    if (session.pendingPermissions.isEmpty) return;
    store.choosePermission(session, session.pendingPermissions.first.options.firstWhere((option) => option.kind == 'allow_once'));
  }
  store.addListener(allow);
  addTearDown(() => store.removeListener(allow));
}

void main() {
  final skip = _runs(_node) ? false : 'needs node';

  test('a message written during a turn waits, and is sent when the turn ends', () async {
    final store = SessionStore(includePreview: false);
    final (_, session) = await _scripted(store);
    unawaited(store.sendText('The test fails. Fix it.'));
    await _until(() => session.pendingPermissions.isNotEmpty, 'the agent to ask');
    await store.sendText('Say queued one');
    expect(session.queue.single.text, 'Say queued one');
    expect(_said(session, 'queued one'), isFalse, reason: 'nothing is sent while the turn runs');
    _allowAll(store, session);
    store.choosePermission(session, session.pendingPermissions.first.options.firstWhere((option) => option.kind == 'allow_once'));
    await _until(() => _said(session, 'queued one'), 'the queued message to be answered');
    expect(session.queue, isEmpty);
    final users = [for (final message in session.messages) if (message.role == MessageRole.user) message.text];
    expect(users, ['The test fails. Fix it.', 'Say queued one'], reason: 'in the order written');
  }, skip: skip, timeout: const Timeout(Duration(minutes: 1)));

  test('Stop keeps the queue, held until the person sends it on', () async {
    final store = SessionStore(includePreview: false);
    final (_, session) = await _scripted(store);
    unawaited(store.sendText('Rename addNumbers to sum'));
    await _until(() => session.pendingPermissions.isNotEmpty, 'the agent to ask');
    await store.sendText('Say after the stop');
    await store.sendText('Say later still');
    await store.cancelActiveSession();
    await _until(() => session.status == SessionStatus.connected, 'the turn to stop');
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(session.queue.map((message) => message.text), ['Say after the stop', 'Say later still']);
    expect(session.queueHeld, 'the turn was stopped');
    expect(_said(session, 'after the stop'), isFalse);
    await store.sendQueued(session);
    await _until(() => _said(session, 'later still'), 'the rest of the queue to go on');
    expect(_said(session, 'after the stop'), isTrue);
    expect(session.queue, isEmpty);
    expect(session.queueHeld, isNull);
  }, skip: skip, timeout: const Timeout(Duration(minutes: 1)));

  testWidgets('the queue shows under the conversation, and each message can be edited or removed', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1100, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore(includePreview: false);
    final incoming = StreamController<String>();
    final sent = <Map<String, dynamic>>[];
    final client = VerifiedAgent.fromChild(
      label: 'V2', messages: incoming.stream, decider: store.askPermission, close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
          'initialize' => {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}},
          'session/new' => {'sessionId': 's1'},
          'session/prompt' => {'messageId': 'm${sent.length}'},
          _ => <String, Object?>{},
        }}));
      },
    );
    await tester.runAsync(() async {
      await client.initialize();
      await client.openSession(cwd: '/work');
    });
    final session = store.addProviderSession(title: 'V2', agent: 'V2', peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    void update(Map<String, Object?> body) =>
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body}}));
    await tester.pumpWidget(CanoaApp(store: store));
    update({'sessionUpdate': 'state_update', 'state': 'running'});
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();

    final composer = find.byWidgetPredicate((widget) => widget is TextField && widget.decoration?.hintText == 'Message agent');
    for (final text in ['first', 'second', 'third']) {
      await tester.enterText(composer, text);
      await tester.tap(find.byTooltip('Queue'));
      await tester.pump();
    }
    expect(find.byType(QueuePanel), findsOneWidget);
    expect(find.text('3 queued: sent one at a time, as each turn ends'), findsOneWidget);
    await tester.tap(find.byTooltip('Edit queued message').first);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.enterText(find.byKey(const Key('queued-text')), 'first, edited');
    await tester.tap(find.text('Save'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.tap(find.byTooltip('Remove queued message').at(1));
    await tester.pump();
    expect(session.queue.map((message) => message.text), ['first, edited', 'third']);
    expect(sent.where((message) => message['method'] == 'session/prompt'), isEmpty);

    update({'sessionUpdate': 'state_update', 'state': 'idle', 'stopReason': 'end_turn'});
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pump();
    final prompts = sent.where((message) => message['method'] == 'session/prompt').toList();
    expect(prompts, hasLength(1), reason: 'one per turn');
    expect((prompts.single['params']['prompt'] as List).single['text'], 'first, edited');
    expect(session.queue.single.text, 'third');
    await tester.tap(find.text('Clear queue'));
    await tester.pump();
    expect(session.queue, isEmpty);
    expect(find.byType(QueuePanel), findsNothing);
  });
}
