// An agent in memory, speaking v1, whose turn runs until a test ends it: what a conversation shown in two places is driven by.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/session_store.dart';

final class ScriptedAgent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store) async {
    client = VerifiedAgent.fromChild(
      label: 'Scripted',
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
        } else if (message['id'] != null && message['method'] != null && message['method'] != 'session/prompt') {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Fix calc', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> update) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': update},
  }));

  void ask(int id, String title) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': 't$id', 'title': title, 'kind': 'edit', 'rawInput': {'file_path': 'a.txt'}},
      'options': [
        {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
        {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
      ],
    },
  }));

  List<Map<String, dynamic>> answers() => [for (final m in sent) if (m['method'] == null && m['id'] != null && m['result'] != null && m['result']['outcome'] != null) m];

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

