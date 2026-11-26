import 'dart:async';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:http/http.dart' as http;

import '../auth/peer_proof.dart' show PeerMethod;
import '../acp/served_host.dart';
import '../brand.dart';
import '../config/canoa_config.dart';
import '../desktop/control_client.dart' show ControlClient, ControlFailed, ControlListener;
import '../desktop/host_control.dart';
import '../desktop/host_supervisor.dart';
import '../desktop/host_tool.dart';
import '../ledger/host_ledger_backend.dart';
import '../ledger/session_ledger.dart' show LedgerBackend;
import '../workspace/open_in.dart' show WorkspaceOpener;
import '../workspace/open_in_host.dart' show HostedOpening;
import 'web_mode.dart';
import 'control_overlays.dart';
import 'control_profile_home.dart';
import 'host_config.dart';
import 'host_files.dart';
import 'host_state.dart';
import 'local_admission.dart';
import 'app_base.dart';
import 'portal_session.dart';
import 'served_control.dart';

/// The entries of the host that served this page as the app's sources: each agent entry it lists, reached by its address after the
/// key proof, as a host that listens on the network is. The page runs nothing itself, so there is no process to start, restart
/// or stop here; [phase] is `running` while the page is open.
final class ServedSupervisor extends HostSupervisor {
  ServedSupervisor(this._served, [this._control]) : super(null);

  final ServedHost _served;
  final HostControl? _control;

  HostControl get _host => _control ?? (throw const HostCommandFailed('this page has no control channel to its host'));

  @override
  String get runningSaid => 'Serving this page.';

  /// The host's keys, PINs and `[host]` are its own to answer for: this asks it, and it refuses a key that holds no `hosting`.
  @override
  bool get managesKeys => _control != null;

  Future<Map<Object?, Object?>> _call(String method, [Map<String, Object?> params = const {}]) async {
    final result = await (await _host.ready()).call(method, params);
    return result is Map<Object?, Object?> ? result : const {};
  }

  Future<T> _asked<T>(Future<T> Function() ask) async {
    try {
      return await ask();
    } on ControlFailed catch (error) {
      throw HostCommandFailed(error.message);
    }
  }

  @override
  Future<List<HostKey>> keys() => _asked(() async {
    final listed = (await _call('host/keys'))['keys'];
    final keys = [
      if (listed is List<Object?>)
        for (final each in listed)
          if (each is Map<Object?, Object?> && each['id'] is String && (each['state'] == 'accepted' || each['state'] == 'waiting'))
            HostKey(id: each['id']! as String, accepted: each['state'] == 'accepted', from: each['source'] as String?),
    ];
    return [...keys.where((key) => key.accepted), ...keys.where((key) => !key.accepted)];
  });

  @override
  Future<void> accept(String id) => _asked(() => _call('host/accept', {'id': id}));

  @override
  Future<void> revoke(String id) => _asked(() => _call('host/revoke', {'id': id}));

  @override
  Future<HostPairing> pair() => _pairing(native: false);

  @override
  Future<HostPairing> pairBrowser() => _pairing(native: true);

  /// `host/pair` on a link of its own, which is what ends it: the host stops showing the PIN when the link closes.
  Future<HostPairing> _pairing({required bool native}) async {
    final ControlClient client;
    try {
      client = await _host.open();
    } on Object catch (error) {
      throw HostCommandFailed('$error');
    }
    final shown = Completer<Map<String, Object?>>();
    final paired = Completer<String>();
    Future<void>? ending;
    Future<void> end() => ending ??= client.close();
    void fail(Object error) {
      final why = HostCommandFailed(error is ControlFailed ? error.message : '$error');
      if (!shown.isCompleted) shown.completeError(why);
      if (!paired.isCompleted) paired.completeError(why);
    }
    // Nobody may be waiting on it when it is cancelled.
    unawaited(paired.future.then((_) {}, onError: (Object _) {}));
    unawaited(
      client
          .call(
            'host/pair',
            {if (native) 'native': true},
            ControlListener(
              onNotification: (method, params) {
                if (method == 'host/progress' && !shown.isCompleted) shown.complete(params);
              },
            ),
            const Duration(minutes: 6),
          )
          .then((result) {
            final who = result is Map ? result['paired'] : null;
            if (who is String && !paired.isCompleted) {
              paired.complete(who);
            } else {
              fail('the host said no key paired');
            }
          }, onError: fail)
          .whenComplete(end),
    );
    final first = await shown.future.timeout(
      const Duration(seconds: 20),
      onTimeout: () {
        unawaited(end());
        throw const HostCommandFailed('the host did not show a PIN');
      },
    );
    final [pin, hostId, url] = [first['pin'], first['peer'], first['url'] ?? ''];
    if (pin is! String || hostId is! String || url is! String) {
      unawaited(end());
      throw const HostCommandFailed('the host showed no PIN');
    }
    final method = first['method'];
    return HostPairing.over(
      pin: pin,
      hostId: hostId,
      url: url,
      method: method is String ? method : null,
      native: native,
      paired: paired.future,
      cancel: () => unawaited(end()),
    );
  }

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

/// The host that served this page, as the app uses it: its sources, and over its control channel the session list, folders,
/// uploads, canoa.cfg and the overlay store. The control channel is the one the host lists; a key it has no grant for gets the
/// host's own refusal for that request, and nothing else is lost.
final class ServedApp {
  ServedApp._(this.host, this.control, this.portal, this.admission, Uri? page)
      : page = page ?? Uri.base,
        supervisor = ServedSupervisor(host, control),
        overlays = ControlOverlayManager(control),
        ledger = HostLedgerBackend.over(control);

  /// The page's host as `canoa.json` lists it, with a control channel of the page's own when the host listens for one; [portal]
  /// when `canoa portal` served the page.
  ServedApp.over(ServedHost host, HostControl control, {PortalSession? portal, LocalAdmission? admission, Uri? page})
      : this._(host, control, portal, admission, page);

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
        : PortalSession(user: user, base: base ?? appBase(), client: client, navigate: navigate, localPeer: localPeer);
    final path = host.admit;
    final admission = portal != null || path == null
        ? null
        : LocalAdmission(base: base ?? Uri.base, path: path, client: client, localPeer: localPeer);
    return ServedApp.over(
      host,
      (control ?? (host) => servedControl(host, failed: portal?.active))(host),
      portal: portal,
      admission: admission,
      page: base,
    );
  }

  final ServedHost host;

  /// The address this page was reached at.
  final Uri page;

  /// Where the person sits: what the host says, else local for a page reached over loopback and remote for any other name. What
  /// needs the host's own computer (an editor, a file manager) branches on this.
  late final WebMode mode = WebMode.of(host.web, page);

  /// Open in for this page by [mode]: the host starts the program, or the page links VS Code Remote. Call [HostedOpening.load] once
  /// the control channel is known to work.
  late final HostedOpening openIn = HostedOpening(mode: mode, opening: files, hostPeer: host.peer, page: page);

  /// The workspace actions this page offers.
  late final WorkspaceOpener opener = WorkspaceOpener.hosted(openIn);

  /// The sign-in the portal that served this page keeps, or null for a page served by anything else.
  final PortalSession? portal;

  /// How this page's key is accepted by a host that runs on the client's own computer and serves the page itself, or null.
  final LocalAdmission? admission;
  final HostControl control;
  final ServedSupervisor supervisor;
  late final HostFiles files = HostFiles(control);
  late final HostConfigStore configStore = HostConfigStore(files);

  /// The documents the host keeps for the person: drafts, the layout and the permission rules with their log.
  late final HostState state = HostState(control);
  final ControlOverlayManager overlays;
  late final ControlProfileHome home = ControlProfileHome(control, overlays: overlays, os: host.os ?? '');

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
      await (portal != null
          ? portal!.connect(host.method, ask)
          : admission != null
              ? admission!.connect(host.method, ask)
              : ask());
      problem = null;
    } on PortalSignedOut {
      problem = 'You are signed out: going to the sign-in page.';
    } on Object catch (error) {
      detail = '$error';
      debugPrint('${Brand.name}: the host\'s control channel cannot be used: $error');
      problem = 'The host\'s control channel cannot be used. Pair this browser with the host, give its key the grants in '
          '${Brand.configFile}\'s [host.control], and reload.';
    }
  }

  /// canoa.cfg as the host holds it, or null, with [configProblem] set, when it cannot be read or this page cannot take a table of
  /// it whole: nothing is then written to the host's file.
  Future<CanoaConfig?> loadConfig() async {
    try {
      return await configStore.load();
    } on Object catch (error) {
      debugPrint('${Brand.name}: the host\'s ${Brand.configFile} cannot be read: $error');
      configProblem = 'The host\'s ${Brand.configFile} could not be read whole by this page, so changes in Settings stay in this page and '
          'nothing is written to the host\'s file.';
      return null;
    }
  }

  /// Reads which overlays the host has installed, which the store shows as installed.
  Future<void> refreshOverlays() async {
    try {
      await overlays.refresh();
    } on Object catch (error) {
      debugPrint('${Brand.name}: the host\'s overlays cannot be read: $error');
      configProblem ??= 'The host\'s installed overlays could not be read.';
    }
  }

  Future<void> close() => control.close();
}
