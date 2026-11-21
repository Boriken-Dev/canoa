part of '../session_store.dart';

extension SessionHistory on SessionStore {
  /// Whether a person can ask this session's agent for its earlier conversations.
  bool canListEarlier(ChatSession session) {
    final client = _clients[session.id];
    return client != null && !client.isClosed && client.canReopenSessions;
  }

  /// The agent's other conversations, where this one is working when that is known. It lists them, and they are its own to name.
  Future<AcpSessionPage> earlierSessions(
    ChatSession from, {
    String? cursor,
  }) async {
    final client = _clients[from.id];
    if (client == null || client.isClosed) {
      throw StateError('A verified ACP peer must be connected');
    }
    final page = await client.listSessions(cwd: from.cwd, cursor: cursor);
    return AcpSessionPage(
      page.sessions
          .where((session) => session.id != from.remoteSessionId)
          .toList(),
      page.nextCursor,
    );
  }

  /// Opens an earlier conversation beside this one, on the same agent. One already open is selected, not opened twice.
  Future<ChatSession> openEarlier(
    ChatSession from,
    AcpSessionInfo earlier,
  ) async {
    final client = _clients[from.id];
    final cwd = earlier.cwd ?? from.cwd;
    if (client == null || client.isClosed) {
      throw StateError('A verified ACP peer must be connected');
    }
    if (cwd == null) {
      throw StateError('The agent did not say where that conversation was');
    }
    for (final session in _sessions) {
      if (session.remoteSessionId == earlier.id &&
          _clients[session.id] == client) {
        select(session.id);
        return session;
      }
    }
    final session = addProviderSession(
      title: shortLine(earlier.title) ?? 'Earlier conversation',
      agent: from.agent,
      peerId: client.peerId,
      remoteSessionId: earlier.id,
      cwd: cwd,
    );
    // Listening first, since the conversation arrives before the answer does.
    attachVerifiedClient(session.id, client, record: false);
    session
      ..setup = from.setup
      ..profile = from.profile;
    try {
      final opened = await session.chat.reopen(
        () => client.reopenSession(earlier.id, cwd: cwd, setup: from.setup),
      );
      _takeHostReport(session, opened);
      // The profile the host reports for this session, else the one of the conversation it opens beside, on the same agent.
      await applyHostProfile(session);
      if (from.profile case final profile?
          when identical(session.profile, profile)) {
        session.chat.note(profile.summary);
      }
      _record(session, client);
      _notify();
      return session;
    } catch (_) {
      forget(session);
      rethrow;
    }
  }

  /// Opens the session a move made, [sessionId] on [client] (the agent [agent] of this computer), beside [from], which stays as it
  /// was. What the move did not carry is [notes]; [carried] is what was said, shown when the agent does not send it itself.
  Future<ChatSession> openMoved(
    ChatSession from,
    VerifiedAgent client, {
    required String sessionId,
    required String agent,
    required String cwd,
    required List<String> notes,
    List<({MessageRole role, String text})> carried = const [],
  }) async {
    if (!client.canReopenSessions) {
      throw AcpFailure('$agent offers neither session/load nor session/resume');
    }
    final session = addProviderSession(
      title: from.title,
      agent: agent,
      peerId: client.peerId,
      remoteSessionId: sessionId,
      cwd: cwd,
    );
    attachVerifiedClient(session.id, client, record: false);
    try {
      final opened = await session.chat.reopen(
        () => client.reopenSession(sessionId, cwd: cwd),
      );
      _takeHostReport(session, opened);
      // An agent that sent all of what was said before has shown it; one that sent less is shown the host's copy of it instead.
      bool said(model.ChatMessage m) =>
          m.role == MessageRole.user || m.role == MessageRole.agent;
      if (session.messages.where(said).length < carried.length) {
        session.chat.removeMessages(said);
        for (final each in carried) {
          session.chat.addMessage(each.role, each.text);
        }
      }
      for (final note in notes) {
        session.chat.note(note);
      }
      _record(session, client);
      _notify();
      return session;
    } catch (_) {
      forget(session);
      rethrow;
    }
  }

  /// Opens a session from the ledger again on [client], an agent just started or connected for it, by `session/load` or
  /// `session/resume`, whichever the agent offers. One already open is selected instead, and [client] is left to the caller.
  /// The profile the host reports for it is applied as [applyHostProfile] does; [expected] says whether one should come.
  ///
  /// With [attach] the session is one the host keeps running for several clients: this takes part in it as it runs, replayed what
  /// was said (all of it, or only what followed this app's cursor when what it said until then is still here), instead of opening
  /// the agent's own copy again. A session already open here whose connection ended is attached again in place.
  Future<ChatSession> reopen(
    LedgerEntry entry,
    VerifiedAgent client, {
    bool expected = false,
    String? host,
    bool attach = false,
  }) async {
    for (final session in _sessions) {
      if (session.peerId == entry.peerId &&
          session.remoteSessionId == entry.sessionId) {
        final live = _clients[session.id];
        if (attach && (live == null || live.isClosed)) {
          return _reattach(session, entry, client, expected: expected);
        }
        select(session.id);
        return session;
      }
    }
    if (client.peerId != entry.peerId) {
      throw StateError(
        'That agent is not the one this session was opened with',
      );
    }
    if (!attach &&
        (client.identity.reopenBy == null || !client.canReopenSessions)) {
      throw AcpFailure(
        '${entry.harness} offers neither session/load nor session/resume, so this session cannot be opened again',
      );
    }
    final cwd = entry.cwd;
    if (cwd == null) {
      throw StateError(
        'Canoa did not note where this session was, which the agent needs to open it',
      );
    }
    // What the agent lists now names the session, when it still has it under the same name.
    var title = entry.title ?? 'Earlier conversation';
    try {
      // A session the host keeps is joined by the first thing said on the connection, so nothing is asked before it.
      final described = attach ? null : await client.describe();
      if (described case AcpDescription(sessions: AcpKnown(value: final listed))) {
        for (final info in listed.sessions) {
          if (info.id == entry.sessionId && !entry.named) {
            title = shortLine(info.title) ?? title;
          }
        }
      }
    } on AcpFailure {
      // Only the name depends on it.
    }
    final endpoint = entry.endpoint == null
        ? null
        : Uri.tryParse(entry.endpoint!);
    final session =
        _insert(
            _newSession(
              title: title,
              agent: entry.entry,
              status: SessionStatus.disconnected,
              peerId: client.peerId,
              remoteSessionId: entry.sessionId,
              cwd: cwd,
              place: entry.place,
              endpoint: endpoint,
              websocketFallback: entry.fallback == null
                  ? null
                  : Uri.tryParse(entry.fallback!),
              method: entry.method,
              worktree: entry.worktree,
              agentNames: !entry.named,
            ),
          )
          ..host = host
          ..site = entry.site
          ..relation = entry.relation
          ..kind = entry.kind
          ..namedByPerson = entry.named
          ..workspaceId = entry.canoaId
          ..cacheLifetime = CacheLifetime.named(entry.cacheLifetime)
          ..account = entry.account
          ..overlays = entry.overlays?.toSet()
          ..pinned = entry.pinned
          ..archived = entry.archived;
    // The ledger's note changes as the mode goes back, so what it was left in is read first.
    final keptMode = entry.mode, keptBypass = entry.bypass;
    attachVerifiedClient(session.id, client, record: false);
    try {
      // The mode the session was left in goes back, once, before anything is sent. A webpage's session is reopened in the folder its
      // host made for it, which the host checks is that site's own.
      final opened = await session.chat.reopen(
        () => attach
            ? _attachOpen(session, entry, client, cwd, [
                for (final kept in entry.messages)
                  (SessionStore._keptRole(kept.role), kept.text, kept.at),
              ])
            : client.reopenSession(
                entry.sessionId,
                cwd: cwd,
                setup: entry.site == null
                    ? AcpSessionSetup.none
                    : siteSetup(entry.site!),
              ),
        keptMode: keptMode,
        keptBypass: keptBypass,
      );
      if (attach) _attached(session, client);
      _takeHostReport(session, opened);
      await applyHostProfile(session, expected: expected, keptMode: keptMode);
      if (!opened.replayed && entry.messages.isNotEmpty) {
        session.chat.note(
          'What Canoa kept of it follows, from its own list, not from the agent.',
        );
        for (final kept in entry.messages) {
          session.chat.addMessage(SessionStore._keptRole(kept.role), kept.text, at: kept.at);
        }
      }
      _record(session, client);
      _notify();
      return session;
    } catch (_) {
      forget(session);
      rethrow;
    }
  }

  /// Attaches to a session the host keeps for several clients, with what [held] says was said until the cursor put back first, so
  /// the replay that follows the cursor completes it. With nothing held the replay is the whole of it, from the start.
  Future<AcpOpenedSession> _attachOpen(
    ChatSession session,
    LedgerEntry entry,
    VerifiedAgent client,
    String cwd,
    List<(MessageRole, String, DateTime)> held,
  ) {
    final cursor = held.isEmpty ? null : (session.cursor ?? entry.cursor);
    if (cursor != null) {
      for (final (role, text, at) in held) {
        session.chat.addMessage(role, text, at: at);
      }
    }
    return client.attachSession(entry.sessionId, cwd: cwd, cursor: cursor);
  }

  /// What follows an attach: where in the host's log this app now is, kept, and a word when the host had dropped some of it.
  void _attached(ChatSession session, VerifiedAgent client) {
    final remote = session.remoteSessionId;
    if (remote == null) return;
    session.cursor = client.hub.cursorOf(remote) ?? session.cursor;
    session.others = client.hub.others(remote);
    _keepCursor(session);
    if (client.hub.truncated(remote)) {
      session.chat.note(
        'The host no longer keeps all of what was said while this app was away, so some of it is missing here.',
      );
    }
  }

  /// A session of the host's hub already open here whose connection ended, attached again on [client]: what it said is put back
  /// as text and only what followed [ChatSession.cursor] is replayed.
  Future<ChatSession> _reattach(
    ChatSession session,
    LedgerEntry entry,
    VerifiedAgent client, {
    bool expected = false,
  }) async {
    final cwd = session.cwd ?? entry.cwd;
    if (cwd == null) {
      throw StateError(
        'Canoa did not note where this session was, which the host needs to attach to it',
      );
    }
    if (client.peerId != entry.peerId) {
      throw StateError(
        'That agent is not the one this session was opened with',
      );
    }
    final held = [
      for (final message in session.messages)
        if (message.role == MessageRole.user ||
            message.role == MessageRole.agent)
          (message.role, message.text, message.at),
    ];
    attachVerifiedClient(session.id, client, record: false);
    final opened = await session.chat.reopen(
      () => _attachOpen(session, entry, client, cwd, held),
    );
    _takeHostReport(session, opened);
    _attached(session, client);
    _record(session, client);
    select(session.id);
    _notify();
    return session;
  }

  /// Shows what Canoa kept of a session it cannot open again, as an offline preview that sends nothing.
  ChatSession showKept(LedgerEntry entry) {
    final session = createPreviewSession(
      title: entry.title ?? 'Earlier conversation',
      agent: entry.harness,
    );
    session.chat.note(
      'What Canoa kept of this session. ${entry.whyNotReopened ?? ''} Nothing typed here reaches an agent.',
    );
    for (final kept in entry.messages) {
      session.chat.addMessage(SessionStore._keptRole(kept.role), kept.text, at: kept.at);
    }
    return session;
  }

  /// Shows a session the host brought in from another program's own folder, as what was said in it and nothing more. It is a
  /// preview: nothing typed here reaches an agent, and the harness's own files are not touched.
  ChatSession showImported({
    required String title,
    required String agent,
    required List<({MessageRole role, String text})> said,
  }) {
    final session = createPreviewSession(title: title, agent: agent);
    session.chat.note(
      'Brought in from $agent\'s own folder. Nothing typed here reaches an agent.',
    );
    for (final each in said) {
      session.chat.addMessage(each.role, each.text);
    }
    return session;
  }

  /// Takes a conversation out of the list. The agent still has it, and its connection stays with whoever else uses it.
  void forget(ChatSession session) {
    _keepText(session);
    // A draft closed leaves nothing kept of it.
    if (session.draftId case final id?) {
      session.draftId = null;
      unawaited(drafts?.remove(id));
    }
    unawaited(_eventSubscriptions.remove(session.id)?.cancel());
    unawaited(_taskSubscriptions.remove(session.id)?.cancel());
    unawaited(_contextSubscriptions.remove(session.id)?.cancel());
    _keepCursor(session);
    _stopHub(session);
    _clients.remove(session.id);
    session.chat.detach();
    _cancelSecrets(session);
    session.chat.onChange = null;
    _sessions.remove(session);
    if (_selectedId == session.id) {
      _selectedId = _sessions.isEmpty ? null : _sessions.first.id;
    }
    _notify();
  }

  /// Ends a conversation here: a question still open is answered cancelled, and its agent ends once no other conversation here
  /// uses it. What the agent saved stays with the agent, for Earlier conversations to open again.
  Future<void> close(ChatSession session) async {
    final client = _clients[session.id];
    session.chat.close();
    forget(session);
    if (client != null && !_clients.values.contains(client)) {
      await client.close();
    }
  }

  Future<void> cancelActiveSession() async {
    final session = selected;
    if (session == null) throw StateError('No active verified ACP session');
    await cancelSession(session);
  }

  /// Ends the turn of [session], which need not be the selected one.
  Future<void> cancelSession(ChatSession session) async {
    final client = _clients[session.id];
    if (session.remoteSessionId == null || client == null) {
      throw StateError('No active verified ACP session');
    }
    // ACP has a request still open at cancellation answered as cancelled, and the agent's turn cannot end until it is: the host's
    // prompts for secrets are such requests too.
    _cancelSecrets(session);
    await session.chat.cancel();
    await stopTasks(session);
  }

  /// Ends every task of [session] that has not ended, and nothing else: the conversation's own turn goes on. The host's answer is
  /// that they ended.
  Future<void> stopTasks(ChatSession session) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null || session.tasks.hostLive.isEmpty) {
      return;
    }
    if (await client.stopTasks(remote)) _tasksStopped(session);
  }

  /// Whether the person can hand [session] a task of their own: its agent is running on a host that takes tasks.
  bool canDelegate(ChatSession session) =>
      session.remoteSessionId != null && (_clients[session.id]?.takesTasks ?? false);

  /// What [session] may hand a task to, as its host lists it; with [source], that agent's models too. Null when the host did not say.
  Future<DelegateTargets?> delegateTargets(ChatSession session, {String? source}) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return null;
    return client.delegateTargets(remote, source: source);
  }

  /// What handing [kind] (`messages` or `files`) of [session] on would give a task, as the host would build it now.
  Future<ContextPreview?> delegateContext(ChatSession session, String kind) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return null;
    return client.delegateContext(remote, kind);
  }

  /// Sets what [session] may delegate to: the agents ([sources]), the one a task with none named goes to, how many run at once, and
  /// which context its agent may attach by its own call. Whether the host took it.
  Future<bool> delegateSetup(ChatSession session, {List<String>? sources, String? defaultSource, int? maxRunning, List<String>? context}) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return false;
    return client.delegateSetup(remote, sources: sources, defaultSource: defaultSource, maxRunning: maxRunning, context: context);
  }

  /// Hands [request] to the host as a task of [session]: the same A2A task the agent's own `delegate_task` makes, so the agent's
  /// `task_status` sees it and the lists here show it as the host's events arrive. Never sent again by itself; a [DelegateUncertain]
  /// is the person's to settle.
  Future<DelegateOutcome> delegateTask(ChatSession session, DelegateRequest request) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return const DelegateRefused('This conversation is not running, so it cannot hand a task on.');
    return client.delegateStart(remote, request);
  }

  /// Ends one of [session]'s tasks, which need not be running its turn.
  Future<void> stopTask(ChatSession session, String taskId) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    final task = session.tasks[taskId];
    if (client == null ||
        remote == null ||
        task == null ||
        task.native ||
        task.phase.ended) {
      return;
    }
    if (await client.stopTasks(remote, taskId: taskId)) {
      _tasksStopped(session, only: taskId);
    }
  }

  void _tasksStopped(ChatSession session, {String? only}) {
    if (only == null) {
      session.tasks.cancelLive(DateTime.now(), false);
      session.taskAllowances.clear();
    } else {
      session.tasks.cancelOne(only, DateTime.now());
      session.taskAllowances.remove(only);
    }
    for (final question in [...session.pendingPermissions]) {
      if (question.task != null &&
          (only == null || question.task!.id == only)) {
        session.chat.settle(question, const AcpCancel());
      }
    }
    _notify();
  }

  /// Attach explicit user-selected page context to the visible local session.
  /// This does not transmit the page to an agent. The eventual ACP adapter
  /// must obtain a separate user-approved context/send decision.
  void addPageContext(String encoded) {
    final session = selected;
    if (session == null || encoded.length > 12000) return;
    try {
      final decoded = jsonDecode(encoded);
      if (decoded is! Map<String, dynamic>) return;
      final url = decoded['url'];
      if (url is! String) return;
      final uri = Uri.tryParse(url);
      if (uri == null || !['http', 'https'].contains(uri.scheme)) return;
      final title = decoded['title'] is String
          ? decoded['title'] as String
          : '';
      final selection = decoded['selection'] is String
          ? decoded['selection'] as String
          : '';
      final suffix = selection.isEmpty
          ? ''
          : '\\nSelected text: ${selection.length > 8000 ? selection.substring(0, 8000) : selection}';
      session.chat.note(
        'Attached page context (not sent): ${title.isEmpty ? uri.host : title}\\n$uri$suffix',
      );
    } on FormatException {
      // Ignore malformed extension messages.
    }
  }

  void select(String id) {
    final chosen = _sessions.where((session) => session.id == id).firstOrNull;
    if (chosen == null) return;
    _selectedId = id;
    chosen.unread = false;
    chosen.failed = false;
    if (!chosen.git.tried) unawaited(refreshGit(chosen));
    _notify();
  }

  /// Pins [session] to the top of the list, or takes it down; kept with its ledger note where it has one.
  void setPinned(ChatSession session, bool pinned) {
    session.pinned = pinned;
    _keepPlacement(session);
    _notify();
  }

  /// Archives [session], out of the way of the list, or brings it back. One that needs the person stays where it is seen.
  void setArchived(ChatSession session, bool archived) {
    session.archived = archived;
    _keepPlacement(session);
    _notify();
  }

  void _keepPlacement(ChatSession session) {
    final peer = session.peerId, remote = session.remoteSessionId;
    if (peer != null && remote != null) {
      ledger?.setPlacement(
        peer,
        remote,
        pinned: session.pinned,
        archived: session.archived,
      );
    }
  }

  /// Names [session] as the person says; the agent stops naming it.
  void rename(ChatSession session, String title) {
    final named = title.trim();
    if (named.isEmpty) return;
    session.chat.rename(named);
    session.namedByPerson = true;
    session.titleFromMessage = false;
    final peer = session.peerId, remote = session.remoteSessionId;
    if (peer != null && remote != null) {
      ledger?.touch(peer, remote, title: session.title, byPerson: true);
    }
    _notify();
  }

  /// A new session that starts as [session] did: the same source, overlays and folder (the repository, in a worktree of its own
  /// where [session] had one). What was said in [session] is not copied: an agent's history is its own.
  ChatSession fork(ChatSession session) => createDraft(
    source: session.source,
    kind: session.kind,
    cwd: session.kind == SessionKind.chat
        ? null
        : session.worktree?.repo ?? session.cwd,
    worktree: session.worktree != null || session.wantsWorktree,
    overlays: session.overlays,
    loadout: session.loadout,
    title: 'New session like ${session.title}',
    keptSource: session.keptSource,
    cacheLifetime: session.cacheLifetime,
    account: session.account,
  );

  /// This does not send data to an agent. Only the clearly labeled preview
  /// session accepts text until the authenticated ACP transport is connected.
  void sendPreviewText(String text) {
    final session = selected;
    final trimmed = text.trim();
    if (session == null || trimmed.isEmpty) return;
    if (session.status != SessionStatus.preview) {
      throw StateError('A verified ACP transport is required to send');
    }
    session.chat.addMessage(MessageRole.user, trimmed);
    session.chat.note('Preview only — this message was not sent to an agent.');
  }

}
