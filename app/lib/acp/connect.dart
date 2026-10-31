import '../transport/staged_connection.dart';
import '../transport/agent_transport.dart';
import 'client.dart';

/// Opens the preferred network transport, then proves the SAME expected peer
/// identity on that selected transport before sending any ACP message.
///
/// WebTransport -> WebSocket fallback happens only during transport opening.
/// A rejected peer proof must close the link and must NOT trigger fallback.
Future<AcpV2Client> connectVerifiedAcp(
  AgentEndpoints endpoints, {
  required String expectedPeerId,
  required VerifyAcpPeer verifyPeer,
  Duration timeout = const Duration(seconds: 10),
  AgentDialer? webTransportDialer,
  AgentDialer? webSocketDialer,
}) async {
  final source = await connectAgentTransport(
    endpoints,
    timeout: timeout,
    webTransportDialer: webTransportDialer,
    webSocketDialer: webSocketDialer,
  );
  final connection = StagedAgentConnection(source);
  final client = await AcpV2Client.open(
    connection,
    expectedPeerId: expectedPeerId,
    verifyPeer: verifyPeer,
    verificationTimeout: timeout,
  );
  try {
    connection.allowApplicationMessages();
    await client.initialize();
    return client;
  } catch (_) {
    await client.close();
    rethrow;
  }
}
