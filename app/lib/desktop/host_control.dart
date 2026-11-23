import 'dart:async';

import 'package:flutter/foundation.dart' show visibleForTesting;

import 'control_client.dart';
import 'host_supervisor.dart';
import 'host_tool.dart';

/// One link to the host's control channel, shared by whatever needs it and opened again when the host has been started anew.
final class HostControl {
  /// The control channel of the host this app runs in its own process.
  HostControl(HostSupervisor this._host, {this.wait = const Duration(seconds: 20)}) : _dial = null;

  /// The control channel of a host reached over the network, as a web page reaches the one that served it: [dial] opens a link,
  /// proving the host's key first. The host is taken to be up; a link that cannot be had fails the request.
  HostControl.dialing(Future<HostLink> Function() this._dial, {this.wait = const Duration(seconds: 20)}) : _host = null;

  final HostSupervisor? _host;
  final Future<HostLink> Function()? _dial;

  /// How long the host has to be up before a request gives up.
  final Duration wait;

  /// How long to wait before each new try at a link the host ended, the last repeated, and how many tries in all.
  static const retryAfter = [Duration(milliseconds: 500), Duration(seconds: 1), Duration(seconds: 2), Duration(seconds: 4), Duration(seconds: 8), Duration(seconds: 15)];
  static const retries = 20;

  ControlClient? _client;
  Future<ControlClient>? _opening;
  bool _waits = true;
  bool _closed = false;
  bool _had = false;
  Timer? _retry;
  final _again = StreamController<void>.broadcast();

  /// Fires each time a link is made after an earlier one ended: the host started again or the connection came back, so what was
  /// read from it before (accounts, grants, limits) may have changed. While something listens, a link the host ended is tried
  /// again with growing pauses, so a page that is only waiting still hears.
  Stream<void> get reconnected => _again.stream;

  Future<ControlClient> _connect() => _opening ??= (() async {
    final client = _client = await open(wait: _waits);
    _watch(client);
    if (_had && !_closed) _again.add(null);
    _had = true;
    return client;
  })().whenComplete(() => _opening = null);

  void _watch(ControlClient client) {
    unawaited(client.ended.then((_) {
      if (_closed || !identical(_client, client)) return;
      _client = null;
      if (_again.hasListener) _tryAgain(0);
    }));
  }

  void _tryAgain(int attempt) {
    if (_closed || attempt >= retries || _client != null || _opening != null) return;
    _retry?.cancel();
    _retry = Timer(retryAfter[attempt < retryAfter.length ? attempt : retryAfter.length - 1], () async {
      try {
        await ready(wait: false);
      } on Object {
        _tryAgain(attempt + 1);
      }
    });
  }

  /// Says that the host was just asked to restart, so that a link a request needs is dialled again, for [within], while the host
  /// starts, rather than failing the request at the first dial that times out. A host that listens through a portal takes longer to
  /// answer than one dial waits for: the portal holds the connection until the host it starts is up.
  void expectRestart([Duration within = const Duration(seconds: 90)]) {
    _restartUntil = DateTime.now().add(within);
  }

  DateTime? _restartUntil;

  /// How long to pause between dials while a restart is expected.
  @visibleForTesting
  static Duration dialPause = const Duration(seconds: 1);

  /// A link of its own, which the caller closes: closing it ends what the host is doing for it, such as an install.
  Future<ControlClient> open({bool wait = true}) async {
    final dial = _dial;
    if (dial != null) {
      while (true) {
        try {
          final link = await dial().timeout(this.wait);
          _restartUntil = null;
          return ControlClient(link);
        } on Object {
          final until = _restartUntil;
          if (until == null || !DateTime.now().isBefore(until)) rethrow;
          await Future<void>.delayed(dialPause);
        }
      }
    }
    final host = _host!;
    if (!wait && host.phase != HostPhase.running && host.phase != HostPhase.idle) {
      throw StateError(host.problem ?? 'the host is not running');
    }
    final deadline = DateTime.now().add(this.wait);
    while (host.phase != HostPhase.running && host.phase != HostPhase.idle) {
      if (DateTime.now().isAfter(deadline) || host.phase == HostPhase.failed || host.phase == HostPhase.held) {
        throw StateError(host.problem ?? 'the host is not running');
      }
      await Future<void>.delayed(const Duration(milliseconds: 25));
    }
    return ControlClient(host.openLink('@control'));
  }

  /// A client whose link is up.
  Future<ControlClient> ready({bool wait = true}) async {
    final client = _client;
    // A link ends with the host, which the app may start again.
    if (client != null && !client.over) return client;
    _client = null;
    _waits = wait;
    return _connect();
  }

  Future<void> close() async {
    _closed = true;
    _retry?.cancel();
    final client = _client;
    unawaited(_again.close());
    await client?.close();
  }
}
