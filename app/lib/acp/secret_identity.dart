import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_acp/secret.dart';

import '../auth/browser_peer.dart';
import 'secret_wire.dart' show SecretIdentity;

/// What signs for an identity whose private key cannot be read: its SubjectPublicKeyInfo, and the function that makes the 64-byte
/// IEEE P1363 ECDSA signature of a message with it.
typedef SecretSigner = ({List<int> spki, Future<Uint8List> Function(Uint8List message) sign});

/// The two labels a secret binding's statement begins with, each with its 4-byte length.
final List<Uint8List> _statementLabels = [
  for (final label in const ['dev.boriken.coqui.acp/secret/v1 binding', 'dev.boriken.coqui.acp/secret/v1 values'])
    Uint8List.fromList([0, 0, 0, label.length, ...ascii.encode(label)]),
];

bool _isStatement(Uint8List message) => _statementLabels.any((prefix) {
  if (message.length < prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (message[i] != prefix[i]) return false;
  }
  return true;
});

/// [signer]'s function, which signs only what a secret binding's statement looks like. The identity's key also proves this app to a
/// host, so a handshake's bytes can never be signed through it.
Future<Uint8List> Function(Uint8List message) secretStatementSigner(SecretSigner signer) => (message) async {
  if (!_isStatement(message)) {
    throw const AcpSecretException(AcpSecretFailure.seal, 'only a secret binding is signed');
  }
  return await signer.sign(message);
};

/// An identity that binds secret answers by ECDSA with [signer], offered to hosts for exactly that binding.
SecretIdentity secretIdentityFor(SecretSigner signer) {
  final prover = AcpSecretProver.signing(spki: signer.spki, sign: secretStatementSigner(signer));
  return (
    prover: prover,
    verifier: AcpSecretVerifier(own: prover.id),
    offer: const AcpSecretsCapability(bindings: [acpBindingEcdsa]),
  );
}

/// The identity this page binds secret answers with, or null where it has no WebCrypto key of its own to sign with: the host is then
/// offered nothing and serves its agents no secret tools. Only the WebCrypto method can: a Noise identity binds by a
/// Diffie-Hellman with the host's static key, which the Noise module does not hand to the page.
Future<SecretIdentity?> browserSecretIdentity() async {
  final signer = await browserSecretSigner();
  return signer == null ? null : secretIdentityFor(signer);
}
