import 'contracts.dart';
import 'websocket.dart';
import 'webtransport.dart';

export 'contracts.dart';

typedef AgentDialer = Future<AgentConnection> Function(
  Uri uri,
  Duration timeout,
  List<String> certificatePins,
);

/// A reliable, ordered message transport for ACP JSON-RPC. This does not
/// authenticate a peer, initialize ACP, or authorize any agent access.
///
/// Prefer browser WebTransport, falling back to an EXPLICIT WSS endpoint
/// only on unavailable transport. Never fall back after authentication,
/// protocol, or a pinned-certificate failure. The caller must bind both
/// endpoints to one verified peer before processing ACP messages.
Future<AgentConnection> connectAgentTransport(
  AgentEndpoints endpoints, {
  Duration timeout = const Duration(seconds: 10),
  AgentDialer? webTransportDialer,
  AgentDialer? webSocketDialer,
}) async {
  final wt = endpoints.webTransport;
  final ws = endpoints.webSocket;
  if (wt != null) {
    try {
      return await (webTransportDialer ?? openAgentWebTransport)(
        wt, timeout, endpoints.certificatePins,
      );
    } on TransportUnavailable {
      if (ws == null) rethrow;
    }
  }
  if (ws != null) {
    return (webSocketDialer ?? openAgentWebSocket)(ws, timeout, const []);
  }
  throw const TransportUnavailable('No usable ACP transport endpoint');
}
