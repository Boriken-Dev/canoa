import 'package:coqui_transport_web/coqui_transport_web.dart';

import '../desktop/host_control.dart';
import 'system_dial_stub.dart' if (dart.library.io) 'system_dial_io.dart' as platform;

/// The host the app's connections to a remote Canoa host go through, once [useHostDials] names one.
HostControl? _control;

/// From now on [connectSystemChannel] opens its WebSocket by asking `control`'s host to carry it (`network/tunnel`), so the
/// connection follows the system's proxy and its login, and its TLS the system's trust and client certificates, as the host's own
/// requests do. A web page's connections are its browser's and need none of this.
void useHostDials(HostControl control) {
  _control = control;
}

/// Back to the platform's own dial, for a test.
void resetSystemDial() {
  _control = null;
}

/// [connectWebChannel] for the connections the app makes for itself: to a Canoa host it was told about, to its control channel.
///
/// With no host to carry them, or a caller's own [webSocketDialer], it is [connectWebChannel]. Otherwise a `wss` WebSocket is made by
/// the host, which reaches the server through the system's proxy (logging in as the system's other traffic does) or straight, and
/// makes the TLS itself with the operating system's trust and the client certificate its keystores hold; a `ws` one is tunnelled
/// through a proxy the system names, and opened by the app itself where the system names none. A connection to this computer is
/// the app's own. WebTransport, which no proxy carries, is not available to the Dart VM, so the WebSocket endpoint is what is
/// dialled; a browser keeps its own networking.
Future<WebChannel> connectSystemChannel(
  WebEndpoints endpoints, {
  Duration timeout = const Duration(seconds: 10),
  WebDialer? webTransportDialer,
  WebDialer? webSocketDialer,
}) {
  final control = _control;
  return connectWebChannel(
    endpoints,
    timeout: timeout,
    webTransportDialer: webTransportDialer,
    webSocketDialer: webSocketDialer ?? (control == null ? null : platform.hostWebSocketDialer(control)),
  );
}
