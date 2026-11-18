import 'package:coqui_transport_web/coqui_transport_web.dart';

import '../desktop/host_control.dart';

/// A page's WebSockets are its browser's: there is nothing to carry them through.
WebDialer? hostWebSocketDialer(HostControl control) => null;
