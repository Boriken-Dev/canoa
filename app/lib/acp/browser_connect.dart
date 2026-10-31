import '../auth/browser_peer.dart';
import '../transport/agent_transport.dart';
import 'client.dart';
import 'connect.dart';

/// Connect using the real non-extractable browser WebCrypto identity and an
/// explicitly pinned peer ID. There is no TOFU and no automatic pairing.
Future<AcpV2Client> connectBrowserAcp(
  AgentEndpoints endpoints, {
  required String pinnedPeerId,
  Duration timeout = const Duration(seconds: 10),
}) => connectVerifiedAcp(
  endpoints, expectedPeerId: pinnedPeerId,
  verifyPeer: verifyBrowserPeer, timeout: timeout,
);
