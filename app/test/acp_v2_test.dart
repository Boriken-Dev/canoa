import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/transport/agent_transport.dart';
import 'package:flutter_test/flutter_test.dart';

final class FakeChannel implements AgentConnection {
  final input = StreamController<String>.broadcast(sync: true);
  final sent = <Map<String, dynamic>>[];
  void Function(Map<String, dynamic>)? respondTo;
  bool closed = false;
  @override
  AgentTransportKind get kind => AgentTransportKind.webSocket;
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
Future<AcpV2Client> verified(FakeChannel channel) => AcpV2Client.open(
    channel, expectedPeerId: peerId, verifyPeer: (_, expected) async => expected);

void main() {
  test('peer proof is required before any ACP request', () async {
    final channel = FakeChannel();
    final proof = Completer<String>();
    final pending = AcpV2Client.open(channel,
      expectedPeerId: peerId, verifyPeer: (_, __) => proof.future);
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

  test('rejects unsupported v1 and absent session capabilities', () async {
    for (final result in [
      {'protocolVersion': 1, 'capabilities': {'session': <String, Object>{}}},
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
    expect(((channel.sent.last['result'] as Map)['outcome'] as Map)['outcome'], 'cancelled');
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
}
