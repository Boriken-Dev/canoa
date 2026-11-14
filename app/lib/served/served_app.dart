import 'dart:async';

import '../acp/served_host.dart';
import '../config/canoa_config.dart';
import '../desktop/host_control.dart';
import '../desktop/host_supervisor.dart';
import '../desktop/host_tool.dart';
import '../ledger/host_ledger_backend.dart';
import '../ledger/session_ledger.dart' show LedgerBackend;
import '../profile/aos_support.dart' show ReadTree;
import '../profile/profile_home.dart';
import '../profile/project_trust.dart';
import 'control_overlays.dart';
import 'host_config.dart';
import 'host_files.dart';
import 'served_control.dart';

/// The entries of the host that served this page as the app's sources: each agent entry it lists, reached by its address after the
/// key proof, as a host that listens on the network is. The page runs nothing itself, so there is no process to start, restart
/// or stop here; [phase] is `running` while the page is open.
final class ServedSupervisor extends HostSupervisor {
  ServedSupervisor(this._served) : super(null);

  final ServedHost _served;

  @override
  HostPhase get phase => HostPhase.running;

  @override
  List<HostListening> get listening => [
    for (final entry in _served.entries)
      if (entry.kind == 'agent')
        HostListening(
          name: entry.name,
          kind: entry.kind,
          url: entry.url,
          peer: _served.peer,
          method: _served.method.id,
          harness: entry.harness,
          title: entry.title,
        ),
  ];

  @override
  List<HostListening> get served => listening;

  @override
  String? get peerId => _served.peer;

  @override
  String? get peerMethod => _served.method.id;

  @override
  Future<void> apply({required String? configPath}) async {}

  @override
  Future<void> restart() async {}

  @override
  Future<void> stop() async {}
}

/// What the overlay store and Loadout need of the host's computer, with the store's git work done by the host over its control
/// channel. The host's files are not readable here, so a layer's tree is left out of what a page shows of the profile.
final class ServedProfileHome implements ProfileHome {
  ServedProfileHome(this.overlays)
      : trust = ProjectTrust(
          read: () async => null,
          write: (_) async => throw StateError('A project profile is trusted from the host\'s own computer, not from a web page'),
        );

  @override
  final ControlOverlayManager overlays;

  @override
  final ProjectTrust trust;

  @override
  String get configDir => '';

  @override
  String get os => 'linux';

  @override
  String join(String dir, String relative) => dir.isEmpty ? relative : '$dir/$relative';

  @override
  String? canonicalDirectory(String directory) => null;

  @override
  Future<List<int>?> readBytes(String path) async => null;

  @override
  Future<ReadTree> readTree(String directory, {int maxFiles = 4096, int maxBytes = 64 * 1024 * 1024}) =>
      throw StateError('A web page does not read the host\'s files');

  @override
  Future<String?> readSecret(String path, {String? within}) async => null;
}

/// The host that served this page, as the app uses it: its sources, and over its control channel the session list, folders,
/// uploads, canoa.cfg and the overlay store. The control channel is the one the host lists; a key it has no grant for gets the
/// host's own refusal for that request, and nothing else is lost.
final class ServedApp {
  ServedApp._(this.host, this.control)
      : supervisor = ServedSupervisor(host),
        overlays = ControlOverlayManager(control),
        ledger = HostLedgerBackend.over(control);

  /// The page's host as `canoa.json` lists it, with a control channel of the page's own when the host listens for one.
  ServedApp.over(ServedHost host, HostControl control) : this._(host, control);

  /// Reads `canoa.json` beside this page (or beside [base]): null where the page was not served by a `canoa`.
  static Future<ServedApp?> discover({HostControl Function(ServedHost host)? control, Uri? base}) async {
    final host = await loadServedHost(base: base);
    if (host == null) return null;
    return ServedApp.over(host, (control ?? servedControl)(host));
  }

  final ServedHost host;
  final HostControl control;
  final ServedSupervisor supervisor;
  late final HostFiles files = HostFiles(control);
  late final HostConfigStore configStore = HostConfigStore(files);
  final ControlOverlayManager overlays;
  late final ServedProfileHome home = ServedProfileHome(overlays);

  /// The session list the host keeps for this page's key.
  final LedgerBackend ledger;

  /// Why the control channel cannot be used, once [probe] has found out; null while it can or has not been asked.
  String? problem;

  /// Whether the control channel was reached and the key proved: false when the host lists none, this page's key is not accepted
  /// there, or it grants the key nothing.
  bool get reachable => problem == null;

  /// Dials the control channel once, so that a page whose key the host has not accepted learns it at the start, not at its first
  /// folder or message. Never throws: [problem] says what is wrong.
  Future<void> probe({Duration within = const Duration(seconds: 12)}) async {
    if (host.control == null) {
      problem = 'This host listens for no control channel, so the page keeps its sessions in this browser only.';
      return;
    }
    try {
      // A key the host has not accepted, or has granted nothing, is turned away after the proof: only a request shows it.
      await (await control.ready()).call('ledger/list', {'limit': 1}).timeout(within);
      problem = null;
    } on Object catch (error) {
      problem = 'The host\'s control channel cannot be used: $error. Pair this browser with the host, give its key the grants in '
          'canoa.cfg\'s [host.control], and reload.';
    }
  }

  /// canoa.cfg as the host holds it, or defaults and [problem] when it cannot be read.
  Future<CanoaConfig> loadConfig() async {
    try {
      return await configStore.load();
    } on Object catch (error) {
      problem ??= 'The host\'s canoa.cfg cannot be read: $error';
      return const CanoaConfig();
    }
  }

  /// Reads which overlays the host has installed, which the store shows as installed.
  Future<void> refreshOverlays() async {
    try {
      await overlays.refresh();
    } on Object catch (error) {
      problem ??= 'The host\'s overlays cannot be read: $error';
    }
  }

  Future<void> close() => control.close();
}
