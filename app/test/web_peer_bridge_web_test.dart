// The web build's peer bridge a step at a time, as the app drives it, against each method's own responder, in a real browser.
//
// Needs the Noise module with its test responder (python tool/build_noise.py --out test/noise --test-peer):
//   flutter test --platform chrome test/web_peer_bridge_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
//
// A PIN pairing has no responder here: its desk needs a trust store on disk. The pairing runs against a real
// `coqui-acp-bridge pair-accept` in bridge_test.dart and the real-browser drivers under bindings/browser/chromium/e2e.
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/auth/web/noise_method.dart';
import 'package:canoa/auth/web/peer_session.dart';
import 'package:canoa/auth/web/web_peer_bridge.dart';
import 'package:canoa/auth/web/webcrypto_method.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/noise_responder.dart';
import 'fixtures/webcrypto_responder.dart';

Uint8List text(String value) => Uint8List.fromList(utf8.encode(value));
String string(Uint8List bytes) => utf8.decode(bytes);

Future<WebPeerBridge> browser() async {
  final webcrypto = await WebCryptoIdentity.generate();
  final noise = await NoiseIdentity.generate();
  return WebPeerBridge(loadWebCrypto: () async => webcrypto, loadNoise: () async => noise);
}

/// Runs [token]'s steps, handing each message sent to [peer], which answers with the messages it sends back.
Future<({String peerId, String session, List<Uint8List> wire})> drive(
  WebPeerBridge bridge,
  String token,
  List<Uint8List> Function(Uint8List message) peer,
) async {
  final wire = <Uint8List>[];
  Uint8List? received;
  for (var steps = 0; steps < 16; steps++) {
    final step = await bridge.step(token, received);
    wire.addAll(step.send);
    final answers = [for (final message in step.send) ...await Future.sync(() => peer(message))];
    if (step.peerId != null) return (peerId: step.peerId!, session: step.session!, wire: wire);
    expect(answers, hasLength(1), reason: 'the peer answers each step once');
    received = answers.single;
  }
  throw StateError('no end');
}

Matcher failure(Pattern message) => throwsA(isA<PeerFailure>().having((e) => e.message, 'message', contains(message)));

void main() {
  test('each method has a key of its own', () async {
    final bridge = await browser();
    final ids = [await bridge.localId(PeerMethod.noise), await bridge.localId(PeerMethod.webcrypto)];
    expect(ids[0], isNot(ids[1]));
    expect(await bridge.unreachable(PeerMethod.noise), isNull);
    expect(await bridge.unreachable(PeerMethod.webcrypto), isNull);
  });

  test('a WebCrypto proof sends text, and ends in a session', () async {
    final bridge = await browser();
    final server = await WebCryptoIdentity.generate();
    final client = await bridge.localId(PeerMethod.webcrypto);
    final responder = WebCryptoResponder.create(server, allowClient: (id) async => id == client);
    PeerSession? serverSession;
    var stage = 0;
    final token = await bridge.begin(PeerMethod.webcrypto, server.peerId);
    final done = await Future(() async {
      final wire = <Uint8List>[];
      Uint8List? received;
      for (;;) {
        final step = await bridge.step(token, received);
        wire.addAll(step.send);
        if (step.peerId != null) return (step: step, wire: wire);
        if (stage++ == 0) {
          received = text(await responder.challenge(step.send.single));
        } else {
          final accepted = await responder.accept(step.send.single);
          serverSession = accepted.session;
          received = text(accepted.reply);
        }
      }
    });
    expect(done.step.peerId, server.peerId);
    expect(string(done.wire.first), contains('"profile":"dev.boriken.coqui.peer-webcrypto/v1"'));
    final record = await bridge.seal(done.step.session!, text('hello'));
    expect(string((await serverSession!.open(record))!), 'hello');
  });

  test('a Noise proof sends its binary handshake and ends in a session both ways', () async {
    final bridge = await browser();
    final side = await ResponderSide.create();
    final responder = side.admit(await bridge.localId(PeerMethod.noise));
    final token = await bridge.begin(PeerMethod.noise, side.peerId);
    final done = await drive(bridge, token, (message) {
      final answers = responder.step(message.toJS).toDart;
      return [for (final answer in answers) answer.toDart];
    });
    expect(done.peerId, side.peerId);
    expect([for (final message in done.wire) message.length], [32, 64]);
    final serverSession = responder.takeSession()!;
    final record = await bridge.seal(done.session, text('hello'));
    expect(string(serverSession.open(record.toJS)!.toDart), 'hello');
    final back = serverSession.seal(text('back').toJS).toDart;
    expect(string((await bridge.open(done.session, back))!), 'back');
    await expectLater(bridge.step('unknown', null), failure('expired or unknown'));

    bridge.closeSession(done.session);
    await expectLater(bridge.seal(done.session, text('late')), failure('Session closed or unknown'));
  });

  test('a Noise proof of a WebCrypto agent fails, and nothing tries the other method', () async {
    final bridge = await browser();
    final server = await WebCryptoIdentity.generate();
    final token = await bridge.begin(PeerMethod.noise, server.peerId);
    final first = await bridge.step(token, null);
    expect(first.send.single, hasLength(32));
    await expectLater(bridge.step(token, Uint8List(96)), throwsA(isA<PeerFailure>()));
    await expectLater(bridge.step(token, Uint8List(96)), failure('expired or unknown'));
  });

  test('a proof that is not asked for again, aborted or past the limit of runs at once is dropped', () async {
    final bridge = await browser();
    final server = await WebCryptoIdentity.generate();
    final tokens = [for (var i = 0; i < 16; i++) await bridge.begin(PeerMethod.webcrypto, server.peerId)];
    await expectLater(bridge.begin(PeerMethod.webcrypto, server.peerId), failure('Too many concurrent'));
    bridge.abort(tokens.first);
    await expectLater(bridge.step(tokens.first, null), failure('expired or unknown'));
    await bridge.begin(PeerMethod.webcrypto, server.peerId);
  });

  test('a pinned id that is not an id is refused before anything is sent', () async {
    final bridge = await browser();
    await expectLater(bridge.begin(PeerMethod.webcrypto, 'nobody'), failure('pinned responder id'));
    await expectLater(bridge.begin(PeerMethod.noise, 'nobody'), throwsA(isA<PeerFailure>()));
    await expectLater(bridge.pair('abc'), throwsA(isA<PeerFailure>()));
  });
}
