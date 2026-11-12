import 'dart:async';

import 'control_client.dart';
import 'host_supervisor.dart';

/// One link to the host's control channel, shared by whatever needs it and opened again when the host has been started anew.
final class HostControl {
  HostControl(this._host, {this.wait = const Duration(seconds: 20)});

  final HostSupervisor _host;

  /// How long the host has to be up before a request gives up.
  final Duration wait;
  ControlClient? _client;
  Future<ControlClient>? _opening;
  bool _waits = true;

  Future<ControlClient> _connect() => _opening ??= (() async => _client = await open(wait: _waits))().whenComplete(() => _opening = null);

  /// A link of its own, which the caller closes: closing it ends what the host is doing for it, such as an install.
  Future<ControlClient> open({bool wait = true}) async {
    if (!wait && _host.phase != HostPhase.running && _host.phase != HostPhase.idle) {
      throw StateError(_host.problem ?? 'the host is not running');
    }
    final deadline = DateTime.now().add(this.wait);
    while (_host.phase != HostPhase.running && _host.phase != HostPhase.idle) {
      if (DateTime.now().isAfter(deadline) || _host.phase == HostPhase.failed || _host.phase == HostPhase.held) {
        throw StateError(_host.problem ?? 'the host is not running');
      }
      await Future<void>.delayed(const Duration(milliseconds: 25));
    }
    return ControlClient(_host.openLink('@control'));
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

  Future<void> close() async => _client?.close();
}
