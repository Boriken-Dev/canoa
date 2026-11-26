part of '../session_store.dart';

extension SessionDrafts on SessionStore {
  /// What a conversation tells the store, and what the store decides of it for the conversation.
  void _wire(ChatSession session) {
    final chat = session.chat;
    chat.attachClient(ownerClient, name: Brand.name);
    chat.onChange = () => _changed(session);
    chat.onAttention = (kind) => _needs(session, kind);
    chat.onModeKept = (mode, bypass) => _keepMode(session, mode, bypass);
    chat.onPermissionSettled = (question) =>
        _permissionSettled(session, question);
    chat.onPrompt = (_, _) => _prompting(session);
    chat.onChoice = (role, value) => _chosen(session, role, value);
    chat.onOptions = () => _rememberSource(session);
    chat.onTitle = (title) => _agentNamed(session, title);
    chat.onAgentMode = (before, now) => _agentChangedMode(session, now);
    chat.permissionPolicy = _Policy(this, session);
    chat.bypassOnceOnly = bypassOnceOnly;
    chat.queueDelivery = _queueDelivery;
    // A mode the agent moved to by itself was not chosen, so it is neither kept for the ledger to put back as the person's
    // request nor remembered as the session's chosen mode.
    chat.agentModeIsChosen = false;
    chat.modeLabel = (choice) => choiceLabel(OptionRole.mode, choice);
    _eventSubscriptions[session.id] = chat.events.listen((event) {
      if (event is TurnEvent) {
        _turnEnded(session, event.turn.outcome ?? TurnOutcome.completed);
      }
    });
  }

  /// A conversation changed: when its connection ended, what Canoa held for that connection ends with it.
  void _changed(ChatSession session) {
    session.tasks.syncNative(
      session.chat.children,
      clock(),
      session.chat.agent,
    );
    if (_clients[session.id] != null && session.chat.connection == null) {
      _clients.remove(session.id);
      _strand(session);
    }
    final report = session.usageReport, account = report?.account;
    if (report != null &&
        account != null &&
        (accountUsage[account]?.revision ?? -1) <= report.revision) {
      accountUsage[account] = report;
    }
    _watchExhaustion(session);
    _watchRefusals(session);
    _notify();
  }

  ChatSession createPreviewSession({
    String title = 'New conversation',
    String agent = '${Brand.name} preview',
  }) {
    final session = _newSession(
      title: title,
      agent: agent,
      status: SessionStatus.preview,
    );
    _sessions.insert(0, session);
    _wire(session);
    _selectedId = session.id;
    _notify();
    return session;
  }

  /// A new session that has started nothing: its agent starts on its first message, from [source] in [cwd], and a worktree
  /// is made then if [worktree]. It is kept between runs until it starts or is closed, and never noted in the ledger before.
  ChatSession createDraft({
    AgentSource? source,
    String? cwd,
    bool worktree = false,
    Set<String>? overlays,
    String? id,
    String text = '',
    Map<String, PendingChoice> pending = const {},
    List<Attachment> attachments = const [],
    String? title,
    String? keptSource,
    bool bypass = false,
    String? workspaceId,
    DateTime? created,
    CacheLifetime? cacheLifetime,
    String? account,
    SessionKind? kind,
    String? loadout,
    String? host,
    String? preset,
    Map<String, String> presetWants = const {},
    String? sideOf,
  }) {
    final session =
        _newSession(
            title: title ?? SessionStore.draftTitle,
            agent: source?.label ?? SessionStore.draftTitle,
            status: SessionStatus.draft,
            cwd: cwd,
            source: source,
            created: created,
            agentNames: title == null,
          )
          ..kind =
              kind ?? (cwd == null ? SessionKind.chat : SessionKind.workspace)
          ..wantsWorktree = worktree
          ..cacheLifetime = cacheLifetime
          ..account = account
          ..overlays = overlays
          ..loadout = loadout
          ..host = host
          ..preset = preset
          ..sideOf = sideOf
          ..draftId = sideOf == null ? id ?? newDraftId() : null
          ..workspaceId = workspaceId ?? newSessionUuid()
          ..draftText = text
          ..keptSource = source == null ? keptSource : null
          ..bypassBeforeAgent = bypass;
    _insert(session);
    session.chat.presetBypass(bypass);
    session.presetWants.addAll(presetWants);
    for (final choice in pending.values) {
      session.chat.holdChoice(choice);
    }
    if (preset == null && presetWants.isEmpty && pending.isEmpty) _wantDefaultPreset(session);
    session.attachments.addAll(attachments.take(maxAttachments));
    _keepDraft(session);
    _askChoices(source);
    return session;
  }

  /// What [session]'s agent is asked for its prompt cache: the person's choice, else the lifetime canoa.cfg's entry sets, else
  /// the chat's default. [chosen] says the person chose it, so a host lets it win over its entry's setting. A lifetime the
  /// provider has said it does not honour is not asked: the agent's default is, and [chosen] is false.
  ({CacheLifetime lifetime, bool chosen}) cacheLifetimeOf(ChatSession session) {
    final source = session.source;
    final choices = source == null ? null : choicesOf(source);
    final asked = session.cacheLifetime;
    final wanted = asked ?? choices?.cacheLifetimeSetting ?? CacheLifetime.chat;
    if (choices != null &&
        choices.cacheLifetime != null &&
        !choices.offersCache(wanted)) {
      return (lifetime: CacheLifetime.harness, chosen: false);
    }
    return (lifetime: wanted, chosen: asked != null);
  }

  /// The lifetime the person chose for [session] that its provider does not honour, which [cacheLifetimeOf] then replaces with
  /// the agent's default; null when the choice stands or none was made.
  CacheLifetime? cacheUnavailable(ChatSession session) {
    final asked = session.cacheLifetime;
    final source = session.source;
    final choices = source == null ? null : choicesOf(source);
    if (asked == null ||
        choices == null ||
        choices.cacheLifetime == null ||
        choices.offersCache(asked)) {
      return null;
    }
    return asked;
  }

  /// Sets how long [session]'s prompt cache should live; null takes the choice back. A session that has started keeps its own.
  void setCacheLifetime(ChatSession session, CacheLifetime? lifetime) {
    if (!session.isDraft) return;
    session.cacheLifetime = lifetime;
    _keepDraft(session);
    _notify();
  }

  /// Sets the account [session] will run on, by its name in the host's `[host.accounts]`; null takes the choice back, which runs
  /// the source's first. A session that has started keeps its own.
  void setAccount(ChatSession session, String? account) {
    if (!session.isDraft) return;
    session.account = account;
    _keepDraft(session);
    // What an account may use is what its own login lists: the choices are asked again as the one chosen.
    _askChoices(session.source, account: account);
    _notify();
  }

  /// Gives [session], a draft, the profile [name], whose values are [preset]: its source (when [source] is the one it names), its
  /// model, effort and mode, its cache lifetime and its loadout (when [loadoutHeld]). What the profile sets replaces what the
  /// person chose for the same thing; what it leaves unset stays as it was. A model, effort or mode the source has not yet
  /// listed is chosen when it does, and one it does not offer is said in [ChatSession.presetUnsupported]. A null [name] only
  /// takes the profile's name off, and every value stays.
  void usePreset(
    ChatSession session,
    String? name,
    Preset? preset, {
    AgentSource? source,
    bool loadoutHeld = true,
  }) {
    if (!session.isDraft) return;
    session
      ..presetWants.clear()
      ..presetUnsupported.clear();
    if (name == null || preset == null) {
      session.preset = null;
      _keepDraft(session);
      _notify();
      return;
    }
    session.preset = name;
    if (preset.source != null) {
      if (source == null) {
        session.presetUnsupported['source'] = 'no source is called ${preset.source}';
      } else if (source != session.source) {
        updateDraft(session, source: source);
      }
    }
    for (final (key, role, value) in [
      ('model', OptionRole.model, preset.model),
      ('effort', OptionRole.effort, preset.effort),
      ('mode', OptionRole.mode, preset.mode),
    ]) {
      if (value == null) continue;
      session.chat.dropChoices(where: (choice) => choice.role == role);
      session.presetWants[key] = value;
    }
    if (preset.mode != null) session.chat.presetBypass(false);
    if (CacheLifetime.named(preset.cacheLifetime) case final lifetime?) session.cacheLifetime = lifetime;
    if (preset.loadout != null && loadoutHeld) {
      session
        ..loadout = preset.loadout
        ..overlays = null;
    }
    _settlePreset(session);
    _keepDraft(session);
    _notify();
  }

  /// Gives [session], a new draft that names no profile, what the default profile sets: the model, effort and mode it wants, its
  /// cache lifetime and its loadout. The draft keeps no profile name, so it reads as the default one.
  void _wantDefaultPreset(ChatSession session) {
    final preset = defaultPresetOf?.call();
    if (preset == null) return;
    for (final (key, value) in [('model', preset.model), ('effort', preset.effort), ('mode', preset.mode)]) {
      if (value != null) session.presetWants[key] = value;
    }
    if (CacheLifetime.named(preset.cacheLifetime) case final lifetime? when session.cacheLifetime == null) {
      session.cacheLifetime = lifetime;
    }
    if (preset.loadout != null && session.loadout == null) session.loadout = preset.loadout;
  }

  /// Chooses, for every draft, the values its profile asked for that its source has now listed.
  void _settlePresets() {
    for (final session in _sessions) {
      if (session.isDraft && session.presetWants.isNotEmpty) _settlePreset(session);
    }
  }

  /// Chooses the model, effort and mode [session]'s profile asks for where the source offers them. The model goes first, since
  /// the efforts a source lists follow it. A value the source has listed and does not offer is unsupported; before it lists
  /// anything the value waits.
  void _settlePreset(ChatSession session) {
    final source = session.source;
    for (final (key, role) in [
      ('model', OptionRole.model),
      ('effort', OptionRole.effort),
      ('mode', OptionRole.mode),
    ]) {
      final value = session.presetWants[key];
      if (value == null) continue;
      final options = optionsOf(session);
      final listed =
          source != null &&
          !isAskingChoices(source) &&
          (options.isNotEmpty || choicesOf(source) != null);
      if (role == OptionRole.mode) {
        final entry = modeEntries(modeOptionIn(options)).where((each) => each.value == value).firstOrNull;
        if (entry != null) {
          _chooseDraftMode(session, entry);
        } else if (listed) {
          session.presetUnsupported[key] = 'the agent offers no mode $value';
        } else {
          continue;
        }
      } else {
        final option = optionIn(options, role);
        if (option != null && option.choices.any((choice) => choice.value == value)) {
          session.chat.holdChoice(PendingChoice(option.id, role, value, byPerson: true));
          if (role == OptionRole.model) _dropUnofferedEffort(session);
        } else if (listed) {
          session.presetUnsupported[key] = option == null
              ? 'the agent offers no $key to choose'
              : 'the agent offers no $key $value';
        } else {
          continue;
        }
      }
      session.presetWants.remove(key);
    }
  }

  /// The values of [preset] that [session], a draft, no longer holds, by key (`source`, `model`, `effort`, `mode`,
  /// `cache_lifetime`, `loadout`): empty while it is as the profile makes it. Only what the profile sets is compared, and a value
  /// still waiting for its source's list, or one the agent cannot take, is not a change the person made.
  List<String> presetChanges(ChatSession session, Preset preset) {
    String? chosen(OptionRole role) => session.pending.values.where((choice) => choice.role == role).firstOrNull?.value;
    bool differs(String key, String? asked, String? held) =>
        asked != null &&
        !session.presetWants.containsKey(key) &&
        !session.presetUnsupported.containsKey(key) &&
        asked != held;
    return [
      if (differs('source', preset.source, session.source?.label)) 'source',
      if (differs('model', preset.model, chosen(OptionRole.model))) 'model',
      if (differs('effort', preset.effort, chosen(OptionRole.effort))) 'effort',
      if (differs('mode', preset.mode, chosen(OptionRole.mode))) 'mode',
      if (differs('cache_lifetime', preset.cacheLifetime, session.cacheLifetime?.id)) 'cache_lifetime',
      if (differs('loadout', preset.loadout, session.loadout)) 'loadout',
    ];
  }

  /// What [session], a draft, holds now of what a profile sets: the values [Preset] names, as a profile of its own.
  Preset presetOf(ChatSession session, {String? context}) {
    String? chosen(OptionRole role) => session.pending.values.where((choice) => choice.role == role).firstOrNull?.value;
    return Preset(
      source: session.source?.label,
      model: chosen(OptionRole.model),
      effort: chosen(OptionRole.effort),
      mode: chosen(OptionRole.mode),
      context: context,
      cacheLifetime: session.cacheLifetime?.id,
      loadout: session.loadout,
    );
  }

  /// Changes what a draft will start with. Once it has started, its source, folder, worktree and overlays stay as they were.
  void updateDraft(
    ChatSession session, {
    AgentSource? source,
    String? cwd,
    bool? worktree,
    Set<String>? overlays,
    bool allOverlays = false,
    String? loadout,
    bool defaultLoadout = false,
    String? text,
    SessionKind? kind,
    String? host,
    bool localHost = false,
  }) {
    if (text != null) session.draftText = text;
    if (session.status == SessionStatus.draft) {
      // A workspace is (host, path): another host holds none of this one's folders, nor a worktree of them.
      if (localHost ? session.host != null : host != null && host != session.host) {
        session
          ..host = localHost ? null : host
          ..wantsWorktree = false
          ..cwd = null
          ..account = null;
      }
      if (source != null && source != session.source) {
        // Choices for another source's options mean nothing to this one's. They go first: the conversation tells its listeners as
        // each goes, and none should see a new source with the old one's choices.
        session.chat.dropChoices();
        session.chat.presetBypass(false);
        session.account = null;
        session
          ..source = source
          ..keptSource = null
          ..agent = source.label;
        _askChoices(source);
      }
      // A folder chosen makes it a workspace session; a chat has no folder of the person's, which a worktree needs.
      if (kind != null) session.kind = kind;
      if (cwd != null) {
        session.cwd = cwd;
        session.kind = SessionKind.workspace;
      }
      if (session.kind == SessionKind.chat) {
        session.cwd = null;
        session.wantsWorktree = false;
      }
      if (worktree != null) session.wantsWorktree = worktree;
      if (loadout != null || defaultLoadout) {
        // A loadout brings its own overlays: the switches made on another one mean nothing to it.
        session
          ..loadout = defaultLoadout ? null : loadout
          ..overlays = null;
      }
      if (overlays != null || allOverlays) {
        session.overlays = allOverlays ? null : overlays;
      }
    }
    _keepDraft(session);
    _notify();
  }

  /// Every draft kept from an earlier run, as sessions; [source] finds each one's source again by its kind and label.
  Future<void> restoreDrafts(AgentSource? Function(String label) source) async {
    final kept = await drafts?.load() ?? const <KeptDraft>[];
    for (final draft in kept) {
      final record = draft.record;
      final from = record['source'] is Map ? record['source'] as Map : const {};
      final kept = from['label'] is String ? from['label'] as String : null;
      createDraft(
        id: draft.id,
        source: kept == null ? null : source(kept),
        keptSource: kept,
        cwd: record['cwd'] is String ? record['cwd'] as String : null,
        kind: record['kind'] == null ? null : SessionKind.named(record['kind']),
        worktree: record['worktree'] == true,
        cacheLifetime: CacheLifetime.named(record['cacheLifetime']),
        account: record['account'] is String
            ? record['account'] as String
            : null,
        overlays: record['overlays'] is List
            ? {
                for (final name in record['overlays'] as List)
                  if (name is String) name,
              }
            : null,
        loadout: record['loadout'] is String
            ? record['loadout'] as String
            : null,
        host: record['host'] is String ? record['host'] as String : null,
        preset: record['preset'] is String ? record['preset'] as String : null,
        presetWants: record['presetWants'] is Map
            ? {
                for (final MapEntry(:key, :value) in (record['presetWants'] as Map).entries)
                  if (key is String && value is String) key: value,
              }
            : const {},
        text: record['text'] is String ? record['text'] as String : '',
        title: record['title'] is String ? record['title'] as String : null,
        bypass: record['bypass'] == true,
        workspaceId: record['workspaceId'] is String
            ? record['workspaceId'] as String
            : null,
        created: DateTime.tryParse('${record['created']}'),
        pending: {
          for (final choice in [
            for (final json
                in record['pending'] is List
                    ? record['pending'] as List
                    : const [])
              ?PendingChoice.fromJson(json),
          ])
            choice.id: choice,
        },
        attachments: draft.attachments,
      );
    }
  }

  /// Gives each draft restored without its source that source, now that [source] finds it: a hosted source exists only once
  /// the host serves it.
  void resolveKeptSources(AgentSource? Function(String label) source) {
    var changed = false;
    for (final session in sessions) {
      final kept = session.keptSource;
      if (!session.isDraft || session.source != null || kept == null) continue;
      final found = source(kept);
      if (found == null) continue;
      session
        ..source = found
        ..keptSource = null
        ..agent = found.label;
      changed = true;
    }
    if (changed) _notify();
  }

  void _keepDraft(ChatSession session) {
    final id = session.draftId;
    if (drafts == null || id == null || !session.isDraft) return;
    drafts!.save(id, {
      'created': session.created.toUtc().toIso8601String(),
      'source': ?switch ((session.source, session.keptSource)) {
        (final source?, _) => {'label': source.label},
        (null, final kept?) => {'label': kept},
        _ => null,
      },
      'cwd': ?session.cwd,
      'workspaceId': ?session.workspaceId,
      'kind': session.kind.name,
      'worktree': session.wantsWorktree,
      'cacheLifetime': ?session.cacheLifetime?.id,
      'account': ?session.account,
      'overlays': ?session.overlays?.toList(),
      'loadout': ?session.loadout,
      'host': ?session.host,
      'preset': ?session.preset,
      if (session.presetWants.isNotEmpty) 'presetWants': session.presetWants,
      'text': session.sending?.text ?? session.draftText,
      if (!session.agentNames) 'title': session.title,
      'pending': [for (final choice in session.pending.values) choice.toJson()],
      if (session.bypass) 'bypass': true,
    }, session.sending?.attachments ?? session.attachments);
  }

  /// What a draft's composer holds, kept with it.
  void keepDraftText(ChatSession session, String text) {
    if (!session.isDraft || session.draftText == text) return;
    session.draftText = text;
    _keepDraft(session);
  }

  /// Binds [client], which [opened] a session in [cwd] for [draft], as [attachVerifiedClient] does: from here the session is
  /// started and the ledger notes it. Throws when the draft was closed meanwhile, and [client] is then the caller's to end.
  void bindDraft(
    ChatSession draft,
    VerifiedAgent client,
    AcpOpenedSession opened, {
    required String cwd,
    SessionWorktree? worktree,
    SessionPlace place = SessionPlace.local,
    Uri? endpoint,
    Uri? websocketFallback,
    String? method,
  }) {
    if (!_sessions.contains(draft) || !draft.isDraft) {
      throw StateError('The session was closed before it started');
    }
    draft
      ..peerId = client.peerId
      ..remoteSessionId = opened.id
      ..cwd = cwd
      ..worktree = worktree
      ..place = place
      ..endpoint = endpoint
      ..websocketFallback = websocketFallback
      ..method = method
      ..ranOnAccount = client.identity.ranOnAccount
      ..ranOnPool = client.identity.ranOnPool;
    applyOpened(draft, opened);
    attachVerifiedClient(draft.id, client);
    final kept = draft.draftId;
    draft.draftId = null;
    if (kept != null) unawaited(drafts?.remove(kept));
    _rememberSource(draft);
  }

  /// Notes what [session]'s agent offers and takes, for its source's next draft.
  void _rememberSource(ChatSession session) {
    final source = session.source;
    final client = _clients[session.id];
    if (source == null || client == null) return;
    ledger?.rememberSource(
      sourceKey(source),
      options: session.options,
      images: client.takesImages,
      files: client.takesFiles,
    );
  }

  /// What the agent of [session] is known to take with a prompt: its own word once it runs, else what its source's agent said
  /// last time; null when neither is known.
  PromptTakes? takesOf(ChatSession session) {
    final client = _clients[session.id];
    // A link names a path on this computer, which only an agent running here can read.
    final here =
        session.place == SessionPlace.local ||
        session.place == SessionPlace.host;
    if (client != null && !client.isClosed) {
      return PromptTakes(
        images: client.takesImages,
        files: client.takesFiles,
        links: here,
      );
    }
    final memory = session.source == null
        ? null
        : ledger?.source(sourceKey(session.source!));
    final images = memory?.images, files = memory?.files;
    return images == null || files == null
        ? null
        : PromptTakes(images: images, files: files);
  }

  /// The choices shown for [session]: the agent's own once it runs. Before, the models and efforts its source's provider
  /// lists (each model's own efforts, following the model chosen), then what the agent offered last time, and for a role
  /// neither has, the modes the app knows the source's agent offers.
  List<ConfigOption> optionsOf(ChatSession session) {
    if (!session.isDraft) return session.options;
    final source = session.source;
    if (source == null) return const [];
    final model = session.pending.values
        .where((choice) => choice.role == OptionRole.model)
        .firstOrNull
        ?.value;
    final listed =
        choicesOf(source)?.options(model: model) ?? const <ConfigOption>[];
    final remembered =
        ledger?.source(sourceKey(source))?.options ?? const <ConfigOption>[];
    return overlaid(overlaid(listed, remembered), knownModes(source));
  }

  /// When [session]'s source's choices were last seen, for a draft showing them; null otherwise, and when they are only what
  /// the app knows.
  DateTime? optionsSeen(ChatSession session) =>
      session.isDraft &&
          session.source != null &&
          (ledger?.source(sourceKey(session.source!))?.options.isNotEmpty ??
              false)
      ? ledger?.source(sourceKey(session.source!))?.seen
      : null;

  /// Puts a person's choice for a draft as its source's default: the profile's [hints] by role where it gives one, else the
  /// last choice made for that source. Choices already made in the draft stay.
  void defaultChoices(
    ChatSession session, {
    Map<OptionRole, String> hints = const {},
  }) {
    if (!session.isDraft) return;
    final source = session.source;
    final memory = source == null ? null : ledger?.source(sourceKey(source));
    for (final role in const [
      OptionRole.mode,
      OptionRole.model,
      OptionRole.effort,
    ]) {
      if (session.pending.values.any((choice) => choice.role == role) ||
          (role == OptionRole.mode && session.bypass)) {
        continue;
      }
      final value =
          hints[role] ??
          (role == OptionRole.mode ? defaultMode : null) ??
          memory?.chosen[role.name];
      if (value == null) continue;
      if (role == OptionRole.mode) {
        _startInMode(session, value);
        continue;
      }
      final option = optionIn(optionsOf(session), role);
      final id = option?.id ?? role.name;
      session.chat.holdChoice(PendingChoice(id, role, value));
    }
    _keepDraft(session);
    _notify();
  }

  /// Bypass chosen before the agent was known is the agent's own, once it turns out to have one that asks for nothing.
  void _resolveBypass(ChatSession session) {
    if (!session.bypass || !session.bypassBeforeAgent || session.isDraft) {
      return;
    }
    session.bypassBeforeAgent = false;
    final option = modeOptionIn(session.options);
    final native = modeEntries(
      option,
    ).where((entry) => entry.bypass && !entry.own).firstOrNull;
    if (option != null && native != null) {
      session.chat.presetBypass(false);
      session.chat.holdChoice(
        PendingChoice(option.id, OptionRole.mode, native.value),
      );
    }
  }

  /// Puts a person's message to a draft: its agent starts, then takes the message. Until it has started, the message stays the
  /// person's to send again; a start that fails says why in the conversation and leaves the draft as it was.
  Future<void> _startThenDeliver(ChatSession session, String text) async {
    final starter = this.starter;
    if (starter == null) {
      throw StateError('This app cannot start an agent here');
    }
    if (session.status != SessionStatus.draft) {
      throw StateError('It is starting already');
    }
    if (session.source == null) {
      throw StateError('Choose where its agent comes from first');
    }
    session.status = SessionStatus.connecting;
    _starting.add(session.id);
    // The conversation shows the message at once; the composer's attachments go with it, and come back if it cannot start.
    final taken = List.of(session.attachments);
    session.chat.holdFirst(
      text,
      attachments: [for (final item in taken) item.name],
    );
    session.attachments.clear();
    session.sending = (text: text, attachments: taken);
    _keepDraft(session);
    final named =
        session.agentNames &&
        session.title == SessionStore.draftTitle &&
        text.trim().isNotEmpty;
    if (named) {
      session
        ..title = titleFrom(text)
        ..titleFromMessage = true;
    }
    _notify();
    try {
      try {
        await timed('start', () => starter(session));
      } catch (error) {
        if (session.remoteSessionId == null) {
          session.status = SessionStatus.draft;
          session.chat.releaseFirst();
          session.attachments.addAll(taken);
          if (session.draftText.isEmpty) session.draftText = text;
          if (named) {
            session
              ..title = SessionStore.draftTitle
              ..titleFromMessage = false;
          }
          debugPrint(
            '${Brand.name}: ${session.source?.label ?? 'an agent'} was not started: ${reasonOf(error)}',
          );
          if (_sessions.contains(session)) {
            session.chat.note('Not started: ${reasonOf(error)}');
          }
        }
        // A tab closed while it connected has nobody to tell.
        if (!_sessions.contains(session)) return;
        rethrow;
      }
      if (!_sessions.contains(session)) return;
      if (_clients[session.id] == null) {
        throw StateError('The agent did not start');
      }
      await timed('held_choices', () => applyHeldChoices(session));
      try {
        await _send(
          session,
          text,
          taken,
          onSent: () {
            // Sent: what follows is queued or sent as usual, even while a v1 agent's first turn runs.
            _starting.remove(session.id);
            session.firstSent = text;
          },
        );
      } catch (_) {
        // The agent runs, but never took the message: it goes back to the composer with what came with it.
        session.chat.releaseFirst();
        for (final item in taken) {
          if (!session.attachments.contains(item)) {
            session.attachments.add(item);
          }
        }
        rethrow;
      }
      session.chat.releaseFirst();
    } finally {
      session.sending = null;
      _keepDraft(session);
      _starting.remove(session.id);
      _notify();
    }
  }

  /// Whether [session]'s first message is still on its way, during which nothing else is sent.
  bool isStarting(ChatSession session) => _starting.contains(session.id);

  /// The conversation the client [from] serves under the agent's session id [remote], if one is here.
  ChatSession? _sessionOf(String remote, VerifiedAgent from) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.remoteSessionId == remote &&
          identical(_clients[session.id], from)) {
        target = session;
      }
    }
    return target;
  }
}
