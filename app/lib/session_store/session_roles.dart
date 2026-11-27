part of '../session_store.dart';

extension SessionRoles on SessionStore {
  /// Gives the draft [session] what [resolved] says a new session of its role runs on: the member's agent (found among [sources] by the
  /// entry it names), account, model and effort. The draft keeps the role, so the host is told to bind the session once it starts.
  /// Answers whether the member's agent is one of [sources]; when it is not the draft is left as it was.
  bool applyRole(
    ChatSession session,
    RoleResolution resolved, {
    required Iterable<AgentSource> sources,
    required String title,
  }) {
    if (!session.isDraft) return false;
    final member = resolved.member;
    final source = sources.where((each) => (each.hostEntry ?? each.label) == member.agent).firstOrNull;
    if (source == null) return false;
    if (session.source != source) updateDraft(session, source: source);
    session
      ..account = member.account
      ..role = resolved.role;
    for (final (key, role, value) in [
      ('model', OptionRole.model, member.model),
      ('effort', OptionRole.effort, member.effort),
    ]) {
      if (value == null) continue;
      session.chat.dropChoices(where: (choice) => choice.role == role);
      session.presetWants[key] = value;
    }
    session.roleServing = RoleServing(
      role: resolved.role,
      title: title,
      member: member,
      why: resolved.why,
      usageKnown: resolved.usageKnown,
    );
    _settlePreset(session);
    _keepDraft(session);
    _notify();
    return true;
  }

  /// Says to the host that the session [session], just started on the member it was given, is served by its role, so the host moves
  /// it on when its usage runs out. A refusal is said in the conversation and the session goes on with no role.
  Future<void> bindRole(ChatSession session) async {
    final roles = this.roles, serving = session.roleServing, id = session.remoteSessionId, cwd = session.cwd;
    if (roles == null || serving == null || id == null || cwd == null) return;
    try {
      await roles.bind(
        role: serving.role,
        member: serving.member.id,
        entry: session.source?.hostEntry ?? session.agent,
        session: id,
        cwd: cwd,
      );
    } on Object catch (error) {
      session
        ..roleServing = null
        ..roleProblem = (key: 'role.problem.bind', reason: reasonOf(error), role: serving.title);
    }
    _notify();
  }

  /// Picks the member by hand: the role no longer serves [session], and the host stops moving it. For a draft it only forgets the role.
  Future<void> endRole(ChatSession session) async {
    final roles = this.roles, id = session.remoteSessionId;
    session
      ..role = null
      ..roleServing = null
      ..roleAsk = null;
    _notify();
    if (session.isDraft || roles == null || id == null) return;
    try {
      await roles.unbind(entry: session.source?.hostEntry ?? session.agent, session: id);
    } on Object catch (error) {
      session.roleProblem = (key: 'role.problem.unbind', reason: reasonOf(error), role: '');
      _notify();
    }
  }

  /// Says in [session]'s served line that something about its role went wrong: [key] is the text that names it, [reason] the host's or
  /// the app's words, [role] what the role is called.
  void roleFailed(ChatSession session, String key, String reason, String role) {
    session.roleProblem = (key: key, reason: reason, role: role);
    _notify();
  }

  /// The open session at [end], when this app has it open.
  ChatSession? sessionAt(RoleEnd end) {
    for (final session in _sessions) {
      if (session.isDraft || session.remoteSessionId != end.session) continue;
      if ((session.source?.hostEntry ?? session.agent) == end.entry) return session;
    }
    return null;
  }

  /// The open session the host moved ([moved]'s from end), when this app has it open.
  ChatSession? sessionMoved(RoleMoved moved) => sessionAt(moved.from);

  /// Restores the "served by" line of [session], opened again from the list: the host's record of its turns says which role served
  /// the last of them and by which member. Asked once per session; a session no role served keeps no line.
  void restoreRole(ChatSession session) {
    final roles = this.roles, id = session.remoteSessionId;
    if (roles == null || id == null || session.isDraft || session.roleServing != null || session.roleRestored) return;
    session.roleRestored = true;
    unawaited(() async {
      try {
        final served = await roles.served(session.source?.hostEntry ?? session.agent, id);
        final role = served.role, member = served.member;
        if (_disposed || role == null || member == null || session.roleServing != null) return;
        if (!roles.asked) await roles.refresh();
        session.roleServing = RoleServing(role: role, title: roles.named(role)?.label ?? role, member: member);
        _notify();
      } on Object {
        // A host with no record of the session, or none to ask, leaves it without a line.
      }
    }());
  }

  /// Follows a move the host made on its own: the conversation goes on in the same tab, at the session id on [target]'s entry that the
  /// host imported it as, with the transcript as it was. [member] is the member the role serves it with now; [start] starts [target]'s
  /// agent for the session, whose agent, source and account are already the new member's. If any step fails the conversation stays
  /// with the agent it had, and the transcript says why. Answers whether it moved.
  Future<bool> followRoleMove(
    ChatSession session,
    RoleMoved moved, {
    required AgentSource target,
    required RoleMember member,
    required Future<VerifiedAgent> Function(ChatSession session) start,
  }) async {
    final from = session.agent, fromSource = session.source, oldId = session.remoteSessionId, cwd = session.cwd;
    if (oldId == null || cwd == null) return false;
    final earlier = session.roleServing;
    if (_moving.contains(session.id)) return false;
    _moving.add(session.id);
    final old = _clients[session.id];
    final said = SessionStore._spoken(session);
    final oldEntry = session.peerId == null ? null : ledger?.find(session.peerId!, oldId);
    final oldPeer = session.peerId, oldAccount = session.account, oldSetup = session.setup, oldProfile = session.profile;
    final fromMember = earlier?.member;
    VerifiedAgent? fresh;
    var changed = false;
    try {
      session
        ..agent = target.label
        ..source = target
        ..account = member.account
        ..ranOnAccount = null
        ..ranOnPool = null
        ..setup = AcpSessionSetup.none
        ..profile = null
        ..remoteSessionId = moved.to.session;
      changed = true;
      fresh = await start(session);
      if (!fresh.canReopenSessions) {
        throw AcpFailure('${target.displayName} offers neither session/load nor session/resume');
      }
      session.peerId = fresh.peerId;
      attachVerifiedClient(session.id, fresh, record: false);
      final opened = await session.chat.reopen(() => fresh!.reopenSession(moved.to.session, cwd: cwd));
      _takeHostReport(session, opened);
      await applyHostProfile(session);
      // An agent that sent back less than was said has shown less than the conversation holds: what it was is put back.
      bool isSpoken(model.ChatMessage m) => m.role == MessageRole.user || m.role == MessageRole.agent;
      if (session.messages.where(isSpoken).length < said.length) {
        session.chat.removeMessages(isSpoken);
        for (final each in said) {
          session.chat.addMessage(each.role, each.text);
        }
      }
    } catch (error) {
      if (changed) {
        session
          ..agent = from
          ..source = fromSource
          ..peerId = oldPeer
          ..account = oldAccount
          ..setup = oldSetup
          ..profile = oldProfile
          ..remoteSessionId = oldId;
        if (old != null && !old.isClosed) attachVerifiedClient(session.id, old, record: false);
      }
      await fresh?.close();
      session.roleProblem = (key: 'role.problem.follow', reason: reasonOf(error), role: earlier?.title ?? moved.role);
      _moving.remove(session.id);
      _notify();
      return false;
    }
    await old?.close();
    // The conversation has another id, with another member: the list follows it, with what the person had made of it.
    if (oldEntry != null) ledger?.remove(oldEntry);
    _record(session, fresh);
    if (oldEntry != null && session.peerId != null) {
      ledger?.setPlacement(session.peerId!, moved.to.session, pinned: oldEntry.pinned, archived: oldEntry.archived);
    }
    session
      ..roleAsk = null
      ..roleServing = earlier?.movedTo(member, at: clock(), replayed: moved.replayed, losses: moved.losses) ??
          RoleServing(
            role: moved.role,
            title: moved.role,
            member: member,
            why: 'exhausted',
            movedAt: clock(),
            movedFrom: fromMember,
            replayed: moved.replayed,
            losses: moved.losses,
          );
    _moving.remove(session.id);
    _notify();
    return true;
  }

  /// The host holds a move of a session for the person's yes: the conversation it names asks "Continue on Y?", and nothing moves until
  /// the person says so. A session this app does not have open is not asked about.
  void askRole(RoleAskPush push) {
    final roles = this.roles, session = sessionAt(push.from);
    if (roles == null || session == null) return;
    session.roleAsk = RoleAsk(
      role: push.role,
      from: session.roleServing?.member ?? RoleMember(id: '', agent: push.from.entry),
      to: roles.memberAt(push.role, push.to),
      left: push.reason,
    );
    _notify();
  }

  /// The person's yes: the host runs the move it holds, and the conversation follows it when it says where it went.
  Future<void> continueRole(ChatSession session) async {
    final roles = this.roles, id = session.remoteSessionId;
    session.roleAsk = null;
    _notify();
    if (roles == null || id == null) return;
    try {
      await roles.continueMove(id);
    } on Object catch (error) {
      session.roleProblem = (key: 'role.problem.continue', reason: reasonOf(error), role: session.roleServing?.title ?? '');
      _notify();
    }
  }

  /// Takes the question away; it comes again when the next turn ends and the move is still the host's to make.
  void dismissRoleAsk(ChatSession session) {
    session.roleAsk = null;
    _notify();
  }
}
