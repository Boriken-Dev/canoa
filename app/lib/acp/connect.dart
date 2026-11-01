import 'package:coqui_web/coqui_web.dart';
import 'client.dart';

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
