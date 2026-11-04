import '../auth/browser_peer.dart';
import 'package:coqui_web/coqui_web.dart';
import 'client.dart';
import 'connect.dart';

/// Connect using the real non-extractable browser key of [method] and an explicitly pinned peer ID. There is no TOFU and no
/// automatic pairing. A method this browser cannot run fails before any connection, and no other method is tried.
Future<VerifiedAgent> connectBrowserAcp(
  WebEndpoints endpoints, {
  required String pinnedPeerId,
  required PeerMethod method,
  Duration timeout = const Duration(seconds: 10),
  AgentDecider? decider,
}) async {
  final why = await browserMethodUnavailable(method);
  if (why != null) throw MethodUnavailable(method, why);
  return connectVerifiedAcp(
    endpoints, expectedPeerId: pinnedPeerId,
    verifyPeer: (channel, expected) => verifyBrowserPeer(channel, expected, method),
    timeout: timeout, decider: decider,
  );
}

/// Pairs this browser's Noise key with the machine waiting at [url] (`coqui-acp-bridge pair-accept --method noise`) by the
/// [pin] it shows, and answers that machine's id, to pin under Noise.
Future<String> pairBrowserAcp(Uri url, String pin, {Duration timeout = const Duration(seconds: 10)}) async {
  final why = await browserMethodUnavailable(PeerMethod.noise);
  if (why != null) throw MethodUnavailable(PeerMethod.noise, why);
  final source = await connectWebChannel(
    chatEndpoints(webSocket: url.toString(), preferWebTransport: false),
    timeout: timeout,
  );
  return pairBrowserByPin(StagedChannel(source), pin);
}
