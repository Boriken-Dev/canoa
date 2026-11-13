import 'package:coqui_acp/secret.dart';

import 'secret_wire.dart' show SecretIdentity;

/// What binds this app's secret answers on the link to the host in its own process, and the host's identity on that link.
typedef LinkSecretIdentity = ({SecretIdentity identity, AcpIdentityId host});

/// The identity for the in-process link whose secret is [secret], which the host minted and this process holds in memory: its answers
/// are bound by an HMAC under a key derived from it (`link-hmac-sha256`), and that is the only binding it offers. Nothing on a link with
/// a relay may use it: the host serves the method to the in-process link alone.
Future<LinkSecretIdentity> linkSecretIdentity(List<int> secret) async {
  final key = await AcpLinkKey.derive(secret);
  final prover = AcpSecretProver.linked(key, AcpLinkRole.app);
  return (
    identity: (
      prover: prover,
      verifier: AcpSecretVerifier(own: prover.id, link: key, linkRole: AcpLinkRole.app),
      offer: const AcpSecretsCapability(bindings: [acpBindingLink]),
    ),
    host: key.identity(AcpLinkRole.host),
  );
}
