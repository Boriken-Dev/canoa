import 'package:coqui_web/coqui_web.dart';
import 'client.dart';

/// The endpoints the chat dials for an agent it pins. Plain WebSocket is taken on loopback only, where `canoa-host` listens:
/// the peer proof does not depend on the transport, and every message after it is sealed in the session it ends in.
WebEndpoints chatEndpoints({
  required String webSocket,
  String webTransport = '',
  bool preferWebTransport = true,
}) => WebEndpoints(
  webTransport: preferWebTransport && webTransport.isNotEmpty ? Uri.parse(webTransport) : null,
  webSocket: Uri.parse(webSocket),
  allowLocalPlainWebSocket: true,
);

/// Opens the preferred network transport, then proves the SAME expected peer
/// identity on that selected transport before sending any ACP message.
///
/// WebTransport -> WebSocket fallback happens only during transport opening.
/// A rejected peer proof must close the link and must NOT trigger fallback.
Future<VerifiedAgent> connectVerifiedAcp(
  WebEndpoints endpoints, {
  required String expectedPeerId,
  required VerifyAcpPeer verifyPeer,
  Duration timeout = const Duration(seconds: 10),
  WebDialer? webTransportDialer,
  WebDialer? webSocketDialer,
  AgentDecider? decider,
  AgentElicitor? elicitor,
}) async {
  final source = await connectWebChannel(
    endpoints,
    timeout: timeout,
    webTransportDialer: webTransportDialer,
    webSocketDialer: webSocketDialer,
  );
  final connection = StagedChannel(source);
  final client = await VerifiedAgent.open(
    connection,
    expectedPeerId: expectedPeerId,
    verifyPeer: verifyPeer,
    verificationTimeout: timeout,
    decider: decider,
    elicitor: elicitor,
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
