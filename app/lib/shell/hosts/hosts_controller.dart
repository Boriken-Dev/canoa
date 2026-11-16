import 'dart:async';

import 'package:coqui_acp/page.dart' show AcpPage;
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show BuildContext;

import '../../config/canoa_config.dart';
import '../../config/connection.dart';
import '../../connect_dialog.dart' show ConnectTarget, UseBrowserAgents;
import '../../desktop/control_client.dart' show ControlClient, ControlFailed;
import '../../desktop/host_supervisor.dart' show HostPhase, HostSupervisor;
import '../../session_store.dart' show ChatSession;
import '../../ledger/session_ledger.dart' show SessionPlace;
import '../layout_state.dart';
import 'host_status.dart';

/// Which kind of row the Hosts index lists.
enum HostItemKind {
  /// The machine this app runs on, with the host inside it.
  local,

  /// The host that served this web page.
  served,

  /// A connection the person saved.
  saved,
}

/// One row of the Hosts index.
final class HostItem {
  const HostItem.local() : this._('local', 'Local', HostItemKind.local, null);
  const HostItem.served(String name) : this._('served', name, HostItemKind.served, null);
  HostItem.saved(Connection connection) : this._('connection:${connection.name}', connection.name, HostItemKind.saved, connection);
  const HostItem._(this.id, this.name, this.kind, this.connection);

  final String id;
  final String name;
  final HostItemKind kind;
  final Connection? connection;

  String get kindLabel => switch (kind) {
    HostItemKind.local => 'This computer',
    HostItemKind.served => 'Serves this page',
    HostItemKind.saved => connection!.kind.label,
  };
}

/// What the app's own host, or the host that served the page, says of itself.
final class HostSummary {
  const HostSummary({
    required this.state,
    this.peer,
    this.method,
    this.agents = const [],
    this.capabilities = const [],
    this.problem,
  });

  /// Where the host is: running, starting, off.
  final String state;
  final String? peer;
  final String? method;
  final List<String> agents;
  final List<String> capabilities;
  final String? problem;

  static const _grantLabels = <String, String>{
    'ledger.read': 'Session list',
    'folders': 'Folder browsing',
    'worktree': 'Git worktrees',
    'store.read': 'Store',
    'store.programs': 'Installing agent programs',
    'accounts': 'Accounts',
    'usage': 'Usage',
    'config.read': 'Reading settings',
    'config.write': 'Writing settings',
    'upload': 'Uploads',
    'host.restart': 'Restarting the host',
    'discover': 'Looking nearby',
  };

  /// The capabilities a set of grants stands for, each named once, in a fixed order; nothing the host did not grant.
  static List<String> capabilitiesOf(Set<String> grants) => [
    for (final label in {for (final MapEntry(:key, :value) in _grantLabels.entries) if (grants.contains(key)) value}) label,
  ];

  /// The summary of [supervisor], with the capabilities [grants] stand for when the host has said them.
  static HostSummary ofSupervisor(HostSupervisor supervisor, {Set<String>? grants}) => HostSummary(
    state: switch (supervisor.phase) {
      HostPhase.running => 'Running',
      HostPhase.starting => 'Starting',
      HostPhase.idle => 'Running, hosting nothing',
      HostPhase.off => 'Off',
      HostPhase.absent => 'Not in this build',
      HostPhase.waiting => 'Ended, starting again',
      HostPhase.failed => 'canoa.cfg is wrong for it',
      HostPhase.held => 'Another host holds this home',
      HostPhase.stopped => 'Stopped',
    },
    peer: supervisor.peerId,
    method: supervisor.peerMethod,
    agents: [for (final entry in supervisor.served) entry.title ?? entry.name],
    capabilities: grants == null ? const [] : capabilitiesOf(grants),
  );
}

/// One candidate a search found nearby: the host's claim, which nothing yet trusts.
final class NearbyCandidate {
  const NearbyCandidate({required this.label, required this.url, required this.peer, required this.method, this.offers = const []});

  /// The candidate as `discover/nearby` words it, or null for one that has no address or id.
  static NearbyCandidate? read(Object? json) {
    if (json is! Map) return null;
    final [label, url, peer, method] = [json['label'], json['url'], json['peer'], json['method']];
    if (url is! String || peer is! String) return null;
    return NearbyCandidate(
      label: label is String && label.isNotEmpty ? label : url,
      url: url,
      peer: peer,
      method: method is String ? method : '',
      offers: [if (json['offers'] case final List<Object?> offers) for (final offer in offers) if (offer is String) offer],
    );
  }

  final String label;
  final String url;
  final String peer;
  final String method;
  final List<String> offers;
}

/// The result of looking nearby: what was found, and why a part of the search could not run.
final class NearbyResult {
  const NearbyResult(this.candidates, {this.problem});
  final List<NearbyCandidate> candidates;
  final String? problem;
}

/// Looks nearby through the Canoa on this computer.
typedef NearbySearch = Future<NearbyResult> Function();

/// Asks the host at [client] what announces itself nearby (`discover/nearby`). What comes back is each candidate's claim, which
/// nothing trusts until it is added and connected to; a refusal or a search that could not run is said, not hidden.
Future<NearbyResult> nearbyOf(ControlClient client, {int seconds = 3}) async {
  try {
    final result = await client.call('discover/nearby', {'seconds': seconds}, null, Duration(seconds: seconds + 12));
    if (result is! Map) return const NearbyResult([], problem: 'The host answered with something this app cannot read.');
    final problem = result['networkError'];
    return NearbyResult(
      [
        if (result['candidates'] case final List<Object?> list)
          for (final each in list) ?NearbyCandidate.read(each),
      ],
      problem: problem is String && problem.isNotEmpty ? 'The link could not be searched: $problem' : null,
    );
  } on ControlFailed catch (error) {
    return NearbyResult(const [], problem: error.code == -32601 || error.code == -32001
        ? 'This host does not let this app look nearby.'
        : error.message);
  }
}

/// The Hosts section's state: its rows, the one chosen, what the last look at each found, and the saved connections, which it adds,
/// renames and removes through the app's own save of canoa.cfg.
class HostsController extends ChangeNotifier {
  HostsController({
    required this.layout,
    required this.save,
    required this.sessions,
    this.probe = probeConnection,
    this.nearby,
    this.local,
    this.servedName,
    this.servedSummary,
    this.mayWrite = _always,
    this.start,
    this.addDialog,
    this.browserProvider,
    this.useBrowserAgents,
  });

  static const section = 'hosts';

  final ShellLayout layout;

  /// Writes the changed configuration where the app keeps canoa.cfg, and returns the one the app now holds.
  final Future<void> Function(CanoaConfig changed) save;

  /// The sessions the app has.
  final List<ChatSession> Function() sessions;
  final ConnectionProbe probe;

  /// Looks nearby, when the Canoa on this computer can; null when none is reachable.
  final NearbySearch? nearby;

  /// What this computer's host says of itself; null where this app runs none (a page, the extension).
  final HostSummary Function()? local;

  /// The name of the host that served this page, when one did, and what it says of itself.
  final String? servedName;
  final HostSummary Function()? servedSummary;

  /// Starts a session on a connection, once the person has chosen its workspace.
  final Future<void> Function(ConnectTarget target)? start;

  /// Opens the Add connection dialog; the app's own when none is given.
  final Future<void> Function(BuildContext context)? addDialog;

  /// The agents the browser's provider holds, when this is a browser that has one, and the way to use them.
  final Future<AcpPage?>? browserProvider;
  final UseBrowserAgents? useBrowserAgents;

  /// Whether this app may write saved connections: not a page whose host has not given its key `config.write`.
  final bool Function() mayWrite;

  static bool _always() => true;

  bool get canSave => mayWrite();

  CanoaConfig _config = const CanoaConfig();
  final Map<String, HostStatus> _status = {};
  final Set<String> _looked = {};

  CanoaConfig get config => _config;

  /// Takes the configuration the app holds now; a saved connection that is gone loses its status.
  void adopt(CanoaConfig config) {
    _config = config;
    final names = {for (final connection in config.connections) 'connection:${connection.name}'};
    _status.removeWhere((id, _) => id.startsWith('connection:') && !names.contains(id));
    notifyListeners();
  }

  /// The rows: Local first, then the host that served the page, then the saved connections by name.
  List<HostItem> get items => [
    if (local != null) const HostItem.local(),
    if (servedName != null) HostItem.served(servedName!),
    for (final connection in _config.connections) HostItem.saved(connection),
  ];

  /// The row chosen: the one the layout kept if it is still listed, else the first.
  HostItem? get selected {
    final all = items;
    final kept = layout.of(section).item;
    return all.where((item) => item.id == kept).firstOrNull ?? all.firstOrNull;
  }

  void select(String id) {
    layout.setItem(section, id);
    notifyListeners();
  }

  HostStatus statusOf(HostItem item) => _status[item.id] ?? const HostStatus.unchecked();

  /// Looks at a connection, and at every Canoa host the first time the section is shown: a Canoa host is read from the list it
  /// serves, which connects to nothing; a direct agent is checked only when the person asks.
  Future<void> check(Connection connection) async {
    final id = 'connection:${connection.name}';
    _status[id] = const HostStatus(HostState.checking);
    notifyListeners();
    final found = await probe(connection);
    // The connection may have been removed or renamed meanwhile.
    if (!_config.connections.any((each) => each.name == connection.name)) return;
    _status[id] = found;
    notifyListeners();
  }

  /// Reads each Canoa host not yet read.
  void lookAtHosts() {
    for (final connection in _config.connections) {
      if (connection.kind == ConnectionKind.canoa && _looked.add('connection:${connection.name}')) unawaited(check(connection));
    }
  }

  /// The sessions that are on [connection]'s host or agent.
  List<ChatSession> sessionsOn(Connection connection) => [
    for (final session in sessions())
      if (session.place == SessionPlace.peer && session.peerId == connection.peer) session,
  ];

  /// Saves [connection], in place of the one named [replacing] when that is given. Throws [FormatException] naming what is wrong.
  Future<void> add(Connection connection, {String? replacing}) async {
    if (!canSave) throw const FormatException('This page cannot save connections: its host has not given it config.write.');
    if (replacing != connection.name && _config.connections.any((each) => each.name == connection.name)) {
      throw FormatException('A connection named ${connection.name} is already saved.');
    }
    final changed = _config.withConnection(connection, replacing: replacing);
    await save(changed);
    _config = changed;
    if (replacing != null && replacing != connection.name) {
      final moved = _status.remove('connection:$replacing');
      if (moved != null) _status['connection:${connection.name}'] = moved;
      if (layout.of(section).item == 'connection:$replacing') layout.setItem(section, 'connection:${connection.name}');
    }
    notifyListeners();
    // A Canoa host is read at once, as it is when the section opens; a direct agent waits for the person to check it.
    if (connection.kind == ConnectionKind.canoa && replacing == null) {
      _looked.add('connection:${connection.name}');
      unawaited(check(connection));
    }
  }

  Future<void> rename(Connection connection, String to) {
    if (!Connection.isName(to)) throw const FormatException('A name is 1 to 64 characters, with no control characters.');
    return add(connection.renamed(to), replacing: connection.name);
  }

  /// Removes the saved connection. Nothing is stopped on the host: only this app's record of it goes.
  Future<void> remove(Connection connection) async {
    final changed = _config.withoutConnection(connection.name);
    await save(changed);
    _config = changed;
    _status.remove('connection:${connection.name}');
    if (layout.of(section).item == 'connection:${connection.name}') layout.setItem(section, null);
    notifyListeners();
  }
}
