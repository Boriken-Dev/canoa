import 'dart:async';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import 'role_models.dart';

/// What the app asks of the host about roles. Roles are written through the host's configuration store, so there is a call to set and
/// one to remove a table, and no method of the roles' own for either.
abstract interface class RolesHost {
  /// Every role with its members in order and the member serving now.
  Future<List<RoleInfo>> list();

  /// The member a new session of [role] would run on, with nothing started. [current] is the member a session runs on now. Throws
  /// [RoleRefused] when no member can serve.
  Future<RoleResolution> resolve(String role, {String? model, String? current});

  /// Says that [session] of [entry], which works in [cwd], is served by [member] of [role], so the host moves it on by itself when a
  /// turn of it ends out of usage.
  Future<void> bind({required String role, required String member, required String entry, required String session, required String cwd});

  /// Ends the role's hold on [session] of [entry], as when the person picks a member by hand.
  Future<void> unbind({required String entry, required String session});

  /// The person's yes to a move the host holds for a role whose handover is `ask`.
  Future<void> continueMove(String session);

  /// Makes `[host.roles.NAME]` hold [table], or changes it to.
  Future<void> setRole(String name, Map<String, Object?> table);

  /// Takes `[host.roles.NAME]` out of the person's layer of canoa.cfg.
  Future<void> removeRole(String name);

  /// What a role served of the session [session] of [entry], from the host's record of its turns.
  Future<RoleServedInfo> served(String entry, String session);

  /// The moves the host makes by itself, as it makes them.
  Stream<RoleMoved> get moves;

  /// The moves the host holds for the person's yes, as it holds them.
  Stream<RoleAskPush> get asks;
}

/// The host's roles over its control channel.
final class ControlRolesHost implements RolesHost {
  ControlRolesHost(this._control) {
    _moves = StreamController<RoleMoved>.broadcast(onListen: _attach, onCancel: _detach);
    _asks = StreamController<RoleAskPush>.broadcast(onListen: _attach, onCancel: _detach);
  }

  final HostControl _control;
  late final StreamController<RoleMoved> _moves;
  late final StreamController<RoleAskPush> _asks;
  StreamSubscription<ControlPush>? _pushes;
  StreamSubscription<void>? _again;
  ControlClient? _heard;

  Future<Object?> _call(String method, Map<String, Object?> params) async =>
      (await _control.ready(wait: false)).call(method, params);

  /// Listens to the control link for `roles/moved`, and again on each link made after the host ended one.
  void _attach() {
    unawaited(_listen());
    _again ??= _control.reconnected.listen((_) => unawaited(_listen()));
  }

  Future<void> _listen([int attempt = 0]) async {
    try {
      final client = await _control.ready(wait: false);
      if (identical(client, _heard)) return;
      await _pushes?.cancel();
      _heard = client;
      _pushes = client.pushes.listen((push) {
        if (push.method == 'roles/moved') {
          final moved = RoleMoved.parse(push.params);
          if (moved != null && !_moves.isClosed) _moves.add(moved);
        } else if (push.method == 'roles/ask') {
          final ask = RoleAskPush.parse(push.params);
          if (ask != null && !_asks.isClosed) _asks.add(ask);
        }
      });
    } on Object {
      // A host that is not up yet is tried again, a few times: the link announces itself only when it comes back after an earlier one.
      if (attempt < _tries && (_moves.hasListener || _asks.hasListener)) {
        _retry?.cancel();
        _retry = Timer(_pause, () => unawaited(_listen(attempt + 1)));
      }
    }
  }

  static const _tries = 20;
  static const _pause = Duration(seconds: 3);
  Timer? _retry;

  void _detach() {
    // The one link serves both streams: it is let go when neither is listened to.
    if (_moves.hasListener || _asks.hasListener) return;
    _retry?.cancel();
    unawaited(_pushes?.cancel());
    unawaited(_again?.cancel());
    _pushes = null;
    _again = null;
    _heard = null;
  }

  @override
  Stream<RoleMoved> get moves => _moves.stream;

  @override
  Stream<RoleAskPush> get asks => _asks.stream;

  @override
  Future<RoleServedInfo> served(String entry, String session) async =>
      RoleServedInfo.parse(await _call('roles/served', {'entry': entry, 'session': session}));

  @override
  Future<List<RoleInfo>> list() async {
    final result = await _call('roles/list', const {});
    final roles = result is Map ? result['roles'] : null;
    return [if (roles is List) for (final each in roles) ?RoleInfo.parse(each)];
  }

  @override
  Future<RoleResolution> resolve(String role, {String? model, String? current}) async {
    try {
      final result = await _call('roles/resolve', {'role': role, 'model': ?model, 'current': ?current});
      return RoleResolution.parse(result) ?? (throw StateError('the host did not say which member serves $role'));
    } on ControlFailed catch (error) {
      // -32002 is a refusal: no member of the role can serve. Any other failure is a host that cannot say.
      if (error.code == -32002) throw RoleRefused(error.message);
      rethrow;
    }
  }

  @override
  Future<void> bind({required String role, required String member, required String entry, required String session, required String cwd}) async {
    await _call('roles/bind', {'role': role, 'member': member, 'entry': entry, 'session': session, 'cwd': cwd});
  }

  @override
  Future<void> unbind({required String entry, required String session}) async {
    await _call('roles/unbind', {'entry': entry, 'session': session});
  }

  @override
  Future<void> continueMove(String session) async {
    await _call('roles/continue', {'session': session});
  }

  @override
  Future<void> setRole(String name, Map<String, Object?> table) async {
    await _call('config/change', {
      'ops': [
        {'path': ['host', 'roles', name], 'value': table},
      ],
    });
  }

  @override
  Future<void> removeRole(String name) async {
    await _call('config/change', {
      'ops': [
        {'path': ['host', 'roles', name], 'value': null},
      ],
    });
  }
}
