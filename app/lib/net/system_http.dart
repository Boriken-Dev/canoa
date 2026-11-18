import 'package:http/http.dart' as http;

import '../desktop/host_control.dart';
import 'host_http_client.dart';
import 'system_dial.dart';

/// The client the app's own requests use on the desktop: the ones it makes for itself to a Canoa host it was told about (the list
/// the host serves, a probe of a saved connection), and not the traffic of its agents or a served page's, which are the host's and
/// the browser's.
///
/// By default it is the platform's client. A desktop app that runs a host calls [useHostNetwork], after which every client made
/// here sends through that host, so the requests follow the system's proxy, proxy login, certificate verifier and client
/// certificates as the host's own do. A caller that passes a client of its own keeps it.
http.Client systemHttpClient() => _make();

http.Client Function() _make = _platform;

http.Client _platform() => http.Client();

/// Sends the app's own requests through the host `control` reaches, from now on, and carries its WebSocket connections to a
/// remote Canoa host through it too (`connectSystemChannel`).
void useHostNetwork(HostControl control) {
  _make = () => HostHttpClient(control);
  useHostDials(control);
}

/// Back to the platform's client, for a test.
void resetSystemHttp() {
  _make = _platform;
  resetSystemDial();
}
