import 'dart:async';

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
  ControlClient? _client;
  Future<ControlClient>? _opening;
  bool _waits = true;

  Future<ControlClient> _connect() => _opening ??= (() async => _client = await open(wait: _waits))().whenComplete(() => _opening = null);

  /// A link of its own, which the caller closes: closing it ends what the host is doing for it, such as an install.
  Future<ControlClient> open({bool wait = true}) async {
    final dial = _dial;
    if (dial != null) return ControlClient(await dial().timeout(this.wait));
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

  Future<void> close() async => _client?.close();
}
