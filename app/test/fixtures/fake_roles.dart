// A host's roles the tests control: the roles it lists, what it answers to a resolve, what it is asked (binds, unbinds, continues,
// writes of canoa.cfg), and the moves it pushes.
import 'dart:async';

import 'package:canoa/roles/role_models.dart';
import 'package:canoa/roles/roles_host.dart';

/// A member as `roles/list` and `roles/resolve` word it.
Map<String, Object?> memberJson(String agent, {String? account, String? model, String? effort, String? profile, bool serving = false, Map<String, Object?>? whyNot}) => {
  'id': [agent, account ?? '', model ?? '', effort ?? '', profile ?? ''].join('/'),
  'agent': agent,
  'account': account,
  'model': model,
  'effort': effort,
  'profile': profile,
  'priority': 0,
  'max_concurrent': null,
  'stop_permille': null,
  'serving': serving,
  'why_not': ?whyNot,
};

/// A role as `roles/list` words it.
RoleInfo roleOf(String name, List<Map<String, Object?>> members, {String? title, String handover = 'auto', String strategy = 'priority', List<String> needs = const []}) {
  final serving = members.where((each) => each['serving'] == true).firstOrNull;
  return RoleInfo.parse({
    'name': name,
    'title': title ?? name,
    'strategy': strategy,
    'needs': needs,
    'stick': true,
    'handover': handover,
    'retry_task': false,
    'serving': serving?['id'],
    'members': members,
  })!;
}

final class FakeRolesHost implements RolesHost {
  FakeRolesHost({List<RoleInfo>? roles}) : roles = roles ?? [];

  final List<RoleInfo> roles;

  /// What `roles/resolve` answers for a role (by name): the resolution, else a refusal when [refusal] is set.
  final resolutions = <String, RoleResolution>{};
  String? refusal;

  /// How many times the roles were listed.
  int listCalls = 0;

  /// What `roles/served` answers, by `entry/session`; a session it holds no record of is a host failure.
  final served_ = <String, RoleServedInfo>{};
  String? listFails;
  final resolves = <({String role, String? model, String? current})>[];

  final binds = <({String role, String member, String entry, String session, String cwd})>[];
  final unbinds = <({String entry, String session})>[];
  final continues = <String>[];
  String? bindRefusal;

  /// What was written to canoa.cfg: a role set by name, or taken out.
  final written = <(String, Map<String, Object?>)>[];
  final removed = <String>[];
  String? writeRefusal;

  final _moves = StreamController<RoleMoved>.broadcast();
  final _asks = StreamController<RoleAskPush>.broadcast();

  /// The host holds a move for the person's yes.
  void ask(RoleAskPush push) => _asks.add(push);

  @override
  Stream<RoleAskPush> get asks => _asks.stream;

  @override
  Future<RoleServedInfo> served(String entry, String session) async =>
      served_['$entry/$session'] ?? (throw StateError('no record of $session'));

  /// The host moves a session on by itself.
  void move(RoleMoved moved) => _moves.add(moved);

  @override
  Stream<RoleMoved> get moves => _moves.stream;

  @override
  Future<List<RoleInfo>> list() async {
    listCalls++;
    if (listFails != null) throw StateError(listFails!);
    return List.of(roles);
  }

  @override
  Future<RoleResolution> resolve(String role, {String? model, String? current}) async {
    resolves.add((role: role, model: model, current: current));
    if (refusal != null) throw RoleRefused(refusal!);
    return resolutions[role] ?? (throw StateError('no resolution for $role'));
  }

  @override
  Future<void> bind({required String role, required String member, required String entry, required String session, required String cwd}) async {
    if (bindRefusal != null) throw StateError(bindRefusal!);
    binds.add((role: role, member: member, entry: entry, session: session, cwd: cwd));
  }

  @override
  Future<void> unbind({required String entry, required String session}) async {
    unbinds.add((entry: entry, session: session));
  }

  @override
  Future<void> continueMove(String session) async {
    continues.add(session);
  }

  @override
  Future<void> setRole(String name, Map<String, Object?> table) async {
    if (writeRefusal != null) throw StateError(writeRefusal!);
    written.add((name, table));
    roles.removeWhere((each) => each.name == name);
    final members = [for (final each in (table['members'] as List).cast<Map<String, Object?>>()) memberJson(each['agent']! as String, account: each['account'] as String?, model: each['model'] as String?, effort: each['effort'] as String?, profile: each['profile'] as String?)];
    roles.add(RoleInfo.parse({'name': name, ...table, 'members': members})!);
  }

  @override
  Future<void> removeRole(String name) async {
    removed.add(name);
    roles.removeWhere((each) => each.name == name);
  }

  Future<void> close() async {
    await _moves.close();
    await _asks.close();
  }
}
