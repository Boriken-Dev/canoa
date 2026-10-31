// Guatiao-style platform selection. The browser has WebTransport; the Dart
// VM currently returns an unsupported error, so the connector can fall back.
export 'webtransport_io.dart'
    if (dart.library.js_interop) 'webtransport_web.dart'
    show openAcpWebTransport;
