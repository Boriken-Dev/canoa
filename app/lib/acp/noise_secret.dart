import 'dart:typed_data';

import 'package:coqui_acp/secret.dart';

import 'secret_wire.dart' show SecretIdentity;

/// What a Noise proof left for secret prompts: the identity this connection binds answers with, or none when the proof gave no
/// usable key of the host's, in which case the connection offers the host no secret at all.
typedef NoiseSecrets = ({SecretIdentity? identity});

const _x25519SpkiLength = 44;

bool _same(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  var diff = 0;
  for (var i = 0; i < a.length; i++) {
    diff |= a[i] ^ b[i];
  }
  return diff == 0;
}

/// The identity a Noise connection to the host [hostId] binds secret answers with: this app's static key ([spki], whose private half
/// [agree] uses without ever handing it over) bound to that one host, whose static key [hostSpki] the handshake proved. Null unless
/// [hostSpki] is an X25519 key whose digest is exactly [hostId], the id this app pinned.
///
/// [agree] is called only with the host's own point: no other key is ever agreed with through this identity.
SecretIdentity? noiseSecretIdentity({
  required String hostId,
  required Uint8List? hostSpki,
  required Uint8List spki,
  required Future<Uint8List> Function(Uint8List point) agree,
}) {
  if (hostSpki == null || hostSpki.length != _x25519SpkiLength) return null;
  try {
    if (AcpIdentityId.of(hostSpki) != AcpIdentityId.parse(hostId)) return null;
    final point = Uint8List.fromList(hostSpki.sublist(_x25519SpkiLength - 32));
    Future<Uint8List> toHost(Uint8List peer) {
      if (!_same(peer, point)) throw const AcpSecretException(AcpSecretFailure.key, 'not the host this connection proved');
      return agree(peer);
    }

    final prover = AcpSecretProver.agreeing(spki: spki, agree: toHost, collectorSpki: hostSpki);
    return (
      prover: prover,
      verifier: AcpSecretVerifier(own: prover.id, agreementSpki: spki, agree: toHost),
      offer: const AcpSecretsCapability(bindings: [acpBindingDh]),
    );
  } on AcpSecretException {
    return null;
  }
}
