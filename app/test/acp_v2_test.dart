import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter_test/flutter_test.dart';

final class FakeChannel implements WebChannel {
  final input = StreamController<String>.broadcast(sync: true);
  final sent = <Map<String, dynamic>>[];
  void Function(Map<String, dynamic>)? respondTo;
  bool closed = false;
  @override
  WebChannelKind get kind => WebChannelKind.webSocket;
  @override
  Stream<String> get messages => input.stream;
  @override
  Future<void> send(String message) async {
    final request = Map<String, dynamic>.from(jsonDecode(message) as Map);
    sent.add(request);
    respondTo?.call(request);
  }
  void answer(int id, Map<String, dynamic> value) {
    input.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': value}));
  }
  void notify(String id, Map<String, dynamic> update) {
    input.add(jsonEncode({'jsonrpc': '2.0', 'method': 'session/update',
      'params': {'sessionId': id, 'update': update}}));
  }
  @override
  Future<void> close() async {
    closed = true;
    await input.close();
  }
}
const peerId = 'peer:test';
Future<VerifiedAgent> verified(FakeChannel channel) => VerifiedAgent.open(
    channel, expectedPeerId: peerId, verifyPeer: (_, expected) async => expected);

void main() {
  test('peer proof is required before any ACP request', () async {
    final channel = FakeChannel();
    final proof = Completer<String>();
    final pending = VerifiedAgent.open(channel,
      expectedPeerId: peerId, verifyPeer: (_, _) => proof.future);
    await Future<void>.delayed(Duration.zero);
    expect(channel.sent, isEmpty);
    proof.complete('wrong');
    await expectLater(pending, throwsA(isA<AcpFailure>()));
    expect(channel.closed, isTrue);
  });

  test('ACP v2 session methods and streamed state remain distinct', () async {
    final channel = FakeChannel();
    channel.respondTo = (request) {
      switch (request['method']) {
        case 'initialize':
          channel.answer(request['id'] as int, {
            'protocolVersion': 2, 'capabilities': {'session': <String, Object>{}},
          });
        case 'session/new':
          channel.answer(request['id'] as int, {'sessionId': 's1'});
        case 'session/resume':
          channel.notify('s1', {'sessionUpdate': 'state_update', 'state': 'running'});
          channel.answer(request['id'] as int, <String, dynamic>{});
        case 'session/prompt':
          channel.answer(request['id'] as int, {'messageId': 'msg1'});
        case 'session/close':
          channel.answer(request['id'] as int, <String, dynamic>{});
      }
    };
    final client = await verified(channel);
    expect(() => client.prompt('s1', 'text'), throwsStateError);
    await client.initialize();
    expect((channel.sent.first['params'] as Map)['protocolVersion'], 2);
    expect(await client.createSession(cwd: '/work'), 's1');
    final events = <AcpSessionUpdate>[];
    final sub = client.updates.listen(events.add);
    await client.resumeSession('s1', cwd: '/work', replayHistory: true);
    expect((channel.sent.last['params'] as Map)['replayFrom'], {'type': 'start'});
    expect(await client.prompt('s1', 'Hi'), 'msg1');
    channel.notify('s1', {'sessionUpdate': 'agent_message_chunk',
      'messageId': 'a1', 'content': {'type': 'text', 'text': 'Hello'}});
    channel.notify('s1', {'sessionUpdate': 'state_update', 'state': 'idle',
      'stopReason': 'end_turn'});
    expect(events[1].text, 'Hello');
    expect(events.last.state, 'idle');
    await client.cancel('s1');
    expect(channel.sent.last['method'], 'session/cancel');
    expect(channel.sent.last.containsKey('id'), isFalse);
    await client.closeSession('s1');
    await sub.cancel();
    await client.close();
  });

  test('rejects a v2 agent with no session capabilities, and an unknown version', () async {
    for (final result in [
      {'protocolVersion': 9, 'capabilities': {'session': <String, Object>{}}},
      {'protocolVersion': 2, 'capabilities': <String, Object>{}},
    ]) {
      final channel = FakeChannel();
      channel.respondTo = (request) => channel.answer(request['id'] as int, result);
      final client = await verified(channel);
      await expectLater(client.initialize(), throwsA(isA<AcpFailure>()));
      await client.close();
    }
  });

  test('unsolicited privileged client methods fail closed', () async {
    final channel = FakeChannel();
    final client = await verified(channel);
    channel.input.add(jsonEncode({'jsonrpc': '2.0', 'id': 4,
      'method': 'terminal/create'}));
    channel.input.add(jsonEncode({'jsonrpc': '2.0', 'id': 5,
      'method': 'session/request_permission'}));
    await Future<void>.delayed(Duration.zero);
    expect((channel.sent.first['error'] as Map)['code'], -32601);
    expect(channel.sent.last['error'], isNotNull, reason: 'a malformed request is refused, never allowed');
    channel.input.add(jsonEncode({'jsonrpc': '2.0', 'id': 6, 'method': 'session/request_permission', 'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': 't1', 'title': 'Run', 'kind': 'execute'},
      'options': [{'optionId': 'yes', 'kind': 'allow_once'}, {'optionId': 'no', 'kind': 'reject_once'}],
    }}));
    await Future<void>.delayed(Duration.zero);
    expect(((channel.sent.last['result'] as Map)['outcome'] as Map)['optionId'], 'no',
        reason: 'with no approval UI yet, every permission is refused');
    await client.close();
  });

  test('loss during prompt does not replay an uncertain operation', () async {
    final channel = FakeChannel();
    channel.respondTo = (request) {
      if (request['method'] == 'initialize') {
        channel.answer(request['id'] as int, {
          'protocolVersion': 2, 'capabilities': {'session': <String, Object>{}},
        });
      }
    };
    final client = await verified(channel);
    await client.initialize();
    final prompt = client.prompt('s1', 'Non-idempotent action');
    final expected = expectLater(prompt, throwsA(isA<AcpFailure>()));
    await channel.input.close();
    await expected;
    expect(channel.sent.where((f) => f['method'] == 'session/prompt'), hasLength(1));
    await client.close();
  });
  test('a v1 agent shows the turn, gathers its chunks, and ends on the prompt answer', () async {
    final channel = FakeChannel();
    int? promptId;
    channel.respondTo = (frame) {
      if (frame['method'] == 'initialize') {
        channel.answer(frame['id'] as int, {'protocolVersion': 1, 'agentCapabilities': <String, Object>{}});
      } else if (frame['method'] == 'session/prompt') {
        promptId = frame['id'] as int;
      }
    };
    final client = await verified(channel);
    await client.initialize();
    expect(client.protocolVersion, 1);
    final store = SessionStore(includePreview: false);
    final session = store.addRemoteSession(
      title: 'Example', agent: 'Test', peerId: peerId,
      remoteSessionId: 'session-1',
      endpoint: Uri.parse('wss://example.org/acp'),
    );
    store.attachVerifiedClient(session.id, client);
    final sending = store.sendText('Question');
    await Future<void>.delayed(Duration.zero);
    expect(session.status, SessionStatus.working);
    expect(session.messages.single.text, 'Question', reason: 'the message shows while the turn runs');
    channel.notify('session-1', {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Hel'}});
    channel.notify('session-1', {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'lo'}});
    expect(session.messages, hasLength(2));
    expect(session.messages.last.text, 'Hello');
    channel.answer(promptId!, {'stopReason': 'end_turn'});
    await sending;
    expect(session.status, SessionStatus.connected);
    await client.close();
    store.dispose();
  });

  test('one verified peer updates only its own Flutter session', () async {
    final channel = FakeChannel();
    channel.respondTo = (frame) {
      if (frame['method'] == 'initialize') {
        channel.answer(frame['id'] as int, {
          'protocolVersion': 2,
          'capabilities': {'session': <String, Object>{}},
        });
      } else if (frame['method'] == 'session/prompt') {
        channel.answer(frame['id'] as int, {'messageId': 'user-1'});
      }
    };
    final client = await verified(channel);
    await client.initialize();
    final store = SessionStore(includePreview: false);
    final session = store.addRemoteSession(
      title: 'Example', agent: 'Test', peerId: peerId,
      remoteSessionId: 'session-1',
      endpoint: Uri.parse('wss://example.org/acp'),
    );
    store.attachVerifiedClient(session.id, client);
    expect(session.status, SessionStatus.connected);
    channel.notify('other-session', {
      'sessionUpdate': 'agent_message', 'messageId': 'other',
      'content': [{'type': 'text', 'text': 'Not this session'}],
    });
    channel.notify('session-1', {
      'sessionUpdate': 'state_update', 'state': 'running',
    });
    channel.notify('session-1', {
      'sessionUpdate': 'agent_message_chunk', 'messageId': 'agent-1',
      'content': {'type': 'text', 'text': 'Hel'},
    });
    channel.notify('session-1', {
      'sessionUpdate': 'agent_message_chunk', 'messageId': 'agent-1',
      'content': {'type': 'text', 'text': 'lo'},
    });
    expect(session.status, SessionStatus.working);
    expect(session.messages.single.text, 'Hello');
    channel.notify('session-1', {
      'sessionUpdate': 'agent_message', 'messageId': 'agent-1',
      'content': [{'type': 'text', 'text': 'Hello!'}],
    });
    expect(session.messages.single.text, 'Hello!');
    await store.sendText('Question');
    expect(session.messages.last.messageId, 'user-1');
    channel.notify('session-1', {
      'sessionUpdate': 'user_message', 'messageId': 'user-1',
      'content': [{'type': 'text', 'text': 'Question'}],
    });
    expect(session.messages, hasLength(2));
    channel.notify('session-1', {
      'sessionUpdate': 'state_update', 'state': 'idle',
    });
    expect(session.status, SessionStatus.connected);
    await client.close();
    await Future<void>.delayed(Duration.zero);
    expect(session.status, SessionStatus.disconnected);
    store.dispose();
  });

  test('pre-auth transport fallback preserves peer verification', () async {
    final calls = <String>[];
    final channel = FakeChannel();
    channel.respondTo = (frame) {
      if (frame['method'] == 'initialize') {
        channel.answer(frame['id'] as int, {
          'protocolVersion': 2,
          'capabilities': {'session': <String, Object>{}},
        });
      }
    };
    final client = await connectVerifiedAcp(
      WebEndpoints(
        webTransport: Uri.parse('https://agent.example:4433/acp'),
        webSocket: Uri.parse('wss://agent.example/acp'),
      ),
      expectedPeerId: peerId,
      webTransportDialer: (url, timeout, pins) async {
        calls.add('webtransport');
        throw const TransportUnavailable('QUIC blocked or MTU too small');
      },
      webSocketDialer: (url, timeout, pins) async {
        calls.add('websocket');
        return channel;
      },
      verifyPeer: (connection, expected) async {
        calls.add('verified');
        return expected;
      },
    );
    expect(calls, ['webtransport', 'websocket', 'verified']);
    expect(channel.sent.first['method'], 'initialize');
    await client.close();
  });

  test('failed peer proof never retries via WebSocket', () async {
    var fallbackAttempted = false;
    final channel = FakeChannel();
    await expectLater(
      connectVerifiedAcp(
        WebEndpoints(
          webTransport: Uri.parse('https://agent.example:4433/acp'),
          webSocket: Uri.parse('wss://agent.example/acp'),
        ),
        expectedPeerId: peerId,
        webTransportDialer: (url, timeout, pins) async => channel,
        webSocketDialer: (url, timeout, pins) async {
          fallbackAttempted = true;
          return FakeChannel();
        },
        verifyPeer: (connection, expected) async => 'wrong-peer',
      ),
      throwsA(isA<AcpFailure>()),
    );
    expect(fallbackAttempted, isFalse);
    expect(channel.closed, isTrue);
    expect(channel.sent, isEmpty);
  });

}
