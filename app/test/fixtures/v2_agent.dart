// An agent speaking ACP v2, in memory, for the conversation's tests.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/session_store.dart';

/// A prompt's answer names the message it inserted, and the agent echoes that message first, as a real v2 agent does.
final class V2Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late final VerifiedAgent client;

  Future<(SessionStore, ChatSession)> start() async {
    final store = SessionStore(includePreview: false);
    client = VerifiedAgent.fromChild(
      label: 'V2',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        if (id == null) return;
        switch (message['method']) {
          case 'initialize':
            reply(id, {'protocolVersion': 2, 'capabilities': {'session': {'prompt': {'embeddedContext': true}}}});
          case 'session/new':
            reply(id, {'sessionId': 's1'});
          case 'session/prompt':
            final blocks = message['params']['prompt'] as List<dynamic>;
            update({'sessionUpdate': 'user_message', 'messageId': 'sent${sent.length}', 'content': blocks});
            reply(id, {'messageId': 'sent${sent.length}'});
          default:
            reply(id, <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'V2', agent: 'V2', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    return (store, session);
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {'sessionId': 's1', 'update': body},
  }));

  /// A question about a tool, as v2 asks it.
  void ask(int id, String title, Map<String, Object?> rawInput) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'title': title,
      'subject': {'type': 'tool_call', 'toolCall': {'toolCallId': 'q$id', 'title': title, 'kind': 'edit', 'rawInput': rawInput}},
      'options': [{'optionId': 'no', 'name': 'No', 'kind': 'reject_once'}, {'optionId': 'yes', 'name': 'Yes', 'kind': 'allow_once'}],
    },
  }));
}
