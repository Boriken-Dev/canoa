// The app's proof driver against a bridge that records what it is asked, on a channel that records what crosses it.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/auth/peer_proof.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter_test/flutter_test.dart';

/// A WebSocket's two kinds of frame, as the peer sees them.
final class Wire implements ByteWebChannel {
  final incoming = StreamController<Uint8List>();
  final text = <String>[];
  final binary = <Uint8List>[];
  bool closed = false;
  @override
  WebChannelKind get kind => WebChannelKind.webSocket;
  @override
  Stream<String> get messages => incoming.stream.map(decodeText);
  @override
  Stream<Uint8List> get bytes => incoming.stream;
  @override
  Future<void> send(String message) async => text.add(message);
  @override
  Future<void> sendBytes(Uint8List message) async => binary.add(message);
  @override
  Future<void> close() async {
    closed = true;
    if (!incoming.isClosed) unawaited(incoming.close());
  }
}

/// Two steps: a first message, then the end on the peer's answer.
final class Scripted implements PeerBridge {
  Scripted({this.why});
  final String? why;
  final calls = <String>[];
  final received = <Uint8List?>[];

  @override
  Future<String?> unreachable(PeerMethod method) async {
    calls.add('unreachable ${method.id}');
    return why;
  }

  @override
  Future<String> localId(PeerMethod method) async => 'sha256:${method.id}';
  @override
  Future<String> begin(PeerMethod method, String expectedPeerId) async {
    calls.add('begin ${method.id}');
    return 'token';
  }

  @override
  Future<String> pair(String pin) async {
    calls.add('pair $pin');
    return 'token';
  }

  @override
  Future<PeerStep> step(String token, Uint8List? message) async {
    received.add(message);
    if (message == null) return PeerStep(send: [Uint8List.fromList(utf8.encode('hello'))]);
    return const PeerStep(peerId: 'sha256:peer', session: 'session');
  }

  @override
  void abort(String token) => calls.add('abort');
  @override
  Future<Uint8List> seal(String session, Uint8List message) async => message;
  @override
  Future<Uint8List?> open(String session, Uint8List record) async => record;
  @override
  Future<Uint8List> closeRecord(String session) async => Uint8List(1);
  @override
  void closeSession(String session) {}
}

void main() {
  test('a method this runtime cannot run fails before anything is sent, and no other is tried', () async {
    final wire = Wire();
    final bridge = Scripted(why: 'no X25519');
    await expectLater(
      provePeer(bridge, StagedChannel(wire), 'sha256:peer', method: PeerMethod.noise),
      throwsA(isA<MethodUnavailable>().having((e) => e.reason, 'reason', 'no X25519')),
    );
    expect(bridge.calls, ['unreachable noise']);
    expect(wire.text, isEmpty);
    expect(wire.binary, isEmpty);
  });

  test('a Noise proof crosses as binary frames, and its binary answer reaches the bridge whole', () async {
    final wire = Wire();
    final bridge = Scripted();
    final proving = provePeer(bridge, StagedChannel(wire), 'sha256:peer', method: PeerMethod.noise);
    final answer = Uint8List.fromList([0xff, 0x00, 0xfe]);
    wire.incoming.add(answer);
    final proved = await proving;
    expect(proved.peerId, 'sha256:peer');
    expect(wire.text, isEmpty);
    expect(wire.binary.single, utf8.encode('hello'));
    expect(bridge.received, [null, answer]);
    expect(bridge.calls, ['unreachable noise', 'begin noise', 'abort']);
  });

  test('a WebCrypto proof crosses as text frames, as it always has', () async {
    final wire = Wire();
    final bridge = Scripted();
    final proving = provePeer(bridge, StagedChannel(wire), 'sha256:peer', method: PeerMethod.webcrypto);
    wire.incoming.add(Uint8List.fromList(utf8.encode('{"challenge":1}')));
    await proving;
    expect(wire.binary, isEmpty);
    expect(wire.text.single, 'hello');
    expect(bridge.calls, ['unreachable webcrypto', 'begin webcrypto', 'abort']);
  });

  test('pairing by PIN runs under Noise, answers the proved id and closes its channel', () async {
    final wire = Wire();
    final bridge = Scripted();
    final pairing = pairByPin(bridge, StagedChannel(wire), '123456');
    wire.incoming.add(Uint8List.fromList([1, 2, 3]));
    expect(await pairing, 'sha256:peer');
    expect(bridge.calls, ['unreachable noise', 'pair 123456', 'abort']);
    expect(wire.binary.single, utf8.encode('hello'));
    expect(wire.closed, isTrue);
  });

  test('pairing where Noise cannot run sends nothing, says why, and closes its channel', () async {
    final wire = Wire();
    final bridge = Scripted(why: 'no X25519');
    await expectLater(pairByPin(bridge, StagedChannel(wire), '123456'), throwsA(isA<MethodUnavailable>()));
    expect(bridge.calls, ['unreachable noise']);
    expect(wire.binary, isEmpty);
    expect(wire.closed, isTrue);
  });

  test('a method is named by its record, and an unknown one is refused', () {
    expect(PeerMethod.parse('noise'), PeerMethod.noise);
    expect(PeerMethod.parse('webcrypto'), PeerMethod.webcrypto);
    expect(() => PeerMethod.parse('tls'), throwsArgumentError);
  });
}
