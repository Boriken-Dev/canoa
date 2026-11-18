import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:coqui_transport_web/coqui_transport_web.dart';
import 'package:web_socket_channel/io.dart';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// A WebSocket dialler that asks the host `control` reaches to carry each connection: it logs in to the system's proxy for it, and
/// for a secure address makes the TLS itself (the operating system's trust and the client certificate its keystores hold, as for
/// its own requests), so the app talks to the host's loopback end as if it were the server and its own roots never decide. The
/// connection is the app's only if it begins with the token the host gave. A name the host does not verify is no connection.
/// Where the system has no proxy for an address that needs no TLS the host says so and the app connects itself, as it did before.
WebDialer hostWebSocketDialer(HostControl control) => (uri, timeout, pins) async {
  final client = HttpClient()
    // The host decides, and takes the environment's proxy variables into account itself.
    ..findProxy = ((_) => 'DIRECT')
    ..connectionFactory = (url, proxyHost, proxyPort) => _connect(control, url, timeout);
  try {
    return await openWebSocketChannel(
      uri,
      timeout,
      pins,
      connector: (target, protocols) => IOWebSocketChannel.connect(target, protocols: protocols, customClient: client),
    );
  } finally {
    // The upgraded socket is no longer the client's; this ends only what it had left.
    client.close();
  }
};

/// The socket for a request to [url] (`http` or `https`: how `dart:io` names a WebSocket's address).
Future<ConnectionTask<Socket>> _connect(HostControl control, Uri url, Duration timeout) async {
  final secure = url.scheme == 'https';
  final host = url.host;
  final port = url.port;
  if (_isLoopback(host)) return _direct(host, port, secure, timeout);
  var cancelled = false;
  final opened = () async {
    final Object? reply;
    try {
      final link = await control.ready();
      reply = await link.call(
        'network/tunnel',
        {
          'url': Uri(scheme: url.scheme, host: host, port: port, path: '/').toString(),
          'seconds': timeout.inSeconds < 1 ? 1 : timeout.inSeconds,
        },
        null,
        timeout + const Duration(seconds: 10),
      );
    } on ControlFailed catch (error) {
      throw SocketException(error.message);
    } on TimeoutException {
      throw const SocketException('The host did not answer in time.');
    } on Object catch (error) {
      throw SocketException('The host that carries the app\'s connections cannot be reached: $error');
    }
    final answer = reply is Map<Object?, Object?> ? reply : const <Object?, Object?>{};
    if (answer['direct'] == true) {
      // Only for an address that needs no TLS: the host makes the TLS to a server itself, never leaves it to this library's roots.
      if (secure) throw const SocketException('The host left the TLS of a secure connection to the app.');
      return _open(host, port, secure, timeout);
    }
    final tunnel = answer['port'];
    final token = answer['token'];
    if (tunnel is! int || token is! String) {
      throw const SocketException('The host answered a connection with something that is not a reply.');
    }
    if (secure) {
      // The host has made the TLS and verified the server for this name; a reply that says otherwise is not carried on.
      final server = answer['tls'];
      final name = server is Map<Object?, Object?> ? server['name'] : null;
      if (name is! String || name.toLowerCase() != host.toLowerCase()) {
        throw const SocketException('The host did not verify the server of a secure connection.');
      }
    }
    final plain = await Socket.connect(InternetAddress.loopbackIPv4, tunnel, timeout: timeout);
    // The host carries only the connection that begins with the token it gave, so no other process on this computer is.
    plain.add(utf8.encode(token));
    return plain;
  }();
  // A connection that arrives after the dial gave up is closed, not left open.
  unawaited(
    opened.then<void>((socket) {
      if (cancelled) socket.destroy();
    }, onError: (_) {}),
  );
  return ConnectionTask.fromSocket(opened, () => cancelled = true);
}

Future<ConnectionTask<Socket>> _direct(String host, int port, bool secure, Duration timeout) async =>
    ConnectionTask.fromSocket(_open(host, port, secure, timeout), () {});

Future<Socket> _open(String host, int port, bool secure, Duration timeout) =>
    secure ? SecureSocket.connect(host, port, timeout: timeout) : Socket.connect(host, port, timeout: timeout);

/// This computer's own address: the app's host or an agent on it, which no proxy is asked about.
bool _isLoopback(String host) {
  if (host.toLowerCase() == 'localhost') return true;
  return InternetAddress.tryParse(host)?.isLoopback ?? false;
}
