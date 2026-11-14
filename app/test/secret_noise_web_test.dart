// The page's own Noise key as the identity secret answers are bound with, in a real browser: the private key stays a non-extractable
// WebCrypto key, the proof gives the peer's static public key, and what the key agrees with opens for the host.
//
// The module comes from tool/build_noise.py --test-peer, as noise_method_web_test.dart's does:
//   python tool/build_noise.py --test-peer
//   flutter test --platform chrome test/secret_noise_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'package:canoa/acp/noise_secret.dart';
import 'package:canoa/auth/web/noise_method.dart';
import 'package:canoa/auth/web/subtle.dart';
import 'package:canoa/auth/web/web_peer_bridge.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp/secret_unprotected.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/noise_responder.dart';

const _schema = '{"type":"object","properties":{"user":{"type":"string"},"password":{"type":"string","writeOnly":true}},"required":["user","password"]}';
final _hostSecret = List<int>.generate(32, (i) => 41 + i);

String _idOf(Uint8List digest) => 'sha256:${[for (final byte in digest) byte.toRadixString(16).padLeft(2, '0')].join()}';

void main() {
  test('a proof gives the peer\'s static public key, whose digest is the id it proved, and no key before it ends', () async {
    final client = await NoiseIdentity.generate();
    final side = await ResponderSide.create();
    final server = side.admit(client.peerId);
    final run = await client.prove(side.peerId);
    var step = await run.step(null);
    expect(step.peerKey, isNull);
    final answers = server.step(step.send.single.toJS).toDart;
    step = await run.step(answers.single.toDart);
    expect(step.peerId, side.peerId);
    final key = step.peerKey!;
    expect(key, hasLength(44), reason: 'an X25519 SubjectPublicKeyInfo');
    expect(_idOf(await digest256(key)), side.peerId);
    expect(AcpIdentityId.of(key).toString(), side.peerId);
  });

  test('the key agrees without leaving WebCrypto, and binds an answer that opens for the host under the id the host paired', () async {
    final key = await NoiseIdentity.generate();
    expect(key.privateKey.extractable, isFalse);
    final hostSpki = await x25519Spki(_hostSecret);
    final hostId = AcpIdentityId.of(hostSpki).toString();
    final app = noiseSecretIdentity(hostId: hostId, hostSpki: hostSpki, spki: key.spki, agree: key.agreeSecret)!;
    expect(app.prover.id.toString(), key.peerId);

    final host = await x25519Prover(_hostSecret, key.spki);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final context = AcpSecretContext(sessionId: 's1', requestId: 'w1', message: 'The deploy needs your login', schema: _schema);
    final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, host, app.prover.id, now);
    app.verifier.trust(host.id);
    final collector = await app.verifier.collector(host.id, AcpSecretMethod.request, context, receiver.recipient, now);
    final sealed = {'password': await collector.seal('password', 'correct-horse-9')};
    const plain = '{"user":"ana"}';
    final auth = await collector.authenticate(app.prover, plain, sealed, now);
    final verifier = AcpSecretVerifier(own: host.id, known: [app.prover.id], agreementSpki: hostSpki, agree: x25519Agreement(_hostSecret));
    final opened = await receiver.openValues(verifier, app.prover.id, plain, sealed, auth, now);
    expect(opened.plain['user'], 'ana');
    expect(opened.secrets['password']!.text, 'correct-horse-9');
  });

  test('the bridge gives an agreement for its own Noise key, which agrees with the host\'s point and with the host\'s own result', () async {
    final bridge = WebPeerBridge(loadNoise: NoiseIdentity.generate);
    final own = await bridge.noiseAgreement();
    expect(own.spki, hasLength(44));
    final point = Uint8List.sublistView(await x25519Spki(_hostSecret), 12);
    final here = await own.agree(Uint8List.fromList(point));
    final there = await x25519Agreement(_hostSecret)(Uint8List.sublistView(own.spki, 12));
    expect(here, there, reason: 'the two static keys agree on one secret');
  });
}
