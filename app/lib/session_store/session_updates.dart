part of '../session_store.dart';

extension SessionUpdates on SessionStore {
  /// A turn of [session] ended, stopped or failed. Failing in a session not shown is told to the person.
  void _turnEnded(ChatSession session, TurnOutcome outcome) {
    if (outcome == TurnOutcome.failed && session.id != _selectedId) {
      session.failed = true;
    }
    session
      ..lastTurnEnded = clock()
      ..lastTurnFailed = outcome == TurnOutcome.failed;
    _watchExhaustion(session);
    _keepText(session);
    _keepCursor(session);
    unawaited(refreshGit(session));
  }

  /// The agent names the conversation, which is then no longer taken from the first message.
  void _agentNamed(ChatSession session, String title) {
    session.titleFromMessage = false;
    if (session.peerId != null && session.remoteSessionId != null) {
      ledger?.touch(session.peerId!, session.remoteSessionId!, title: title);
    }
  }

  /// A prompt of [session] is about to go: the disk as it is now is what the turn is measured from, and the ledger notes it.
  void _prompting(ChatSession session) {
    _gitBeforeTurn(session);
    if (session.peerId != null && session.remoteSessionId != null) {
      ledger?.touch(session.peerId!, session.remoteSessionId!);
    }
  }

  /// The agent took a choice the person made: it is the source's default from now on.
  void _chosen(ChatSession session, OptionRole role, String value) {
    // A switch inside a conversation is one quiet line in it; the first choices, made before the conversation began, are not.
    if ((role == OptionRole.model || role == OptionRole.effort) &&
        !session.isDraft &&
        !isStarting(session) &&
        !session.opening &&
        session.messages.isNotEmpty) {
      final choice = optionIn(
        session.options,
        role,
      )?.choices.where((each) => each.value == value).firstOrNull;
      session.chat.note(
        '${role == OptionRole.model ? 'Model' : 'Effort'}: ${choiceLabel(role, choice ?? Choice(value, value))}',
      );
    }
    final source = session.source;
    if (source == null) return;
    ledger?.rememberChoice(sourceKey(source), role, value);
    _rememberSource(session);
  }

  /// Notes in the ledger the mode that was set, and whether Canoa's Bypass is on, to put both back when [session] is opened
  /// again. The agent's starting default was set by nobody and is not kept; nor is `custom`, which cannot be chosen.
  void _keepMode(ChatSession session, String? mode, bool bypass) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    if (ledger == null ||
        remote == null ||
        peerId == null ||
        session.isDraft ||
        session.opening) {
      return;
    }
    ledger!.setMode(peerId, remote, mode: mode, bypass: bypass);
  }

  /// The agent changed its own mode, which is where the session now is: said when it follows the person's "always" answer.
  void _agentChangedMode(ChatSession session, String now) {
    final answered = session.alwaysAnsweredAt;
    if (answered == null ||
        DateTime.now().difference(answered) >= const Duration(seconds: 30)) {
      return;
    }
    session.alwaysAnsweredAt = null;
    final label =
        modeEntries(
          modeOptionIn(session.options),
        ).where((entry) => entry.value == now).firstOrNull?.label ??
        now;
    session.chat.note(
      'After your "always" answer, ${shortLine(session.agent) ?? 'the agent'} switched this session to $label. '
      'The mode control changes it back.',
    );
  }

  /// Follows the tasks the host reports for [session]. The tasks of an earlier connection ended with it, and the host keeps none for
  /// the next, so a reconnect asks for nothing.
  void _watchTasks(ChatSession session, VerifiedAgent client) {
    unawaited(_taskSubscriptions.remove(session.id)?.cancel());
    _taskSubscriptions[session.id] = client.taskEvents.listen(
      (notice) => _taskEvent(session, notice),
    );
  }

  /// Follows what the host's hub says of [session]: who else is attached, a request another client answered, and where in its
  /// log this app is. All of it is empty for a session the host does not share.
  void _watchHub(ChatSession session, VerifiedAgent client) {
    _stopHub(session);
    final remote = session.remoteSessionId;
    session.others = remote == null ? const [] : client.hub.others(remote);
    _hubSubscriptions[session.id] = [
      client.updates.listen((update) {
        if (update.sessionId != session.remoteSessionId) return;
        session.cursor = client.hub.cursorOf(update.sessionId) ?? session.cursor;
      }),
      client.hub.events.listen((event) => _hubEvent(session, client, event)),
    ];
  }

  void _stopHub(ChatSession session) {
    final held = _hubSubscriptions.remove(session.id);
    if (held == null) return;
    for (final subscription in held) {
      unawaited(subscription.cancel());
    }
  }

  void _hubEvent(ChatSession session, VerifiedAgent client, HubEvent event) {
    if (event.sessionId != session.remoteSessionId) return;
    switch (event) {
      case HubPresence():
        session.others = client.hub.others(event.sessionId);
        _keepCursor(session);
        _notify();
      case HubClosed():
        _closedElsewhere(session, client, event);
      case HubEnded():
        session.others = const [];
        session.chat.note('The host ended this session, so nobody can go on with it.');
        _keepCursor(session);
        _notify();
    }
  }

  /// Another client answered a request this conversation was still asking: it closes here, and says who answered.
  void _closedElsewhere(ChatSession session, VerifiedAgent client, HubClosed closed) {
    final call = closed.toolCallId;
    if (call == null) return;
    for (final question in [...session.pendingPermissions]) {
      if (question.toolCallId != call) continue;
      final by = closed.by;
      final who = by == null
          ? 'The host'
          : (client.hub.clientOf(closed.sessionId, by)?.named ?? 'Another client');
      final chosen = closed.outcome?['optionId'];
      final how = chosen is String
          ? (question.options.where((option) => option.optionId == chosen).firstOrNull?.name ?? chosen)
          : null;
      session.chat.settle(question, const AcpCancel());
      session.chat.note(
        '$who answered "${shortLine(question.title) ?? 'a request'}"'
        '${how == null || how.isEmpty ? '' : ': $how'}.',
      );
    }
  }

  /// Notes in the ledger where in the host's log of [session] this app is.
  void _keepCursor(ChatSession session) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    final cursor = session.cursor;
    if (ledger == null || remote == null || peerId == null || cursor == null) return;
    ledger!.setCursor(peerId, remote, cursor);
  }

  /// Keeps what each usage report of [session]'s agent says its context holds, which the shared model does not keep (a report
  /// that names no kinds replaces the last ones, since the figures they split are no longer the window's), and when each of its
  /// terminals was first heard of and ended.
  void _watchContext(ChatSession session, VerifiedAgent client) {
    unawaited(_contextSubscriptions.remove(session.id)?.cancel());
    _contextSubscriptions[session.id] = client.updates.listen((update) {
      if (update.sessionId != session.remoteSessionId) return;
      if (a2uiCarried(update.raw['_meta']) case final carried?) {
        a2uiOf(session).carry(carried);
      }
      if (update.kind == 'terminal_update' ||
          update.kind == 'terminal_output_chunk') {
        final id = update.raw['terminalId'];
        if (id is! String) return;
        final span = session.terminalSpans[id] ??= TerminalSpan(clock());
        final exit = update.raw['exitStatus'];
        if (span.ended == null && exit is Map && exit['exitCode'] is int) {
          span.ended = clock();
        }
        return;
      }
      if (update.kind != 'usage_update') return;
      session.contextKinds = contextKindsOf(update.raw);
      _notify();
    });
  }

  void _taskEvent(ChatSession session, DelegateNotice notice) {
    if (notice['sessionId'] != session.remoteSessionId ||
        !session.tasks.apply(notice, at: clock())) {
      return;
    }
    final ended = [
      for (final task in session.tasks.all)
        if (task.phase.ended) task.id,
    ];
    for (final id in ended) {
      session.taskAllowances.remove(id);
      // The host gave the task's question up with the task, so a card still waiting for it has nothing to answer.
      for (final question in [...session.pendingPermissions]) {
        if (question.task?.id == id) {
          session.chat.settle(question, const AcpCancel());
        }
      }
    }
    _notify();
  }

  /// Asks the agent of [session] for its usage report now, as it does after each turn.
  Future<void> refreshUsage(ChatSession session) => session.chat.refreshUsage();

  /// Notes [session] in the ledger, with what its agent said about itself.
  void _record(ChatSession session, VerifiedAgent client) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    if (ledger == null || remote == null || peerId == null) return;
    final identity = client.identity;
    ledger!.record(
      peerId: peerId,
      sessionId: remote,
      entry: session.agent,
      place: session.place,
      reportedName: identity.name,
      markers: identity.markers,
      version: identity.version,
      reopenBy: identity.reopenBy,
      cwd: session.cwd,
      title: session.titleFromMessage ? session.agent : session.title,
      endpoint: session.endpoint?.toString(),
      fallback: session.websocketFallback?.toString(),
      method: session.method,
      worktree: session.worktree,
      canoaId: session.workspaceId,
      kind: session.kind,
      named: session.namedByPerson,
      site: session.site,
      relation: session.relation,
      cacheLifetime: session.cacheLifetime?.id,
      account: session.account,
      overlays: session.overlays?.toList(),
    );
  }

  /// Keeps what was said in [session] with its ledger entry, which the ledger does only when the person turned that on.
  void _keepText(ChatSession session) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    if (ledger == null ||
        remote == null ||
        peerId == null ||
        !ledger!.settings.keepText) {
      return;
    }
    ledger!.keepText(peerId, remote, [
      for (final message in session.messages)
        if (message.role == MessageRole.user ||
            message.role == MessageRole.agent)
          KeptMessage(message.role.name, message.text, message.at),
    ]);
  }

  /// What the agent said when it opened [session].
  void applyOpened(ChatSession session, AcpOpenedSession opened) {
    session.chat.applyOpened(opened);
    _takeHostReport(session, opened);
    _notify();
  }

  void _takeHostReport(ChatSession session, AcpOpenedSession opened) {
    final meta = opened.raw['_meta'];
    session.hostReport = meta is Map ? meta[hostProfileMeta] : null;
  }

  /// Why [session] cannot go on under another account now; null when it can. Where an account keeps its conversations is the
  /// host's to say when it moves one, so a refusal for that comes with the move, in the conversation's own words.
  String? accountSwitchBlock(ChatSession session) {
    if (session.isDraft) return null;
    final client = _clients[session.id];
    if (session.status == SessionStatus.preview ||
        session.site != null ||
        session.place == SessionPlace.browser) {
      return 'This conversation cannot be moved to another account.';
    }
    if (session.status == SessionStatus.connecting || session.opening) {
      return 'Wait for the agent to finish starting.';
    }
    if (session.status == SessionStatus.working) {
      return 'Wait for the turn to end, or stop it.';
    }
    if (session.needsYou) return 'Answer what the agent asked first.';
    if (client == null || client.isClosed) return 'The agent is not connected.';
    if (session.remoteSessionId == null ||
        session.cwd == null ||
        client.identity.reopenBy == null ||
        !client.canReopenSessions) {
      return 'The agent cannot open this conversation again, which a new account needs.';
    }
    if (_clients.entries.any(
      (each) => each.key != session.id && identical(each.value, client),
    )) {
      return 'Another conversation uses the same agent. Close it first.';
    }
    return null;
  }

  /// An account as a note names it: its name, or what the login the agent has where it runs is called.
  String _accountWords(String account) =>
      account == machineLogin ? "this machine's login" : account;

}
