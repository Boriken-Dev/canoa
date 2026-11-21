part of '../session_store.dart';

extension SessionSwitching on SessionStore {
  /// Moves the running [session] to [account], which is [machineLogin] for the login the agent has where it runs. Its agent ends, [move] carries the conversation to the account's own folder when
  /// the accounts keep theirs apart (answering the id it has there), [start] starts the agent again for the account, and the
  /// conversation is opened on it with the model, effort and mode it was in. Where the accounts share a folder it is the same
  /// conversation, resumed as it is. If any step fails the agent is started again on the account it had and the conversation is
  /// opened there, and the transcript says why. Answers whether it moved.
  Future<bool> switchAccount(
    ChatSession session,
    String account, {
    required Future<VerifiedAgent> Function(ChatSession session) start,
    Future<AccountMove> Function(ChatSession session, String? from, String to)?
    move,
    bool expected = false,
    ({String pool, String why})? pool,
  }) async {
    final before = session.account;
    // A session on a pool goes on naming it and moves between its accounts: it runs on [ranOn], the host's word for it.
    final ranOn = session.ranOnAccount;
    if (pool == null ? before == account : ranOn == account) return true;
    final block = accountSwitchBlock(session);
    if (block != null) {
      session.chat.note(
        'Account: not moved to ${_accountWords(account)}. $block',
      );
      return false;
    }
    final old = _clients[session.id]!;
    _moving.add(session.id);
    final oldId = session.remoteSessionId!, cwd = session.cwd!;
    for (final role in const [OptionRole.model, OptionRole.effort]) {
      final option = optionIn(session.options, role);
      if (option != null && option.current.isNotEmpty) {
        session.chat.holdChoice(PendingChoice(option.id, role, option.current));
      }
    }
    final keptMode = session.chat.underlyingMode ?? session.chat.mode,
        keptBypass = session.bypass;
    final oldEntry = session.peerId == null
        ? null
        : ledger?.find(session.peerId!, oldId);

    /// Starts the agent for the session's account and opens [id] on it.
    Future<VerifiedAgent> reopenOn(String id) async {
      final client = await start(session);
      try {
        session.remoteSessionId = id;
        attachVerifiedClient(session.id, client, record: false);
        final opened = await session.chat.reopen(
          () => client.reopenSession(id, cwd: cwd, setup: session.setup),
          keptMode: keptMode,
          keptBypass: keptBypass,
        );
        _takeHostReport(session, opened);
        await applyHostProfile(session, expected: expected, keptMode: keptMode);
      } catch (_) {
        await client.close();
        rethrow;
      }
      return client;
    }

    AccountMove? moved;
    try {
      // The old agent holds the account's login, and the conversation is read from its folder: it ends first.
      await old.close();
      moved = move == null
          ? null
          : await move(session, pool == null ? before : ranOn, account);
      session
        ..account = pool?.pool ?? account
        ..startAccount = pool == null ? null : account;
      final client = await reopenOn(
        moved != null && moved.moved ? moved.session : oldId,
      );
      session
        ..ranOnAccount = client.identity.ranOnAccount ?? (pool == null ? null : account)
        ..ranOnPool = pool?.pool ?? client.identity.ranOnPool;
      if (moved != null && moved.moved) {
        // The conversation has a new id in the new folder: the list follows it, with what the person had made of it.
        if (oldEntry != null) ledger?.remove(oldEntry);
        _record(session, client);
        if (oldEntry != null && session.peerId != null) {
          ledger?.setPlacement(
            session.peerId!,
            session.remoteSessionId!,
            pinned: oldEntry.pinned,
            archived: oldEntry.archived,
          );
        }
      } else {
        _record(session, client);
      }
    } catch (error) {
      session
        ..account = before
        ..startAccount = pool == null ? null : ranOn;
      try {
        await reopenOn(oldId);
        session.chat.note(
          'Account: not moved to ${_accountWords(account)}: ${reasonOf(error)}. This conversation stays on ${before == null ? 'its first account' : _accountWords(before)}.',
        );
      } catch (again) {
        session.chat.note(
          'Account: not moved to ${_accountWords(account)}: ${reasonOf(error)}. Its agent could not be started again on ${before == null ? 'its first account' : _accountWords(before)} either: ${reasonOf(again)}. Open it again from Earlier conversations.',
        );
      }
      session.startAccount = null;
      _moving.remove(session.id);
      _notify();
      return false;
    }
    session.startAccount = null;
    _moving.remove(session.id);
    _watchExhaustion(session);
    final how = moved != null && moved.moved
        ? 'moved to that account\'s own folder (${moved.via == 'record' ? 'from what this host recorded of it' : 'by the agent\'s own export'}) and picked up'
        : 'picked up as it was';
    session.chat.note(
      pool == null
          ? 'Account: ${before == null ? 'the first' : _accountWords(before)} → ${_accountWords(account)}. The conversation was $how.'
          : 'Pool ${pool.pool}: moved from ${ranOn == null ? 'its first account' : _accountWords(ranOn)} to ${_accountWords(account)}: ${pool.why}. The conversation was $how.',
    );
    _notify();
    return true;
  }

  /// What a limit the agent reported for the account [session] runs on comes to: a hint for the host while it holds, which is until
  /// the time it names, or with none until the turn it refused is no longer the last.
  List<LimitHint> _limitsOf(ChatSession session) {
    final account = session.ranOnAccount, hit = session.usageReport?.limit;
    if (account == null || hit == null) return const [];
    final now = clock().millisecondsSinceEpoch ~/ 1000;
    final until =
        hit.resetsAt ??
        (hit.retryAfter == null ? null : hit.observedAt + hit.retryAfter!.inSeconds);
    if (until != null ? until <= now : !session.lastTurnFailed) return const [];
    return [(account: account, resetsAt: until)];
  }

  /// What the host is asked of a turn of [session] on [account]: the model it is on, how long ago its last turn ended and how long
  /// its cache lives.
  Future<TurnAdvice> _askTurn(ChatSession session, String account) {
    final ended = session.lastTurnEnded;
    final model = optionIn(session.options, OptionRole.model)?.current;
    return poolTurn!(
      session,
      account: account,
      model: model == null || model.isEmpty ? null : model,
      current: session.ranOnAccount,
      idleSeconds: ended == null
          ? null
          : clock().difference(ended).inSeconds.clamp(0, 1 << 40).toInt(),
      lifetime: cacheLifetimeOf(session).lifetime.id,
      limited: _limitsOf(session),
    );
  }

  /// Whether a message on [session] first asks the host which account of its pool takes the turn. A message on any other session
  /// goes at once, with no wait in between, so what the agent sends back cannot arrive before the message is in the conversation.
  bool _asksPool(ChatSession session) =>
      session.account != null &&
      poolTurn != null &&
      moveAccount != null &&
      session.ranOnAccount != null &&
      session.status != SessionStatus.working &&
      (runsOnPool?.call(session) ?? false);

  /// Before a message goes on a session that was started on a pool: the host says which account of the pool takes the turn, and
  /// when that is not the one the session runs on, the conversation moves to it first. A refusal (no account has usage left) stops
  /// the message, which stays with the person, and says why in the conversation. A host that cannot be asked never holds a message
  /// back.
  Future<void> _beforeTurn(ChatSession session) async {
    final pool = session.account;
    if (pool == null || !_asksPool(session)) return;
    final TurnAdvice advice;
    try {
      advice = await _askTurn(session, pool);
    } on TurnRefused catch (refused) {
      session.chat.note('Pool $pool: ${refused.message}');
      _notify();
      throw StateError(refused.message);
    } catch (_) {
      return;
    }
    final from = session.ranOnAccount;
    if (!advice.moved || advice.account == from) return;
    await moveAccount!(
      session,
      advice.account,
      pool: (pool: pool, why: SessionStore._whyMoved(advice, from)),
    );
  }

  /// What has run out for [account], as the session's own report says it: a bucket that is exhausted for the session's model and
  /// whose window has not ended, or a refusal for a limit that still holds. Null when there is room.
  String? _exhaustedFor(ChatSession session) {
    final report = session.usageReport;
    if (report == null) return null;
    final now = clock().millisecondsSinceEpoch ~/ 1000;
    final model = optionIn(session.options, OptionRole.model)?.current;
    for (final bucket in report.buckets) {
      if (!bucket.isExhausted || bucket.pastReset(now)) continue;
      final covers = bucket.models.kind == 'only'
          ? model != null && bucket.models.models.contains(model)
          : true;
      if (covers) return '${bucket.name} is used up';
    }
    if (_limitsOf(session).isNotEmpty) return 'a turn was refused for a limit';
    return null;
  }

  /// When the account a single-account session runs on has run out, asks the host what could take it and offers the person the
  /// move, once. Nothing moves by itself, and nothing is offered again until the account has had room.
  void _watchExhaustion(ChatSession session) {
    if (session.isDraft || session.status == SessionStatus.preview || _moving.contains(session.id)) return;
    final account = session.ranOnAccount ?? session.account;
    if (account == null || account == machineLogin || poolTurn == null) return;
    final left = _exhaustedFor(session);
    if (left == null) {
      session.offerHeld = false;
      session.accountOffer = null;
      return;
    }
    if (session.offerHeld || (runsOnPool?.call(session) ?? false)) return;
    session.offerHeld = true;
    unawaited(() async {
      try {
        final advice = await _askTurn(session, account);
        if (_disposed || !advice.exhausted || (session.ranOnAccount ?? session.account) != account) return;
        final others = advice.alternatives.take(3).toList();
        if (others.isEmpty) {
          session.chat.note(
            'Account ${_accountWords(account)} has run out (${advice.left ?? left}), and no other account of this agent could take the conversation.',
          );
        } else {
          session.accountOffer = AccountOffer(
            account: account,
            alternatives: others,
            left: advice.left ?? left,
            refillsAt: advice.refillsAt,
          );
        }
        _notify();
      } catch (_) {
        // A host that cannot say leaves the person with the conversation as it is; it is asked again when the account is.
        session.offerHeld = false;
      }
    }());
  }

  /// Moves [session] to [account], which its offer named, and takes the offer away. It is the person's own move.
  Future<bool> takeAccountOffer(ChatSession session, String account) async {
    session.accountOffer = null;
    _notify();
    final move = moveAccount;
    return move == null ? false : move(session, account);
  }

  /// Takes the offer away, and does not make it again until the account has had room.
  void dismissAccountOffer(ChatSession session) {
    session.accountOffer = null;
    session.offerHeld = true;
    _notify();
  }

  /// Why [session] cannot change agent now; null when it can. The conversation moves by the host's export and import, so it
  /// has to be one of this computer's own agents, started, and at rest.
  String? agentSwitchBlock(ChatSession session) {
    if (session.isDraft) return null;
    final client = _clients[session.id];
    if (session.status == SessionStatus.preview ||
        session.site != null ||
        session.place != SessionPlace.local) {
      return 'Only a conversation with one of this computer\'s own agents can change agent.';
    }
    if (session.status == SessionStatus.connecting || session.opening) {
      return 'Wait for the agent to finish starting.';
    }
    if (session.status == SessionStatus.working) {
      return 'Wait for the turn to end, or stop it.';
    }
    if (session.needsYou) return 'Answer what the agent asked first.';
    if (client == null || client.isClosed) return 'The agent is not connected.';
    if (session.remoteSessionId == null || session.cwd == null) {
      return 'The agent has not said where this conversation is.';
    }
    if (_clients.entries.any(
      (each) => each.key != session.id && identical(each.value, client),
    )) {
      return 'Another conversation uses the same agent. Close it first.';
    }
    return null;
  }

  /// Moves the running [session] to the agent [target]: the host exports the conversation from the agent it is with and imports it
  /// into [target] as that agent's own session ([move], answering the new id and what did not carry), [start] starts [target]'s
  /// agent, and the conversation opens on it, so the new agent has all of what was said. The agent it leaves keeps its session
  /// and the link to it, and a switch back brings what was said since: its whole conversation again at a new id, or its own
  /// session as it was when nothing was said in between. If any step fails the conversation stays with the agent it had, which was
  /// never stopped, and the transcript says why. Answers whether it moved.
  Future<bool> switchAgent(
    ChatSession session,
    AgentSource target, {
    required Future<MoveReport> Function(ChatSession session, String to) move,
    required Future<VerifiedAgent> Function(
      ChatSession session,
      AgentSource target,
    )
    start,
  }) async {
    final from = session.agent, fromSource = session.source;
    if (from == target.label) return true;
    final fromName = fromSource?.displayName ?? from,
        toName = target.displayName;
    final block = agentSwitchBlock(session);
    if (block != null) {
      session.chat.note('Agent: not moved to $toName. $block');
      return false;
    }
    final old = _clients[session.id]!;
    final oldId = session.remoteSessionId!, cwd = session.cwd!;
    final said = SessionStore._spoken(session);
    final earlier = session.agentLinks[target.label];
    final unchanged = earlier != null && earlier.said == said.length;
    final oldEntry = session.peerId == null
        ? null
        : ledger?.find(session.peerId!, oldId);
    final oldPeer = session.peerId,
        oldAccount = session.account,
        oldSetup = session.setup,
        oldProfile = session.profile;

    MoveReport? report;
    final String newId;
    try {
      if (unchanged) {
        newId = earlier.id;
      } else {
        report = await move(session, target.label);
        newId = report.session;
      }
    } catch (error) {
      session.chat.note(
        'Agent: not moved to $toName: ${reasonOf(error)}. This conversation stays with $fromName.',
      );
      _notify();
      return false;
    }

    VerifiedAgent? fresh;
    var changed = false;
    try {
      fresh = await start(session, target);
      if (!fresh.canReopenSessions) {
        throw AcpFailure(
          '$toName offers neither session/load nor session/resume',
        );
      }
      changed = true;
      session
        ..agent = target.label
        ..source = target
        ..peerId = fresh.peerId
        ..account = null
        ..setup = AcpSessionSetup.none
        ..profile = null
        ..remoteSessionId = newId;
      attachVerifiedClient(session.id, fresh, record: false);
      final opened = await session.chat.reopen(
        () => fresh!.reopenSession(newId, cwd: cwd),
      );
      _takeHostReport(session, opened);
      await applyHostProfile(session);
      // An agent that sent back less than was said has shown less than the conversation holds: what it was is put back.
      bool isSpoken(model.ChatMessage m) =>
          m.role == MessageRole.user || m.role == MessageRole.agent;
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
        attachVerifiedClient(session.id, old, record: false);
      }
      await fresh?.close();
      session.chat.note(
        'Agent: not moved to $toName: ${reasonOf(error)}. This conversation stays with $fromName.',
      );
      _notify();
      return false;
    }
    session.agentLinks[from] = (id: oldId, said: said.length);
    await old.close();
    // The conversation has another id, with another agent: the list follows it, with what the person had made of it.
    if (oldEntry != null) ledger?.remove(oldEntry);
    _record(session, fresh);
    if (oldEntry != null && session.peerId != null) {
      ledger?.setPlacement(
        session.peerId!,
        newId,
        pinned: oldEntry.pinned,
        archived: oldEntry.archived,
      );
    }
    session.chat.note(SessionStore._switchLine(fromName, toName, report));
    _notify();
    return true;
  }

  /// A person's choice of one of the agent's options. It goes to the agent now when it can take it; before the agent starts, or
  /// while a turn runs, it waits and goes before the next prompt. [value] null takes back a choice still waiting.
  /// Answers whether the agent refused it, which a choice still waiting has not yet.
  Future<bool> choose(
    ChatSession session,
    ConfigOption option,
    String? value,
  ) async {
    if (value != null && (session.isDraft || isStarting(session))) {
      final role = roleOf(option);
      session.chat.holdChoice(
        PendingChoice(option.id, role, value, byPerson: true),
      );
      if (session.isDraft && role == OptionRole.model) {
        _dropUnofferedEffort(session);
      }
      _keepDraft(session);
      return false;
    }
    final refused = await session.chat.choose(option, value);
    if (value == null) _keepDraft(session);
    return refused;
  }

  /// Sends the choices the person made before [session]'s agent ran, now that it has started and before its first message: the
  /// mode and the model go together, and the other options, the effort among them, once the model has answered, since the
  /// choices an option offers can depend on the model. Whatever is left waiting goes before the prompt as the chat sends it.
  Future<void> applyHeldChoices(ChatSession session) async {
    final chat = session.chat;
    ConfigOption? offered(PendingChoice choice) {
      final options = chat.options;
      return options
              .where((option) => option.id == choice.id && option.choices.any((each) => each.value == choice.value))
              .firstOrNull ??
          options
              .where((option) => roleOf(option) == choice.role && option.choices.any((each) => each.value == choice.value))
              .firstOrNull;
    }

    final held = [
      for (final choice in session.pending.values)
        // Canoa's own Bypass sits on the agent's mode, which the chat sets itself.
        if (choice.byPerson && !(choice.role == OptionRole.mode && (choice.value == 'custom' || chat.bypass))) choice,
    ];
    final together = [
      for (final choice in held)
        if (choice.role == OptionRole.mode || choice.role == OptionRole.model) choice,
    ];
    Future<void> send(PendingChoice choice) async {
      final option = offered(choice);
      if (option == null) return;
      if (choice.role == OptionRole.mode) {
        await chat.chooseMode(ModeEntry(choice.value, choice.value));
      } else if (option.current != choice.value) {
        await chat.choose(option, choice.value);
      }
    }

    await Future.wait([for (final choice in together) send(choice)]);
    await Future.wait([
      for (final choice in held)
        if (!together.contains(choice)) send(choice),
    ]);
  }

  /// A new model's efforts replace the old one's: an effort chosen for the draft that the model does not take is taken back.
  void _dropUnofferedEffort(ChatSession session) {
    final effort = optionIn(optionsOf(session), OptionRole.effort);
    session.chat.dropChoices(
      where: (choice) =>
          choice.role == OptionRole.effort &&
          effort != null &&
          !effort.choices.any((each) => each.value == choice.value),
    );
  }

  /// Finishes applying the profile the host prepared for [session], from what it reported when the session opened: the model,
  /// effort and mode are set where the agent offers an option with that value, and the report goes on the session and, in one
  /// line, into it. A reopened session's [keptMode], the one it was left in and which is put back first, wins over the profile's
  /// mode, which is applied only if the agent did not take the kept one. With [expected] and no report, the session says it was
  /// given none.
  Future<void> applyHostProfile(
    ChatSession session, {
    bool expected = false,
    String? keptMode,
  }) async {
    final chat = session.chat;
    final reported = hostProfileOf(session.hostReport, fallback: session.agent);
    session.hostReport = null;
    if (reported == null || reported.report.items.isEmpty) {
      if (reported == null && expected) {
        chat.note('The host gave this agent no profile.');
      }
      _notify();
      return;
    }
    final chosen = SessionStore._choicesFor(reported.values, [
      for (final option in chat.options)
        (
          option.id,
          option.category,
          [for (final choice in option.choices) choice.value],
          option.isMode,
        ),
    ]);
    final keptTaken =
        keptMode != null &&
        (keptMode == 'custom' ||
            modeOptionIn(chat.options)?.current == keptMode);
    final set = <String, String>{};
    final wanted = [
      for (final item in chosen)
        if (!(keptTaken && item.$3 == 'mode')) item,
    ];
    Future<void> apply((String, String, String) item) async {
      final (id, value, _) = item;
      final option = chat.options.firstWhere((option) => option.id == id);
      // Canoa's own Bypass stays where it is: the mode set is the agent's, which it sits on.
      if (option.isMode && !chat.bypass) {
        await chat.chooseMode(ModeEntry(value, value));
      } else {
        await chat.choose(option, value);
      }
    }

    // The mode and the model go together; the effort and the rest follow the model, whose choices theirs can depend on.
    bool first((String, String, String) item) => item.$3 == 'mode' || item.$3 == 'model';
    await Future.wait([for (final item in wanted) if (first(item)) apply(item)]);
    await Future.wait([for (final item in wanted) if (!first(item)) apply(item)]);
    for (final (id, value, key) in wanted) {
      final option = chat.options.firstWhere((option) => option.id == id);
      if (chat.options
              .where((option) => option.id == id)
              .firstOrNull
              ?.current ==
          value) {
        set[key] = option.isMode
            ? 'session/set_mode'
            : 'session/set_config_option $id';
      }
    }
    ProfileItem settled(ProfileItem item) {
      if (!const {'model', 'effort', 'mode'}.contains(item.key)) return item;
      final how = set[item.key];
      if (keptTaken && item.key == 'mode') {
        return ProfileItem(
          item.key,
          item.layer,
          ItemState.off,
          'the mode this session was left in is applied instead',
        );
      }
      return how != null
          ? ProfileItem(item.key, item.layer, ItemState.applied, how)
          : ProfileItem(
              item.key,
              item.layer,
              ItemState.unsupported,
              'the agent offers no ${item.key} option with this value',
            );
    }

    session.profile = ProfileReport(
      reported.report.harness,
      [for (final item in reported.report.items) settled(item)],
      loadout: reported.report.loadout,
    );
    chat.note(session.profile!.summary);
    _notify();
  }

}
