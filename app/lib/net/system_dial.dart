import 'package:coqui_transport_web/coqui_transport_web.dart';

import '../desktop/host_control.dart';
import 'system_dial_stub.dart' if (dart.library.io) 'system_dial_io.dart' as platform;

/// The host the app's connections to a remote Canoa host go through, once [useHostDials] names one.
HostControl? _control;

/// From now on [connectSystemChannel] opens its WebSocket by asking `control`'s host to carry it (`network/tunnel`), so the
/// connection follows the system's proxy and its login as the host's own requests do. A web page's connections are its browser's
/// and need none of this.
void useHostDials(HostControl control) {
  _control = control;
}

/// Back to the platform's own dial, for a test.
void resetSystemDial() {
  _control = null;
}

/// [connectWebChannel] for the connections the app makes for itself: to a Canoa host it was told about, to its control channel.
///
/// With no host to carry them, or a caller's own [webSocketDialer], it is [connectWebChannel]. Otherwise a WebSocket to a computer
/// the system reaches through a proxy is tunnelled by the host, which logs in to the proxy as the system's other traffic does; one
/// the system reaches directly is opened by the app itself, as before, and so is one to this computer. WebTransport, which no proxy
/// carries, is not available to the Dart VM, so the WebSocket endpoint is what is dialled; a browser keeps its own networking.
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
