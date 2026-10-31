import 'dart:async';

import 'channel.dart';
import 'endpoints.dart';
import 'websocket.dart';
import 'webtransport.dart';

typedef WebTransportDialer = Future<AcpMessageChannel> Function(
    Uri url, Duration timeout, List<String> certificatePins);
typedef WebSocketDialer = Future<AcpMessageChannel> Function(
    Uri url, Duration timeout);

/// Connect only; do NOT allow ACP messages until an independent peer-proof
/// adapter has authenticated the remote peer. The connector does not claim
/// that WSS or a certificate hash proves coqui-peer pairing.
final class AcpConnector {
  const AcpConnector({
    WebTransportDialer? webTransport,
    WebSocketDialer? webSocket,
  }) : _webTransport = webTransport ?? openAcpWebTransport,
       _webSocket = webSocket ?? openAcpWebSocket;

  final WebTransportDialer _webTransport;
  final WebSocketDialer _webSocket;

  /// Prefer WebTransport. Fall back to the advertised WebSocket URL ONLY if
  /// opening WebTransport fails, before any peer proof or ACP messages.
  /// An authentication failure, protocol error, or later drop MUST NOT
  /// trigger a fallback or replay an uncertain session/prompt request.
  Future<AcpTransportConnection> dial(
    AcpEndpoints endpoints, {
    Duration timeout = const Duration(seconds: 10),
  }) async {
    if (timeout <= Duration.zero) throw ArgumentError.value(timeout, 'timeout');

    if (endpoints.webTransport case final wt?) {
      try {
        final channel = await _webTransport(
          wt, timeout, endpoints.certificatePins,
        );
        return AcpTransportConnection(AcpWireTransport.webTransport, channel);
      } catch (_) {
        // Dial failures only. The caller is responsible for authenticating
        // this replacement socket against the SAME expected peer identity.
      }
    }
    final channel = await _webSocket(endpoints.webSocket, timeout);
    return AcpTransportConnection(AcpWireTransport.webSocket, channel);
  }
}
