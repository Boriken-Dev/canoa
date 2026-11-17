import 'dart:async';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:http/http.dart' as http;

import '../auth/peer_proof.dart' show PeerMethod;
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
import 'host_state.dart';
import 'portal_session.dart';
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
  ServedProfileHome(this.overlays, {required this.os})
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
  final String os;

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
  ServedApp._(this.host, this.control, this.portal)
      : supervisor = ServedSupervisor(host),
        overlays = ControlOverlayManager(control),
        ledger = HostLedgerBackend.over(control);

  /// The page's host as `canoa.json` lists it, with a control channel of the page's own when the host listens for one; [portal]
  /// when `canoa portal` served the page.
  ServedApp.over(ServedHost host, HostControl control, {PortalSession? portal}) : this._(host, control, portal);

  /// Reads `canoa.json` beside this page (or beside [base]): null where the page was not served by a `canoa`, and, behind a
  /// portal, where nobody is signed in (the browser is sent to sign in).
  static Future<ServedApp?> discover({
    HostControl Function(ServedHost host)? control,
    Uri? base,
    http.Client? client,
    void Function(Uri login)? signedOut,
    void Function(Uri address)? navigate,
    Future<String> Function(PeerMethod method)? localPeer,
  }) async {
    final host = await loadServedHost(base: base, client: client, signedOut: signedOut);
    if (host == null) return null;
    final user = host.user;
    final portal = user == null
        ? null
        : PortalSession(user: user, base: base ?? Uri.base, client: client, navigate: navigate, localPeer: localPeer);
    return ServedApp.over(host, (control ?? (host) => servedControl(host, failed: portal?.active))(host), portal: portal);
  }

  final ServedHost host;

  /// The sign-in the portal that served this page keeps, or null for a page served by anything else.
  final PortalSession? portal;
  final HostControl control;
  final ServedSupervisor supervisor;
  late final HostFiles files = HostFiles(control);
  late final HostConfigStore configStore = HostConfigStore(files);

  /// The documents the host keeps for the person: drafts, the layout and the permission rules with their log.
  late final HostState state = HostState(control);
  final ControlOverlayManager overlays;
  late final ServedProfileHome home = ServedProfileHome(overlays, os: host.os ?? '');

  /// The session list the host keeps for this page's key.
  final LedgerBackend ledger;

  /// Why the control channel cannot be used, once [probe] has found out; null while it can or has not been asked. Always one of a
  /// few fixed sentences: what the host or the network said is in [detail] and the log, never in the page's words.
  String? problem;

  /// What went wrong, as the failure said it: for the log, since it may come from an endpoint this page has not proven.
  String? detail;

  /// Why canoa.cfg was not read from the host, when it was not; the page then writes nothing to the host's file.
  String? configProblem;

  /// Whether canoa.cfg was read from the host whole, which is what every write to it is made against.
  bool get configLoaded => configStore.loaded;

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
    // A key the host has not accepted, or has granted nothing, is turned away after the proof: only a request shows it.
    Future<void> ask() async => (await control.ready()).call('ledger/list', {'limit': 1}).timeout(within);
    try {
      // Behind a portal the page introduces its own key after the first refusal, then asks again.
      await (portal == null ? ask() : portal!.connect(host.method, ask));
      problem = null;
    } on PortalSignedOut {
      problem = 'You are signed out: going to the sign-in page.';
    } on Object catch (error) {
      detail = '$error';
      debugPrint('Canoa: the host\'s control channel cannot be used: $error');
      problem = 'The host\'s control channel cannot be used. Pair this browser with the host, give its key the grants in '
          'canoa.cfg\'s [host.control], and reload.';
    }
  }

  /// canoa.cfg as the host holds it, or null, with [configProblem] set, when it cannot be read or this page cannot take a table of
  /// it whole: nothing is then written to the host's file.
  Future<CanoaConfig?> loadConfig() async {
    try {
      return await configStore.load();
    } on Object catch (error) {
      debugPrint('Canoa: the host\'s canoa.cfg cannot be read: $error');
      configProblem = 'The host\'s canoa.cfg could not be read whole by this page, so changes in Settings stay in this page and '
          'nothing is written to the host\'s file.';
      return null;
    }
  }

  /// Reads which overlays the host has installed, which the store shows as installed.
  Future<void> refreshOverlays() async {
    try {
      await overlays.refresh();
    } on Object catch (error) {
      debugPrint('Canoa: the host\'s overlays cannot be read: $error');
      configProblem ??= 'The host\'s installed overlays could not be read.';
    }
  }

  Future<void> close() => control.close();
}
