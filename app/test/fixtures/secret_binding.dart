// A request answered by an identity, as the host and the app do it, checked from the host's side: what a signer must get right.
import 'package:canoa/acp/secret_wire.dart' show SecretIdentity;
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp/secret_unprotected.dart';

const _schema = '{"type":"object","properties":{"user":{"type":"string"},"password":{"type":"string","writeOnly":true}},"required":["user","password"]}';

/// The host asks [app] for a password and opens the answer: the plain text and the secret, as the host reads them.
Future<(String, String)> answerOpenedByHost(SecretIdentity app) async {
  final host = ecdsaP256Prover(List<int>.generate(32, (i) => i + 61));
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  const context = AcpSecretContext(sessionId: 's1', requestId: 'r-binding', message: 'The deploy needs your login', schema: _schema);
  final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, host, app.prover.id, now);
  app.verifier.trust(host.id);
  final collector = await app.verifier.collector(host.id, AcpSecretMethod.request, context, receiver.recipient, now);
  final sealed = {'password': await collector.seal('password', 'correct-horse-9')};
  const plain = '{"user":"ana"}';
  final auth = await collector.authenticate(app.prover, plain, sealed, now);
  final opened = await receiver.openValues(AcpSecretVerifier(own: host.id, known: [app.prover.id]), app.prover.id, plain, sealed, auth, now);
  return (opened.plain['user']! as String, opened.secrets['password']!.text);
}
