import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'peer_proof.dart';

const _native = 'a native build has no browser WebCrypto to hold its key';

Future<String?> browserMethodUnavailable(PeerMethod method) async => _native;

Future<VerifiedPeer> verifyBrowserPeer(WebChannel channel, String expectedPeerId, PeerMethod method) =>
    Future.error(MethodUnavailable(method, _native));

Future<String> localBrowserPeerId(PeerMethod method) => Future.error(MethodUnavailable(method, _native));

Future<String> pairBrowserByPin(WebChannel channel, String pin) async {
  await channel.close();
  throw const MethodUnavailable(PeerMethod.noise, _native);
}
