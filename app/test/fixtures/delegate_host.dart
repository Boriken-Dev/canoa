// What the delegation tests share: the host's events as the app receives them, and an agent in memory that answers the app's own
// requests to the host the way the host does.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/delegate.dart';
import 'package:canoa/session_store.dart';


Map<String, Object?> taskNotice(String task, Map<String, Object?> event) => {
  'jsonrpc': '2.0',
  'method': delegateEventMethod,
  'params': {'sessionId': 's1', 'taskId': task, 'event': event},
};

Map<String, Object?> taskEvent(String id, String state, {String title = 'Fix the tests', String source = 'codex', String? at}) => {
  'task': {
    'id': id,
    'status': {'state': 'TASK_STATE_$state', 'timestamp': ?at},
    'metadata': {'title': title, 'source': source, 'folder': '/work'},
  },
};

Map<String, Object?> statusEvent(String state, {String? at, String? note}) => {
  'statusUpdate': {
    'status': {
      'state': 'TASK_STATE_$state',
      'timestamp': ?at,
      if (note != null)
        'message': {
          'parts': [
            {'text': note},
          ],
        },
    },
  },
};

const permissionOptions = [
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'allow-always', 'name': 'Yes, for the session', 'kind': 'allow_always'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

/// An agent in memory, speaking v1, that answers the app's own requests to the host the way the host does.
final class DelegateHost {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  /// What the host answers a `history` request with, by task.
  final histories = <String, List<Object?>>{};

  Future<ChatSession> start(SessionStore store, {String label = 'Scripted', bool decide = true}) async {
    client = VerifiedAgent.fromChild(
      label: label,
      messages: incoming.stream,
      decider: decide ? store.askPermission : null,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        switch (message['method']) {
          case 'initialize':
            reply(id, {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
          case 'session/new':
            reply(id, {'sessionId': 's1'});
          case delegateStopMethod:
            reply(id, <String, Object?>{});
          case delegateHistoryMethod:
            reply(id, {'updates': histories[message['params']['taskId']] ?? const []});
          case 'session/prompt' || 'session/cancel' || null:
            break;
          default:
            if (id != null) reply(id, <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: label, agent: label, peerId: client.peerId, remoteSessionId: opened.id, cwd: '/work');
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void say(Map<String, Object?> message) => incoming.add(jsonEncode(message));

  /// A permission request of the conversation's own agent (`parent`), or of the task [task] (the host's `TASK:CALL`).
  void ask(int id, String title, {String? task, String call = 'c', Object? input = const {'file_path': 'a.txt'}}) => say({
    'jsonrpc': '2.0',
    'id': id,
    'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': task == null ? 'p$id' : '$task:$call$id', 'title': title, 'kind': 'edit', 'rawInput': input},
      'options': permissionOptions,
      if (task != null)
        '_meta': {
          delegateMeta: {'taskId': task, 'title': 'Fix the tests', 'source': 'codex'},
        },
    },
  });

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;

  List<Map<String, dynamic>> sentOf(String method) => [for (final message in sent) if (message['method'] == method) message];
}
