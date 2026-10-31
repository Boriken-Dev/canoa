import 'dart:async';

import 'package:canoa/transport/channel.dart';
import 'package:canoa/transport/connector.dart';
import 'package:canoa/transport/endpoints.dart';
import 'package:flutter_test/flutter_test.dart';

final class FakeChannel implements AcpMessageChannel {
  final controller = StreamController<String>();
  bool closed = false;

  @override
  Stream<String> get messages => controller.stream;

  @override
  Future<void> send(String message) async => controller.add(message);

  @override
  Future<void> close() async {
    closed = true;
    await controller.close();
  }
}

void main() {
  final wt = Uri.parse('https://agent.example:4433/acp');
  final ws = Uri.parse('wss://agent.example/acp');

  test('WebTransport is first when both are advertised', () async {
    final calls = <String>[];
    final preferred = FakeChannel();
    final connector = AcpConnector(
      webTransport: (url, timeout, pins) async {
        calls.add('webtransport');
        expect(url, wt);
        return preferred;
      },
      webSocket: (url, timeout) async {
        calls.add('websocket');
        return FakeChannel();
      },
    );
    final connected = await connector.dial(
      AcpEndpoints(webTransport: wt, webSocket: ws),
    );
    expect(connected.kind, AcpWireTransport.webTransport);
    expect(calls, ['webtransport']);
    await connected.channel.close();
  });

  test('WebSocket fallback only after WebTransport dial fails', () async {
    final calls = <String>[];
    final fallback = FakeChannel();
    final connector = AcpConnector(
      webTransport: (url, timeout, pins) async {
        calls.add('webtransport');
        throw StateError('QUIC blocked');
      },
      webSocket: (url, timeout) async {
        calls.add('websocket');
        expect(url, ws);
        return fallback;
      },
    );
    final connection = await connector.dial(
      AcpEndpoints(webTransport: wt, webSocket: ws),
    );
    expect(connection.kind, AcpWireTransport.webSocket);
    expect(calls, ['webtransport', 'websocket']);
    await connection.channel.close();
  });

  test('WebSocket is used directly when no WebTransport is advertised', () async {
    final connector = AcpConnector(
      webTransport: (url, timeout, pins) async => throw StateError('not called'),
      webSocket: (url, timeout) async => FakeChannel(),
    );
    final connection = await connector.dial(AcpEndpoints(webSocket: ws));
    expect(connection.kind, AcpWireTransport.webSocket);
    await connection.channel.close();
  });

  test('invalid or insecure endpoint combinations are refused', () {
    expect(() => AcpEndpoints(webSocket: Uri.parse('ws://agent.example')), throwsArgumentError);
    expect(() => AcpEndpoints(webSocket: ws, webTransport: Uri.parse('http://agent.example')), throwsArgumentError);
    expect(() => AcpEndpoints(webSocket: Uri.parse('wss://agent.example/?token=secret')), throwsArgumentError);
    expect(() => AcpEndpoints(webSocket: Uri.parse('wss://user:secret@agent.example')), throwsArgumentError);
    expect(() => AcpEndpoints(webSocket: ws, certificatePins: ['foo']), throwsArgumentError);
  });

  test('local development WS requires an explicit opt in', () {
    final localhost = Uri.parse('ws://127.0.0.1:8765/acp');
    expect(() => AcpEndpoints(webSocket: localhost), throwsArgumentError);
    expect(AcpEndpoints(webSocket: localhost, allowLocalPlaintext: true).webSocket, localhost);
    expect(() => AcpEndpoints(
      webSocket: Uri.parse('ws://192.168.1.2:8000/acp'),
      allowLocalPlaintext: true,
    ), throwsArgumentError);
  });

  test('no automatic mid-session fallback or command replay', () async {
    var websockets = 0;
    final channel = FakeChannel();
    final connector = AcpConnector(
      webTransport: (url, timeout, pins) async => channel,
      webSocket: (url, timeout) async {
        websockets++;
        return FakeChannel();
      },
    );
    final connection = await connector.dial(
      AcpEndpoints(webTransport: wt, webSocket: ws),
    );
    await connection.channel.close();
    expect(websockets, 0);
  });
}
