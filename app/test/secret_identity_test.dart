// The identity the app binds secret answers with when its key cannot be read: it signs a secret binding and nothing else, and what it
// signs verifies for the host.
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/secret_identity.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp/secret_unprotected.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_binding.dart';

({List<int> spki, Future<Uint8List> Function(Uint8List) sign, List<Uint8List> signed}) keyed() {
  final scalar = List<int>.generate(32, (i) => i + 7);
  final inner = ecdsaP256Signer(scalar);
  final signed = <Uint8List>[];
  return (
    spki: ecdsaP256Spki(scalar),
    sign: (message) {
      signed.add(message);
      return inner(message);
    },
    signed: signed,
  );
}

void main() {
  test('it offers the extension for the ECDSA binding only, under the id of the key', () {
    final key = keyed();
    final identity = secretIdentityFor((spki: key.spki, sign: key.sign));
    expect(identity.offer.toJson(), {'version': 1, 'methods': ['request', 'show'], 'bindings': [acpBindingEcdsa]});
    expect(identity.prover.id, AcpIdentityId.of(key.spki));
    expect(identity.verifier.own, identity.prover.id);
  });

  test('an answer it binds opens for the host, which reads the plain text and the secret', () async {
    final key = keyed();
    final identity = secretIdentityFor((spki: key.spki, sign: key.sign));
    expect(await answerOpenedByHost(identity), ('ana', 'correct-horse-9'));
    // It signed twice, for the host's key binding it checked and for the answer: each a secret statement.
    expect(key.signed, isNotEmpty);
    for (final message in key.signed) {
      expect(utf8.decode(message.sublist(4, 4 + 39), allowMalformed: true), startsWith('dev.boriken.coqui.acp/secret/v1 '));
    }
  });

  test('it signs no message that is not a secret binding, so the key that proves this app cannot sign a handshake through it', () async {
    final key = keyed();
    final sign = secretStatementSigner((spki: key.spki, sign: key.sign));
    // The proof's own bytes, a statement of another label, and the label with no length in front.
    final handshake = Uint8List.fromList(utf8.encode('{"type":"proof","role":"client"}'));
    final other = Uint8List.fromList([0, 0, 0, 5, ...utf8.encode('other'), 1, 2, 3]);
    final bare = Uint8List.fromList(utf8.encode('dev.boriken.coqui.acp/secret/v1 binding'));
    for (final message in [handshake, other, bare, Uint8List(0)]) {
      await expectLater(sign(message), throwsA(isA<AcpSecretException>()));
    }
    expect(key.signed, isEmpty);
  });
}
