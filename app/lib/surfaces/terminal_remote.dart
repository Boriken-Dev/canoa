import 'package:http/http.dart' as http;

import '../acp/served_host.dart';
import '../brand.dart';
import '../config/connection.dart';
import '../desktop/host_control.dart';
import '../desktop/host_grants.dart';
import '../served/served_control.dart';
import 'terminal_controller.dart';
import 'terminal_surface.dart';

/// The control channel of a host reached at the address its list names.
typedef ControlOfServedHost = HostControl Function(ServedHost host);

/// What the Canoa host saved as [connection] offers of a shell, asked of the host itself: its list names its control address, the
/// channel proves its key against the one saved, and `host/grants` says whether this app's key holds `terminal`. Every way it can
/// say no is a [TerminalRefused] with the reason, so the Terminal section shows it instead of a shell that never opens.
Future<TerminalAccess> terminalOnConnection(
  Connection connection, {
  http.Client? client,
  ControlOfServedHost control = servedControl,
}) async {
  final name = connection.name;
  if (connection.kind != ConnectionKind.canoa) {
    return TerminalRefused('$name is a direct agent endpoint: it offers a session and no shell.');
  }
  final host = await loadServedHost(base: Uri.tryParse(connection.address), client: client);
  if (host == null) return TerminalRefused('$name did not answer with its list of what it serves, so no shell can be asked of it.');
  if (host.peer != connection.peer) {
    return TerminalRefused('$name says its id is not the one saved. Remove the connection and add it again only if you changed that host\'s key.');
  }
  if (host.control == null) return TerminalRefused('$name listens for no control channel, so it offers no shell.');
  final channel = control(host);
  final grants = HostGrants(channel.ready);
  try {
    await grants.refresh();
  } on Object {
    // refresh keeps what it knew when the host cannot be reached; the unknown state below says so.
  }
  if (!grants.known) {
    await channel.close();
    return TerminalRefused('$name could not be reached with this app\'s key. Pair this app with it first, then try again.');
  }
  if (!grants.may(HostGrants.terminal)) {
    await channel.close();
    return TerminalRefused('$name has not given this app a terminal. Its ${Brand.configFile} names the keys that may have one.');
  }
  return TerminalOnHost(ControlTerminals(channel));
}
