import '../history/session_move.dart' show MoveLoss;

String? _text(Object? value) => value is String && value.isNotEmpty ? value : null;

int? _int(Object? value) => value is int ? value : null;

List<String> _texts(Object? value) => [if (value is List) for (final each in value) if (each is String) each];

/// Why a member of a role cannot serve now, in the host's words: [until] is when it may again (Unix seconds), [allowance] says the
/// reason is used-up usage, [unavailable] that the member itself cannot run (not installed, not signed in, model not listed).
final class RoleSkip {
  const RoleSkip({required this.member, required this.why, this.until, this.allowance = false, this.unavailable = false});

  static RoleSkip? parse(Object? json) {
    if (json is! Map || json['member'] is! String) return null;
    return RoleSkip(
      member: json['member'] as String,
      why: _text(json['why']) ?? '',
      until: _int(json['until']),
      allowance: json['allowance'] == true,
      unavailable: json['unavailable'] == true,
    );
  }

  final String member;
  final String why;
  final int? until;
  final bool allowance, unavailable;
}

/// One agent, account, model, effort and profile that can serve a role. [id] is `agent/account/model/effort/profile`, which the host
/// names a member by; [agent] is an entry of the host.
final class RoleMember {
  const RoleMember({
    required this.id,
    required this.agent,
    this.account,
    this.model,
    this.effort,
    this.profile,
    this.priority = 0,
    this.maxConcurrent,
    this.stopPermille,
    this.serving = false,
    this.whyNot,
  });

  static RoleMember? parse(Object? json) {
    if (json is! Map || json['agent'] is! String) return null;
    return RoleMember(
      id: _text(json['id']) ?? '${json['agent']}',
      agent: json['agent'] as String,
      account: _text(json['account']),
      model: _text(json['model']),
      effort: _text(json['effort']),
      profile: _text(json['profile']),
      priority: _int(json['priority']) ?? 0,
      maxConcurrent: _int(json['max_concurrent']),
      stopPermille: _int(json['stop_permille']),
      serving: json['serving'] == true,
      whyNot: RoleSkip.parse(json['why_not']),
    );
  }

  final String id, agent;
  final String? account, model, effort, profile;
  final int priority;
  final int? maxConcurrent, stopPermille;

  /// The member the role gives a new session now.
  final bool serving;

  /// Why it cannot serve, when it cannot.
  final RoleSkip? whyNot;

  /// What canoa.cfg's `members` array holds for it: only the keys that are set.
  Map<String, Object?> toTable() => {
    'agent': agent,
    'account': ?account,
    'model': ?model,
    'effort': ?effort,
    'profile': ?profile,
    if (priority != 0) 'priority': priority,
    'max_concurrent': ?maxConcurrent,
    'stop_permille': ?stopPermille,
  };

  /// The parts a person reads, in order: the agent as [agentName] names it, then the account, model and effort that are set.
  String label({String? agentName}) => [agentName ?? agent, ?account, ?model, ?effort].join(' · ');
}

/// A role of canoa.cfg's `[host.roles]` as the host lists it: its members in order and which one serves now.
final class RoleInfo {
  const RoleInfo({
    required this.name,
    this.title = '',
    this.strategy = 'priority',
    this.needs = const [],
    this.stick = true,
    this.handover = 'auto',
    this.resumePrompt,
    this.retryTask = false,
    this.serving,
    this.members = const [],
    this.nothingLeft,
  });

  static RoleInfo? parse(Object? json) {
    if (json is! Map || json['name'] is! String) return null;
    return RoleInfo(
      name: json['name'] as String,
      title: _text(json['title']) ?? '',
      strategy: _text(json['strategy']) ?? 'priority',
      needs: _texts(json['needs']),
      stick: json['stick'] != false,
      handover: _text(json['handover']) ?? 'auto',
      resumePrompt: _text(json['resume_prompt']),
      retryTask: json['retry_task'] == true,
      serving: _text(json['serving']),
      members: [if (json['members'] is List) for (final each in json['members'] as List) ?RoleMember.parse(each)],
      nothingLeft: json['nothing_left'] is Map ? _text((json['nothing_left'] as Map)['message']) : null,
    );
  }

  final String name, title, strategy, handover;
  final List<String> needs;
  final bool stick, retryTask;
  final String? resumePrompt;

  /// The id of the member serving now; null when none can.
  final String? serving;
  final List<RoleMember> members;

  /// The host's words when no member can serve.
  final String? nothingLeft;

  /// What a person calls it: the title, else the name.
  String get label => title.isEmpty ? name : title;

  RoleMember? get servingMember => members.where((each) => each.id == serving).firstOrNull;

  /// What `[host.roles.NAME]` holds for it, only the keys that differ from the host's defaults or are set.
  Map<String, Object?> toTable() => {
    if (title.isNotEmpty) 'title': title,
    'strategy': strategy,
    if (needs.isNotEmpty) 'needs': needs,
    'stick': stick,
    'handover': handover,
    'resume_prompt': ?resumePrompt,
    if (retryTask) 'retry_task': true,
    'members': [for (final each in members) each.toTable()],
  };

  RoleInfo copyWith({
    String? title,
    String? strategy,
    List<String>? needs,
    bool? stick,
    String? handover,
    String? resumePrompt,
    bool clearResumePrompt = false,
    bool? retryTask,
    List<RoleMember>? members,
  }) => RoleInfo(
    name: name,
    title: title ?? this.title,
    strategy: strategy ?? this.strategy,
    needs: needs ?? this.needs,
    stick: stick ?? this.stick,
    handover: handover ?? this.handover,
    resumePrompt: clearResumePrompt ? null : resumePrompt ?? this.resumePrompt,
    retryTask: retryTask ?? this.retryTask,
    serving: serving,
    members: members ?? this.members,
    nothingLeft: nothingLeft,
  );
}

/// What a new session of a role would run on (`roles/resolve`): the member, how it was chosen (`start`, `kept`, `in_turn` or
/// `exhausted`) and each member left out.
final class RoleResolution {
  const RoleResolution({
    required this.role,
    required this.member,
    this.moved = false,
    this.why = 'start',
    this.left,
    this.usageKnown = true,
    this.usagePending = false,
    this.sameHarness = true,
    this.skipped = const [],
  });

  static RoleResolution? parse(Object? json) {
    if (json is! Map || json['role'] is! String) return null;
    final member = RoleMember.parse(json['member']);
    if (member == null) return null;
    return RoleResolution(
      role: json['role'] as String,
      member: member,
      moved: json['moved'] == true,
      why: _text(json['why']) ?? 'start',
      left: _text(json['left']),
      usageKnown: json['usage_known'] != false,
      usagePending: json['usage_pending'] == true,
      sameHarness: json['same_harness'] != false,
      skipped: [if (json['skipped'] is List) for (final each in json['skipped'] as List) ?RoleSkip.parse(each)],
    );
  }

  final String role;
  final RoleMember member;
  final bool moved;
  final String why;
  final String? left;
  final bool usageKnown, usagePending, sameHarness;
  final List<RoleSkip> skipped;
}

/// The host refused to give a role's session a member: none can serve. [message] says why each cannot and when the soonest refills.
final class RoleRefused implements Exception {
  const RoleRefused(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A session of an entry, as `roles/moved` names the two ends of a move.
final class RoleEnd {
  const RoleEnd({required this.entry, required this.session});

  static RoleEnd? parse(Object? json) {
    if (json is! Map || json['entry'] is! String || json['session'] is! String) return null;
    return RoleEnd(entry: json['entry'] as String, session: json['session'] as String);
  }

  final String entry, session;
}

/// The member a push names: its agent entry, and its account and model when it has them.
final class RoleTarget {
  const RoleTarget({required this.entry, this.account, this.model});

  static RoleTarget? parse(Object? json) {
    if (json is! Map || json['entry'] is! String) return null;
    return RoleTarget(entry: json['entry'] as String, account: _text(json['account']), model: _text(json['model']));
  }

  final String entry;
  final String? account, model;
}

/// The host moved a role's session to its next member: the conversation now lives at [to], on the member [member] names, and what the
/// import could not carry is in [losses] (`roles/moved`).
final class RoleMoved {
  const RoleMoved({required this.role, required this.from, required this.to, this.member, this.replayed = false, this.losses = const []});

  static RoleMoved? parse(Object? json) {
    if (json is! Map || json['role'] is! String) return null;
    final from = RoleEnd.parse(json['from']), to = RoleEnd.parse(json['to']);
    if (from == null || to == null) return null;
    final losses = <MoveLoss>[];
    if (json['losses'] case final List<Object?> listed) {
      for (final each in listed.take(32)) {
        if (each is String) {
          losses.add(MoveLoss(each, 1, null));
        } else if (each is Map && each['kind'] is String) {
          losses.add(MoveLoss(each['kind'] as String, each['count'] is int ? each['count'] as int : 1, _text(each['detail'])));
        }
      }
    }
    return RoleMoved(
      role: json['role'] as String,
      from: from,
      to: to,
      member: RoleTarget.parse(json['member']),
      replayed: json['replayed'] == true,
      losses: losses,
    );
  }

  /// The member the session runs on now, as the host names it; null when it names none.
  final RoleTarget? member;

  final String role;
  final RoleEnd from, to;
  final bool replayed;
  final List<MoveLoss> losses;
}

/// Which role serves a session and by whom: what the conversation's "served by" line says. [movedAt], [movedFrom] and [losses] are
/// set once the session has moved.
final class RoleServing {
  const RoleServing({
    required this.role,
    required this.title,
    required this.member,
    this.why,
    this.usageKnown = true,
    this.movedAt,
    this.movedFrom,
    this.replayed = false,
    this.losses = const [],
  });

  final String role, title;
  final RoleMember member;

  /// How the host chose it (`start`, `kept`, `in_turn` or `exhausted`).
  final String? why;
  final bool usageKnown;
  final DateTime? movedAt;

  /// The member it left.
  final RoleMember? movedFrom;
  final bool replayed;
  final List<MoveLoss> losses;

  RoleServing movedTo(RoleMember next, {required DateTime at, required bool replayed, required List<MoveLoss> losses}) => RoleServing(
    role: role,
    title: title,
    member: next,
    why: 'exhausted',
    usageKnown: usageKnown,
    movedAt: at,
    movedFrom: member,
    replayed: replayed,
    losses: losses,
  );
}

/// The host holds a move for the person's yes (`roles/ask`): the session [from] would go to [to], and [reason] says why.
final class RoleAskPush {
  const RoleAskPush({required this.role, required this.from, required this.to, this.reason});

  static RoleAskPush? parse(Object? json) {
    if (json is! Map || json['role'] is! String) return null;
    final from = RoleEnd.parse(json['from']), to = RoleTarget.parse(json['to']);
    if (from == null || to == null) return null;
    return RoleAskPush(role: json['role'] as String, from: from, to: to, reason: _text(json['reason']));
  }

  final String role;
  final RoleEnd from;
  final RoleTarget to;
  final String? reason;
}

/// What `roles/served` says of a session: the role and member of the last turn a role served, null when no role served it.
final class RoleServedInfo {
  const RoleServedInfo({this.role, this.member});

  static RoleServedInfo parse(Object? json) {
    final turns = json is Map && json['turns'] is List ? json['turns'] as List : const [];
    for (final turn in turns.reversed) {
      if (turn is! Map || _text(turn['role']) == null) continue;
      final member = RoleMember.parse({...turn.cast<String, Object?>(), 'id': turn['member']});
      return RoleServedInfo(role: _text(turn['role']), member: member);
    }
    return const RoleServedInfo();
  }

  final String? role;
  final RoleMember? member;
}

/// A move the host holds for the person's yes (`handover = "ask"`): the member whose usage ran out, the one that would take over, and
/// what ran out in the host's words.
final class RoleAsk {
  const RoleAsk({required this.role, required this.from, required this.to, this.left});

  final String role;
  final RoleMember from, to;
  final String? left;
}
