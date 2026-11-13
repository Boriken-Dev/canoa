import 'package:coqui_web/coqui_web.dart';
import 'client.dart';
import 'pipe_channel.dart';

/// The endpoints the chat dials for an agent it pins. Plain WebSocket is taken on loopback only, where `canoa` listens by
/// default: the peer proof does not depend on the transport, and every message after it is sealed in the session it ends in.
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
  AgentSecrets? secrets,
  Map<String, Object?> initializeMeta = const {},
}) async {
  return _verifiedAcp(
    await connectWebChannel(
      endpoints,
      timeout: timeout,
      webTransportDialer: webTransportDialer,
      webSocketDialer: webSocketDialer,
    ),
    expectedPeerId: expectedPeerId,
    verifyPeer: verifyPeer,
    timeout: timeout,
    decider: decider,
    elicitor: elicitor,
    secrets: secrets,
    initializeMeta: initializeMeta,
  );
}

/// As [connectVerifiedAcp] for an entry the host listens on with a pipe or a Unix socket: [pipe] is the `pipe:NAME` or
/// `unix:PATH` that `canoa serve` printed as its url. The operating system keeps other users out, and the peer proof still
/// has to show the host holds [expectedPeerId]'s key before any ACP message.
Future<VerifiedAgent> connectVerifiedAcpOverPipe(
  String pipe, {
  required String expectedPeerId,
  required VerifyAcpPeer verifyPeer,
  Duration timeout = const Duration(seconds: 10),
  AgentDecider? decider,
  AgentElicitor? elicitor,
  AgentSecrets? secrets,
  Map<String, Object?> initializeMeta = const {},
}) async => _verifiedAcp(
  await openPipeChannel(pipe, timeout: timeout),
  expectedPeerId: expectedPeerId,
  verifyPeer: verifyPeer,
  timeout: timeout,
  decider: decider,
  elicitor: elicitor,
  secrets: secrets,
  initializeMeta: initializeMeta,
);

Future<VerifiedAgent> _verifiedAcp(
  WebChannel source, {
  required String expectedPeerId,
  required VerifyAcpPeer verifyPeer,
  required Duration timeout,
  AgentDecider? decider,
  AgentElicitor? elicitor,
  AgentSecrets? secrets,
  Map<String, Object?> initializeMeta = const {},
}) async {
  final connection = StagedChannel(source);
  final client = await VerifiedAgent.open(
    connection,
    expectedPeerId: expectedPeerId,
    verifyPeer: verifyPeer,
    verificationTimeout: timeout,
    decider: decider,
    elicitor: elicitor,
    secrets: secrets,
    initializeMeta: initializeMeta,
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
