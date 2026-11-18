import 'dart:async';

import 'package:http/http.dart' as http;

import '../../acp/browser_connect.dart';
import '../../acp/connect.dart' show chatEndpoints;
import '../../acp/served_host.dart';
import '../../auth/peer_proof.dart' show MethodUnavailable, PeerMethod;
import '../../config/connection.dart';
import '../../net/system_http.dart';

/// Where a host stands, as the last look at it found it.
enum HostState {
  /// Not looked at yet: a direct agent is checked only when the person asks, since a check connects to it.
  unchecked('Not checked'),
  checking('Checking'),

  /// It answered, and what it answered holds the pinned id.
  online('Online'),

  /// It did not answer.
  offline('Not reachable'),

  /// It answered with another id than the one pinned: not the host the person saved.
  changed('Another key'),

  /// It answered and turned this app's key away: it has to be paired there.
  refused('Needs pairing');

  const HostState(this.label);
  final String label;
}

/// An agent a host lists: the name it shows and the address a client dials for it.
final class HostAgent {
  const HostAgent(this.name, this.url);
  final String name;
  final String url;
}

/// What a look at a host found: the state, and what the host says of itself. Everything but the state is the host's claim; the proof
/// at connect is what decides whether it holds.
final class HostStatus {
  const HostStatus(this.state, {this.detail, this.peer, this.method, this.agents = const [], this.capabilities = const [], this.served});

  const HostStatus.unchecked() : this(HostState.unchecked);

  final HostState state;

  /// Why, when the state is not a good one.
  final String? detail;

  /// The peer id the host said it holds, and the proof it runs.
  final String? peer;
  final String? method;

  /// The agents it lists.
  final List<HostAgent> agents;

  /// What it says it can do besides run agents, read from it: never assumed.
  final List<String> capabilities;

  /// The list it served, whole, for a Canoa host that answered under the pinned id: its entries, system and control address.
  final ServedHost? served;
}

/// Looks at one saved connection. A host that cannot be reached is a [HostStatus], not an error.
typedef ConnectionProbe = Future<HostStatus> Function(Connection connection);

/// The probe the app uses. A Canoa host is read from the list it serves at its address (`canoa.json`, which names its agents and
/// whether it has a control channel); a direct agent is connected to under the pinned id and the connection closed, which is a
/// check the person asks for since it reaches an agent that may start for it.
Future<HostStatus> probeConnection(Connection connection, {http.Client? client, Duration timeout = const Duration(seconds: 6)}) async {
  try {
    return switch (connection.kind) {
      ConnectionKind.canoa => await _probeHost(connection, client, timeout),
      ConnectionKind.acp => await _probeAgent(connection, timeout),
    };
  } on TimeoutException {
    return const HostStatus(HostState.offline, detail: 'It did not answer in time.');
  } on Object catch (error) {
    return HostStatus(HostState.offline, detail: '$error');
  }
}

Future<HostStatus> _probeHost(Connection connection, http.Client? client, Duration timeout) async {
  final base = Uri.parse(connection.address);
  final list = base.resolve('canoa.json');
  final ask = client ?? systemHttpClient();
  try {
    return await _readHostList(connection, ask, list, timeout);
  } finally {
    if (client == null) ask.close();
  }
}

Future<HostStatus> _readHostList(Connection connection, http.Client ask, Uri list, Duration timeout) async {
  final reply = await ask.get(list).timeout(timeout);
  if (reply.statusCode != 200) return HostStatus(HostState.offline, detail: 'It answered ${reply.statusCode} for its list.');
  final host = ServedHost.parse(reply.body);
  if (host == null) return const HostStatus(HostState.offline, detail: 'That address does not serve a Canoa host\'s list.');
  if (host.peer != connection.peer) {
    return HostStatus(
      HostState.changed,
      detail: 'It says its id is ${host.peer}, not the one saved. Remove the connection and add it again only if you changed that host\'s key.',
      peer: host.peer,
      method: host.method.id,
    );
  }
  return HostStatus(
    HostState.online,
    peer: host.peer,
    method: host.method.id,
    agents: [for (final entry in host.entries) if (entry.kind == 'agent') HostAgent(entry.title ?? entry.name, entry.url)],
    capabilities: [if (host.control != null) 'Management (sessions, folders, files)'],
    served: host,
  );
}

Future<HostStatus> _probeAgent(Connection connection, Duration timeout) async {
  final endpoints = connection.transport == ConnectionTransport.webtransport
      ? chatEndpoints(webSocket: '', webTransport: connection.address)
      : chatEndpoints(webSocket: connection.address, preferWebTransport: false);
  try {
    final agent = await connectBrowserAcp(
      endpoints,
      pinnedPeerId: connection.peer,
      method: PeerMethod.parse(connection.method),
      timeout: timeout,
    );
    await agent.close();
    return HostStatus(HostState.online, peer: connection.peer, method: connection.method);
  } on MethodUnavailable catch (error) {
    return HostStatus(HostState.offline, detail: '$error');
  } on Object catch (error) {
    final text = '$error';
    final refused = RegExp('(not (paired|accepted|trusted))|refused|unauthor', caseSensitive: false).hasMatch(text);
    return HostStatus(refused ? HostState.refused : HostState.offline, detail: text);
  }
}
