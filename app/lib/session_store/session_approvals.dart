part of '../session_store.dart';

extension SessionApprovals on SessionStore {
  /// Puts a permission request to the person, in the conversation it belongs to: the one [from] serves under the request's
  /// session id. A request about no session here is refused, and one past the conversation's limit is cancelled.
  Future<AcpDecision> askPermission(
    AcpPermissionRequest request,
    VerifiedAgent from,
  ) {
    final target = _sessionOf(request.sessionId, from);
    if (target == null) {
      return Future.value(const AcpDeny('not a session here'));
    }
    return target.chat.askPermission(request);
  }

  /// Puts an agent's question to the person, in the conversation the client [from] serves under its session id. A question about
  /// no session here is cancelled, and so is one past the conversation's limit.
  Future<AcpElicitationAnswer> askQuestion(
    AcpElicitation request,
    VerifiedAgent from,
  ) {
    final target = _sessionOf(request.sessionId, from);
    if (target == null) return Future.value(const AcpElicitCancel());
    return target.chat.askQuestion(request);
  }

  /// The conversation the client [from] serves under the session id [remote], the host's identity, which the proven peer id of [from]
  /// is, and what binds the answer. The link to the host in this process has the secret they share and the host identity derived from
  /// it; any other program this app started has no identity, so it can ask for no secret.
  (ChatSession, AcpIdentityId, SecretIdentity)? _secretTarget(
    String remote,
    VerifiedAgent from,
  ) {
    final linked = from.linkSecrets;
    // A connection a Noise key proved binds by that key alone, never by the store's WebCrypto identity.
    final identity = from.provedByNoise
        ? from.noiseSecrets
        : linked?.identity ?? secretIdentity;
    if (identity == null) return null;
    final AcpIdentityId host;
    try {
      host = linked?.host ?? AcpIdentityId.parse(from.peerId);
    } on AcpSecretException {
      return null;
    }
    final session = _sessionOf(remote, from);
    if (session == null || session.pendingSecrets.length >= maxPendingSecrets) {
      return null;
    }
    identity.verifier.trust(host);
    return (session, host, identity);
  }

  /// What the secret cards of [prompt] check with and bind to.
  SecretCardEnvironment secretEnvironment(SecretPrompt prompt) {
    final identity = prompt.identity ?? secretIdentity!;
    return SecretCardEnvironment(
      prover: identity.prover,
      verifier: identity.verifier,
      expected: prompt.host,
      host: 'Canoa',
    );
  }

  /// The person's answer to [prompt], a request of [session]'s.
  void answerSecret(
    ChatSession session,
    SecretRequestPrompt prompt,
    AcpSecretRequestReply reply,
  ) {
    if (!session.pendingSecrets.remove(prompt)) return;
    if (!prompt.reply.isCompleted) prompt.reply.complete(reply);
    _notify();
  }

  /// The person's answer to the open step of [prompt], a show of [session]'s. A show that is not going ahead ends here.
  void readySecretShow(
    ChatSession session,
    SecretShowPrompt prompt,
    AcpSecretShowOpened reply,
  ) {
    if (!session.pendingSecrets.contains(prompt)) return;
    if (!prompt.ready.isCompleted) prompt.ready.complete(reply);
    if (reply.recipient == null) {
      session.pendingSecrets.remove(prompt);
      if (!prompt.done.isCompleted) prompt.done.complete(false);
    }
    _notify();
  }

  /// The person closed [prompt], a show of [session]'s, having seen the values or not.
  void finishSecretShow(
    ChatSession session,
    SecretShowPrompt prompt,
    bool shown,
  ) {
    if (!session.pendingSecrets.remove(prompt)) return;
    if (!prompt.done.isCompleted) prompt.done.complete(shown);
    _notify();
  }

  /// The person's answer to the oldest of [session]'s questions.
  void answerQuestion(
    ChatSession session,
    AcpElicitationAnswer answer, {
    String by = ownerClient,
  }) => session.chat.answerQuestion(answer, by: by);

  /// The person's answer to the oldest question in [session]: the agent's first option that allows once, or refuses once.
  void answerPermission(ChatSession session, {required bool allow}) =>
      session.chat.answerPermission(allow: allow, by: ownerClient);

  /// Marks [path] viewed at [version] in [session]'s changes, or not viewed when [version] is null.
  void markViewed(ChatSession session, String path, int? version) {
    if (version == null) {
      session.viewedChanges.remove(path);
    } else {
      session.viewedChanges[path] = version;
    }
    _notify();
  }

  /// Answers the oldest question in [session] with the agent's own refusal of this one request (else a plain refusal), and
  /// leaves the turn running.
  void _refuseOnce(ChatSession session) {
    final question = session.pendingPermissions.first;
    final refusal =
        question.options
            .where((option) => option.kind == 'reject_once')
            .firstOrNull ??
        question.options.where((option) => option.rejects).firstOrNull;
    session.chat.settle(
      question,
      refusal == null
          ? const AcpDeny('the person declined and said what to do instead')
          : AcpSelect(refusal.optionId),
      by: ownerClient,
    );
  }

  /// The person's choice of one of the agent's own answers to the oldest question in [session]. What an "always" answer
  /// remembers is the agent's to keep.
  void choosePermission(
    ChatSession session,
    AcpPermissionOption option, {
    String by = ownerClient,
  }) {
    if (session.pendingPermissions.isEmpty) return;
    final question = session.pendingPermissions.first;
    if (!session.chat.choosePermission(option, by: by)) return;
    // A plain Deny stops the turn, as it does in Claude; a subagent's request is that subagent's to lose.
    if (option.rejects &&
        question.task == null &&
        session.status == SessionStatus.working) {
      unawaited(cancelSession(session).catchError((Object _) {}));
    }
  }

  /// An answer settled a request: an "always" is a task's own or the conversation's.
  void _permissionSettled(ChatSession session, PermissionQuestion question) {
    final outcome = question.outcome;
    final answer = outcome?.answer;
    if (outcome == null ||
        outcome.how != Settlement.answered ||
        outcome.by == model.bypassClient ||
        answer is! AcpSelect) {
      return;
    }
    if (question.options
            .where((option) => option.optionId == answer.optionId)
            .firstOrNull
            ?.kind !=
        'allow_always') {
      return;
    }
    final task = question.task, key = question.allowanceKey;
    if (task != null && key != null) {
      // A task's "always" is the task's: the conversation's own mode and the other tasks are not changed by it.
      final allowed = session.taskAllowances.putIfAbsent(task.id, () => {});
      if (allowed.length < maxTaskAllowances) allowed.add(key);
    } else {
      session.alwaysAnsweredAt = DateTime.now();
    }
  }

  /// Whether [session] is in a mode Canoa can see asks about commands: the agent's own Manual or Plan. In Accept edits, in
  /// Bypass, in the mode an agent's own "always" switches to, in a mode it names otherwise, or in none yet, a remembered command
  /// would answer what the mode leaves to the person, or add to what the agent already allows.
  bool _modeAsks(ChatSession session) {
    if (session.bypass) return false;
    final value = underlyingMode(session)?.toLowerCase();
    return value != null && const {'manual', 'default', 'plan'}.contains(value);
  }

  bool _pathsHere(ChatSession session) => pathsOnHost?.call(session) ?? (!kIsWeb && session.place == SessionPlace.local);

  /// What a rule could state of [request] in [session]: commands only while the mode asks about them; changes to files only for a
  /// session whose files this app can ask about (on this computer, or on the host it is a client of), in a folder where links can be
  /// followed and that is neither home nor Canoa's settings; an MCP tool
  /// always. Null when no rule can cover it.
  RuleSubject? _ruleSubject(ChatSession session, AcpPermissionRequest request) {
    final subject = subjectOf(
      kind: request.kind,
      rawInput: request.rawInput,
      toolCall: request.toolCall,
    );
    final folder = session.cwd;
    return switch (subject) {
      PathSubject() when !_pathsHere(session) || folder == null || !(rules?.pathsAllowedIn(folder) ?? false) => null,
      CommandSubject() || McpSubject() when !_modeAsks(session) => null,
      _ => subject,
    };
  }

  /// The scope the person set, as [type] and [value], for [question], the request the dialog was opened for: remembered as a rule
  /// for this source in this folder, and that request answered with the agent's allow-once answer. Returns why it cannot be made,
  /// or null once it is.
  Future<String?> rememberHere(ChatSession session, PermissionQuestion question, RuleType type, String value) async {
    final store = rules, folder = session.cwd;
    if (store == null || folder == null || folder.isEmpty) {
      return 'There is nowhere to keep a rule.';
    }
    if (!session.pendingPermissions.contains(question)) {
      return 'That request is no longer waiting.';
    }
    final subject = question.subject, kind = question.kind;
    final once = question.options.where((option) => option.kind == 'allow_once').firstOrNull;
    if (subject == null || kind == null || once == null) return 'No rule can cover this request.';
    if (store.problem != null) return 'The rules file cannot be read, so no rule is made: ${store.problem}';
    // The host says where the links in the scope lead before the rule is judged; that takes a moment, in which the request may
    // have been answered elsewhere.
    await store.prepare(() => store.checkScope(type, value, subject, folder) ?? store.conflict(folder, type));
    if (!session.pendingPermissions.contains(question)) return 'That request is no longer waiting.';
    final problem = store.checkScope(type, value, subject, folder) ?? store.conflict(folder, type);
    if (problem != null) return problem;
    final rule = store.add(
      source: SessionStore.ruleSource(session),
      peer: session.peerId ?? '',
      folder: folder,
      kind: kind,
      toolName: subject.toolName,
      type: type,
      value: value,
    );
    if (rule == null) {
      return 'Canoa keeps at most ${PermissionRules.maxRules} rules: revoke one in Settings, Permissions first.';
    }
    session.chat.note(
      'Always allow here: ${rule.describe}, for ${shortLine(session.agent) ?? 'this source'} in $folder. Canoa now '
      'answers a matching request with allow once, without asking you. Settings, Permissions revokes it.',
    );
    session.chat.settle(question, AcpSelect(once.optionId), by: ownerClient);
    return null;
  }

  /// The person's answer to a question offering nothing they could choose: it is answered cancelled.
  void cancelPermission(ChatSession session, {String by = ownerClient}) =>
      session.chat.cancelPermission(by: by);

  /// Takes away what [session] held for the connection that ended or was replaced: its tasks' agents ended with it, and so did the
  /// secret prompts put to the person.
  void _strand(ChatSession session) {
    session.tasks.cancelLive(DateTime.now());
    session.taskAllowances.clear();
    _cancelSecrets(session);
  }

  void _cancelSecrets(ChatSession session) {
    for (final prompt in session.pendingSecrets) {
      prompt.cancel();
    }
    session.pendingSecrets.clear();
  }

  /// A session over a browser provider's connection, which has no endpoint of Canoa's own.
  ChatSession addProviderSession({
    required String title,
    required String agent,
    required String peerId,
    required String remoteSessionId,
    String? cwd,
    SessionWorktree? worktree,
    String? site,
    Relation? relation,
    String? workspaceId,
    String? loadout,
    bool agentNames = false,
  }) {
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    return _insert(
      _newSession(
          title: title,
          agent: agent,
          status: SessionStatus.disconnected,
          peerId: peerId,
          remoteSessionId: remoteSessionId,
          cwd: cwd,
          place: peerId.startsWith('local:')
              ? SessionPlace.local
              : SessionPlace.browser,
          worktree: worktree,
          agentNames: agentNames,
        )
        ..site = site
        ..relation = relation
        ..loadout = loadout
        ..kind = site == null ? SessionKind.workspace : SessionKind.chat
        ..workspaceId = workspaceId,
    );
  }

  ChatSession _insert(ChatSession session) {
    _sessions.insert(0, session);
    _wire(session);
    _selectedId = session.id;
    _notify();
    return session;
  }

  /// A verified transport may attach an existing remote session. This method
  /// records its identity but does not establish trust or connect to a peer.
  ChatSession addRemoteSession({
    required String title,
    required String agent,
    required String peerId,
    required String remoteSessionId,
    required Uri endpoint,
    Uri? websocketFallback,
    SessionPlace place = SessionPlace.peer,
    String? method,
    String? cwd,
    SessionWorktree? worktree,
    bool agentNames = false,
  }) {
    // This records transport locations, not the authentication state. The
    // session stays disconnected until peer proof and ACP are implemented.
    if (endpoint.scheme == 'https') {
      WebEndpoints(
        webTransport: endpoint,
        webSocket: websocketFallback,
        allowPlainWebSocket: plainOnLoopback(websocketFallback),
      );
    } else if (endpoint.scheme == 'wss' || endpoint.scheme == 'ws') {
      if (websocketFallback != null) {
        throw ArgumentError('A direct WSS endpoint cannot have a fallback');
      }
      // Plain WebSocket only on loopback, where canoa listens.
      WebEndpoints(webSocket: endpoint, allowPlainWebSocket: plainOnLoopback(endpoint));
    } else {
      throw ArgumentError.value(
        endpoint,
        'endpoint',
        'Use HTTPS WebTransport or WSS',
      );
    }
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    return _insert(
      _newSession(
        title: title,
        agent: agent,
        peerId: peerId,
        remoteSessionId: remoteSessionId,
        endpoint: endpoint,
        websocketFallback: websocketFallback,
        status: SessionStatus.disconnected,
        place: place,
        method: method,
        cwd: cwd,
        worktree: worktree,
        agentNames: agentNames,
      ),
    );
  }

  /// The model's connection over [client], one for each so that a conversation can tell its own.
  VerifiedChatConnection _connectionOf(VerifiedAgent client) =>
      _connections[client] ??= VerifiedChatConnection(client);

  /// A verified and initialized ACP client can be bound to one session, which the ledger then notes unless [record] is false.
  /// Opening and peer authentication remain the connector's responsibility.
  void attachVerifiedClient(
    String localId,
    VerifiedAgent client, {
    bool record = true,
  }) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.id == localId) target = session;
    }
    if (target == null ||
        target.remoteSessionId == null ||
        target.peerId != client.peerId ||
        client.isClosed) {
      throw StateError('Verified peer and remote session must match');
    }
    final session = target;
    final earlier = _clients[localId];
    // What the earlier connection asked can no longer be answered, whichever client is asked.
    if (earlier != null && !identical(earlier, client)) _strand(session);
    _clients[localId] = client;
    session._phase = null;
    session.chat.attach(_connectionOf(client));
    _watchTasks(session, client);
    _watchContext(session, client);
    _watchHub(session, client);
    if (record) _record(session, client);
    unawaited(refreshGit(session));
    _notify();
  }
}
