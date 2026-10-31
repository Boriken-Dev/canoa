import 'dart:async';
import 'dart:typed_data';

import 'package:canoa/transport/agent_transport.dart';
import 'package:flutter_test/flutter_test.dart';

class FakeLink implements AgentConnection {
  FakeLink(this.kind);
  @override
  final AgentTransportKind kind;
  @override
  Stream<String> get messages => const Stream<String>.empty();
  @override
  Future<void> send(String message) async {}
  @override
  Future<void> close() async {}
}

void main() {
  final wt = Uri.parse('https://agent.example:4433/acp');
  final ws = Uri.parse('wss://agent.example:4434/acp');

  test('WebTransport wins when available', () async {
    final called = <String>[];
    final link = await connectAgentTransport(
      AgentEndpoints(webTransport: wt, webSocket: ws),
      webTransportDialer: (url, timeout, pins) async {
        called.add('wt');
        return FakeLink(AgentTransportKind.webTransport);
      },
      webSocketDialer: (url, timeout, pins) async {
        called.add('ws');
        return FakeLink(AgentTransportKind.webSocket);
      },
    );
    expect(link.kind, AgentTransportKind.webTransport);
    expect(called, ['wt']);
  });

  test('unavailable WebTransport falls back to configured WSS', () async {
    final called = <String>[];
    final link = await connectAgentTransport(
      AgentEndpoints(webTransport: wt, webSocket: ws),
      webTransportDialer: (url, timeout, pins) async {
        called.add('wt');
        throw const TransportUnavailable('HTTP/3 blocked');
      },
      webSocketDialer: (url, timeout, pins) async {
        called.add('ws');
        expect(url, ws);
        return FakeLink(AgentTransportKind.webSocket);
      },
    );
    expect(link.kind, AgentTransportKind.webSocket);
    expect(called, ['wt', 'ws']);
  });

  test('pinned TLS failure never silently downgrades', () async {
    var touchedWs = false;
    final endpoints = AgentEndpoints(
      webTransport: wt,
      webSocket: ws,
      certificatePins: ['sha256:${'ab' * 32}'],
    );
    await expectLater(
      connectAgentTransport(
        endpoints,
        webTransportDialer: (url, timeout, pins) async =>
            throw const TransportSecurityFailure('pin mismatch'),
        webSocketDialer: (url, timeout, pins) async {
          touchedWs = true;
          return FakeLink(AgentTransportKind.webSocket);
        },
      ),
      throwsA(isA<TransportSecurityFailure>()),
    );
    expect(touchedWs, isFalse);
  });

  test('no fallback after an unknown or protocol failure', () async {
    var fallback = false;
    await expectLater(connectAgentTransport(
      AgentEndpoints(webTransport: wt, webSocket: ws),
      webTransportDialer: (url, timeout, pins) async =>
          throw const TransportProtocolFailure('bad ACP envelope'),
      webSocketDialer: (url, timeout, pins) async {
        fallback = true;
        return FakeLink(AgentTransportKind.webSocket);
      },
    ), throwsA(isA<TransportProtocolFailure>()));
    expect(fallback, isFalse);
  });

  test('only accepts explicitly named endpoints and secure defaults', () {
    expect(
      () => AgentEndpoints(webTransport: wt),
      returnsNormally,
    );
    expect(() => AgentEndpoints(), throwsArgumentError);
    expect(() => AgentEndpoints(webTransport: Uri.parse('https://agent.example/acp')),
        throwsArgumentError);
    expect(() => AgentEndpoints(webSocket: Uri.parse('ws://agent.example/acp')),
        throwsArgumentError);
    expect(() => AgentEndpoints(webSocket: Uri.parse('wss://else.example'), webTransport: wt),
        throwsArgumentError);
    expect(() => AgentEndpoints(webSocket: Uri.parse('wss://agent.example?key=x')),
        throwsArgumentError);
    expect(() => AgentEndpoints(
      webSocket: Uri.parse('ws://localhost:8080/acp'), allowLocalPlainWebSocket: true,
    ), returnsNormally);
  });

  test('WebTransport little-endian frames survive chunk boundaries', () {
    final decoder = AgentFrameDecoder();
    final joined = Uint8List.fromList([
      ...frameMessage('{"jsonrpc":"2.0","id":1}'),
      ...frameMessage('{"jsonrpc":"2.0","method":"hello"}'),
    ]);
    expect(decoder.add(Uint8List.fromList(joined.sublist(0, 3))), isEmpty);
    expect(decoder.add(Uint8List.fromList(joined.sublist(3, 8))), isEmpty);
    final received = decoder.add(Uint8List.fromList(joined.sublist(8)));
    expect(received, ['{"jsonrpc":"2.0","id":1}', '{"jsonrpc":"2.0","method":"hello"}']);
  });

  test('oversized and invalid frames are rejected', () {
    expect(() => frameMessage('abcdef', maxBytes: 5),
        throwsA(isA<TransportProtocolFailure>()));
    final decoder = AgentFrameDecoder(maxBytes: 5);
    expect(() => decoder.add(Uint8List.fromList([6, 0, 0, 0])),
        throwsA(isA<TransportProtocolFailure>()));
    final invalidUtf8 = AgentFrameDecoder();
    expect(() => invalidUtf8.add(Uint8List.fromList([1,0,0,0,255])),
        throwsA(isA<TransportProtocolFailure>()));
  });

  test('direct WebSocket remains supported', () async {
    final link = await connectAgentTransport(
      AgentEndpoints(webSocket: ws),
      webSocketDialer: (url, timeout, pins) async =>
          FakeLink(AgentTransportKind.webSocket),
    );
    expect(link.kind, AgentTransportKind.webSocket);
  });
}
