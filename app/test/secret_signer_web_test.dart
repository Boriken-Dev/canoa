// The page's own WebCrypto key as the identity secret answers are bound with, in a real browser: the private key stays non-extractable,
// and what it signs verifies for the host.
//
//   flutter test --platform chrome test/secret_signer_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:typed_data';

import 'package:canoa/acp/secret_identity.dart';
import 'package:canoa/auth/web/peer_session.dart' show PeerFailure;
import 'package:canoa/auth/web/web_peer_bridge.dart';
import 'package:canoa/auth/web/webcrypto_method.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_binding.dart';

void main() {
  test('the key signs no handshake: the real signature input of each role is refused, and nothing is signed', () async {
    final key = await WebCryptoIdentity.generate();
    for (final role in ['server', 'client', 'accept']) {
      await expectLater(key.signSecretStatement(handshakeSignatureInput(role)), throwsA(isA<PeerFailure>()));
    }
    await expectLater(key.signSecretStatement(Uint8List(0)), throwsA(isA<PeerFailure>()));
  });

  test('a non-extractable key binds an answer that opens for the host, under the id the host paired', () async {
    final key = await WebCryptoIdentity.generate();
    expect(key.privateKey.extractable, isFalse);
    final identity = secretIdentityFor((spki: key.spki, sign: key.signSecretStatement));
    expect(identity.prover.id.toString(), key.peerId);
    // WebCrypto's signature is randomized and may have a high s: the binding takes the low form.
    for (var i = 0; i < 3; i++) {
      expect(await answerOpenedByHost(identity), ('ana', 'correct-horse-9'));
    }
  });

  test('the bridge gives a signer for its own key, which refuses what is not a secret binding', () async {
    final bridge = WebPeerBridge(loadWebCrypto: WebCryptoIdentity.generate);
    final signer = await bridge.webcryptoSigner();
    expect(signer.spki, hasLength(91));
    final identity = secretIdentityFor((spki: signer.spki, sign: signer.sign));
    expect(await answerOpenedByHost(identity), ('ana', 'correct-horse-9'));
    await expectLater(secretStatementSigner((spki: signer.spki, sign: signer.sign))(Uint8List.fromList([1, 2, 3])), throwsA(anything));
  });
}
