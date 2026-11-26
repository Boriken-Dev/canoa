import 'package:flutter/foundation.dart';

import 'control_client.dart';

/// The grants the host gives the key this app reaches its control channel with: the host's own link holds every one, a paired
/// browser key the ones canoa.cfg (or a portal) gave it. What the app shows of the host's program Store and accounts follows
/// these, so the host is the only place that decides.
final class HostGrants extends ChangeNotifier {
  HostGrants(this._control);

  final Future<ControlClient> Function() _control;
  Set<String>? _held;

  /// The grant that installs, updates, rolls back and removes agent programs.
  static const programs = 'store.programs';

  /// The grant that lists, adds, signs in to and removes accounts.
  static const accounts = 'accounts';

  /// The grant that lists the secrets the host holds, shows one on request, forgets one and sets one.
  static const secrets = 'secrets';

  /// The grant that asks for the host's notices, says which session the person is looking at, and sets up the notifiers.
  static const notify = 'notify';

  /// The grant that writes the app's own tables of canoa.cfg, the saved connections among them.
  static const configWrite = 'config.write';

  /// The grant that lists, pairs and removes keys, shows a PIN and edits `[host]`.
  static const hosting = 'hosting';

  /// The grant that writes regular UTF-8 text files on the host via `fs/write`.
  static const filesWrite = 'files.write';

  /// The grant that reads what the agents spent and the limits they reported.
  static const usage = 'usage';

  /// The grant that opens a shell on the host, as the person.
  static const terminal = 'terminal';

  /// The grant that lists the host's speech providers and runs a recording on one.
  static const voice = 'voice';

  /// The grant that adds, changes and removes the host's speech providers.
  static const voiceSetup = 'voice.setup';

  /// The grant that arms a watch that prompts a shared session again.
  static const sessionsWatch = 'sessions.watch';

  /// Whether the host said this key holds [grant]; false until it has said.
  bool may(String grant) => _held?.contains(grant) ?? false;

  /// The grants the host said, or null before it has said.
  Set<String>? get held => _held == null ? null : Set.unmodifiable(_held!);

  /// Whether the host has answered.
  bool get known => _held != null;

  /// Asks the host again. A host that cannot be reached leaves what was known as it was; one with no such method (an older
  /// host) grants nothing of this.
  Future<void> refresh() async {
    final Set<String> held;
    try {
      final result = await (await _control()).call('host/grants', const {});
      final names = result is Map ? result['grants'] : null;
      held = {if (names is List) for (final name in names) if (name is String) name};
    } on ControlFailed catch (error) {
      if (error.code != -32601) return;
      _set(const {});
      return;
    } on Object {
      return;
    }
    _set(held);
  }

  void _set(Set<String> held) {
    if (setEquals(held, _held)) return;
    _held = held;
    notifyListeners();
  }
}
