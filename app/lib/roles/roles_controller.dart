import 'dart:async';

import 'package:flutter/foundation.dart';

import 'role_models.dart';
import 'roles_host.dart';

/// An agent a role's member may name: an entry of the host, and what it is called.
typedef RoleAgent = ({String entry, String name});

/// The roles Settings lists and the start screen offers: what the host says of each, and the calls that make, change and remove one.
class RolesController extends ChangeNotifier {
  RolesController({required this.host, this.agents = _none});

  static List<RoleAgent> _none() => const [];

  final RolesHost host;

  /// The agents a member may be, by the host's entry names; the app supplies what its host lists.
  final List<RoleAgent> Function() agents;

  List<RoleInfo> _roles = const [];
  String? _problem;
  bool _loading = false, _asked = false, _disposed = false;

  List<RoleInfo> get roles => _roles;

  /// The host's words when it could not list roles (no grant for accounts, a host with none); null once it has.
  String? get problem => _problem;
  bool get loading => _loading;

  /// Whether the host has been asked yet.
  bool get asked => _asked;

  RoleInfo? named(String name) => _roles.where((each) => each.name == name).firstOrNull;

  /// The member [moved] landed the session on: the one of [moved]'s role on the entry it went to that serves now, else the first of
  /// that entry that can serve, else the first of it. The move names the entry and the session, not the member.
  RoleMember? landedOn(RoleMoved moved) {
    final there = [for (final each in named(moved.role)?.members ?? const <RoleMember>[]) if (each.agent == moved.to.entry) each];
    return there.where((each) => each.serving).firstOrNull ?? there.where((each) => each.whyNot == null).firstOrNull ?? there.firstOrNull;
  }

  /// What the entry [entry] is called, as the app lists it; the entry's own name when it lists none.
  String agentName(String entry) => agents().where((each) => each.entry == entry).firstOrNull?.name ?? entry;

  Future<void> refresh() async {
    if (_loading) return;
    _loading = true;
    _notify();
    try {
      _roles = await host.list();
      _problem = null;
    } on Object catch (error) {
      _problem = '$error';
    }
    _loading = false;
    _asked = true;
    _notify();
  }

  /// The member a new session of [role] would run on. Throws [RoleRefused] with the host's words when none can serve.
  Future<RoleResolution> resolve(String role, {String? model, String? current}) => host.resolve(role, model: model, current: current);

  Future<void> bind({required String role, required String member, required String entry, required String session, required String cwd}) =>
      host.bind(role: role, member: member, entry: entry, session: session, cwd: cwd);

  Future<void> unbind({required String entry, required String session}) => host.unbind(entry: entry, session: session);

  Future<void> continueMove(String session) => host.continueMove(session);

  /// Makes or changes the role [role.name]; the roles are read again after the host wrote it.
  Future<void> save(RoleInfo role) async {
    await host.setRole(role.name, role.toTable());
    await refresh();
  }

  Future<void> remove(String name) async {
    await host.removeRole(name);
    await refresh();
  }

  Stream<RoleMoved> get moves => host.moves;

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
