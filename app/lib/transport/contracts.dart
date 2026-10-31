import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

enum AgentTransportKind { webTransport, webSocket }

/// A transport is not an authenticated ACP session. A caller must run the
/// verified peer handshake before emitting or accepting ACP messages.
abstract interface class AgentConnection {
  AgentTransportKind get kind;
  Stream<String> get messages;
  Future<void> send(String message);
  Future<void> close();
}

class TransportUnavailable implements Exception {
  const TransportUnavailable(this.reason);
  final String reason;
  @override
  String toString() => 'Transport unavailable: $reason';
}

class TransportSecurityFailure implements Exception {
  const TransportSecurityFailure(this.reason);
  final String reason;
  @override
  String toString() => 'Transport security failure: $reason';
}

class TransportProtocolFailure implements Exception {
  const TransportProtocolFailure(this.reason);
  final String reason;
  @override
  String toString() => 'Transport protocol failure: $reason';
}

/// Explicit endpoints for one authenticated peer. Never derive a different
/// WebSocket host or port from an HTTPS WebTransport URL.
final class AgentEndpoints {
  AgentEndpoints({
    this.webTransport,
    this.webSocket,
    this.certificatePins = const [],
    this.allowLocalPlainWebSocket = false,
  }) {
    if (webTransport == null && webSocket == null) {
      throw ArgumentError('At least one endpoint is required');
    }
    if (webTransport case final url?) {
      if (url.scheme != 'https' || !url.hasPort || !_plainUrl(url)) {
        throw ArgumentError.value(url, 'webTransport', 'Use HTTPS with an explicit port and no URL credentials, query or fragment');
      }
    }
    if (webSocket case final url?) {
      final loopback = _isLoopback(url.host);
      if ((url.scheme != 'wss' && !(url.scheme == 'ws' && loopback && allowLocalPlainWebSocket)) ||
          !_plainUrl(url)) {
        throw ArgumentError.value(url, 'webSocket', 'Use WSS, or explicitly allow WS on localhost');
      }
    }
    if (webTransport != null && webSocket != null &&
        webTransport!.host.toLowerCase() != webSocket!.host.toLowerCase()) {
      throw ArgumentError('WebTransport and fallback WebSocket must share a host');
    }
    if (certificatePins.isNotEmpty && webTransport == null) {
      throw ArgumentError('WebTransport certificate pins require a WebTransport endpoint');
    }
    for (final pin in certificatePins) {
      if (!RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(pin)) {
        throw ArgumentError.value(pin, 'certificatePins', 'Expected sha256:<64 hex digits>');
      }
    }
  }

  final Uri? webTransport;
  final Uri? webSocket;
  final List<String> certificatePins;
  final bool allowLocalPlainWebSocket;

  static bool _plainUrl(Uri url) =>
      url.hasAuthority && url.host.isNotEmpty && url.userInfo.isEmpty &&
      !url.hasQuery && !url.hasFragment;
  static bool _isLoopback(String host) =>
      host == 'localhost' || host == '127.0.0.1' || host == '::1';
}

/// ACP JSON-RPC frames are length-prefixed only *inside a WebTransport
/// stream*. WebSocket delivers one text message per ACP JSON-RPC message.
/// This follows Guatiao's transport framing, not its application protocol.
const maxAgentMessage = 1024 * 1024;

Uint8List frameMessage(String text, {int maxBytes = maxAgentMessage}) {
  final bytes = utf8.encode(text);
  if (bytes.isEmpty || bytes.length > maxBytes) {
    throw const TransportProtocolFailure('Message exceeds frame limits or is empty');
  }
  final frame = Uint8List(4 + bytes.length);
  ByteData.sublistView(frame, 0, 4).setUint32(0, bytes.length, Endian.little);
  frame.setRange(4, frame.length, bytes);
  return frame;
}

/// Accepts fragmented and coalesced WebTransport stream bytes, preserving
/// ordered message boundaries and rejecting over-sized frames.
final class AgentFrameDecoder {
  AgentFrameDecoder({this.maxBytes = maxAgentMessage});
  final int maxBytes;
  final List<int> _buffer = [];

  List<String> add(Uint8List chunk) {
    _buffer.addAll(chunk);
    final decoded = <String>[];
    var offset = 0;
    while (_buffer.length - offset >= 4) {
      final size = _buffer[offset] |
          (_buffer[offset + 1] << 8) |
          (_buffer[offset + 2] << 16) |
          (_buffer[offset + 3] << 24);
      if (size < 1 || size > maxBytes) {
        throw const TransportProtocolFailure('WebTransport frame is empty or exceeds the limit');
      }
      if (_buffer.length - offset < size + 4) break;
      try {
        decoded.add(utf8.decode(_buffer.sublist(offset + 4, offset + 4 + size)));
      } on FormatException {
        throw const TransportProtocolFailure('Invalid UTF-8 in WebTransport frame');
      }
      offset += size + 4;
    }
    if (offset > 0) _buffer.removeRange(0, offset);
    // A single oversized unframed chunk must never accumulate indefinitely.
    if (_buffer.length > maxBytes + 4) {
      throw const TransportProtocolFailure('Incomplete WebTransport frame exceeds limit');
    }
    return decoded;
  }
}
