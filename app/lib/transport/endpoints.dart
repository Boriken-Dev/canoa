/// Advertised transport endpoints of one logical agent peer.
///
/// These must come from a verified/discovered peer descriptor. The URL is a
/// location, NOT a peer identity or authorization grant.
final class AcpEndpoints {
  AcpEndpoints({
    required this.webSocket,
    this.webTransport,
    this.certificatePins = const [],
    this.allowLocalPlaintext = false,
  }) {
    _check(webSocket, kind: 'WebSocket');
    if (webTransport != null) _check(webTransport!, kind: 'WebTransport');
    for (final pin in certificatePins) {
      if (!RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(pin)) {
        throw ArgumentError.value(pin, 'certificatePins', 'Expected a SHA-256 certificate fingerprint');
      }
    }
  }

  final Uri webSocket;
  final Uri? webTransport;

  /// WebTransport browser server certificate fingerprints, not peer-auth.
  final List<String> certificatePins;

  /// Only permits ws:// on loopback for deliberate local development.
  final bool allowLocalPlaintext;

  void _check(Uri url, {required String kind}) {
    if (!url.hasAuthority || url.host.isEmpty || url.userInfo.isNotEmpty ||
        url.hasQuery || url.hasFragment) {
      throw ArgumentError.value(url, kind, 'Endpoint requires a host without credentials, query or fragment');
    }
    if (kind == 'WebTransport') {
      if (url.scheme != 'https') {
        throw ArgumentError.value(url, kind, 'WebTransport requires HTTPS');
      }
    } else if (url.scheme != 'wss' &&
        !(url.scheme == 'ws' && allowLocalPlaintext && _loopback(url.host))) {
      throw ArgumentError.value(url, kind, 'WebSocket requires WSS, except explicit loopback WS');
    }
  }

  static bool _loopback(String host) =>
      host == 'localhost' || host == '127.0.0.1' || host == '::1';
}
