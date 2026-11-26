import 'dart:async';
import 'acp/chat_connection.dart';
import 'acp/client.dart';
import 'acp/connect.dart' show plainOnLoopback;
import 'acp/delegate.dart';
import 'acp/delegate_targets.dart';
import 'acp/local_agent.dart' show readLocalFile;
import 'chat/timing_log.dart';
import 'config/canoa_config.dart' show Preset;
import 'acp/secret_wire.dart' show SecretIdentity;
import 'package:coqui_acp/secret.dart';
import 'usage/limit_rules.dart' show LimitsLink;
import 'usage/limits_group.dart' show LimitStop;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/coqui_acp_chat.dart'
    show
        AttentionKind,
        ChatStatus,
        PermissionQuestion,
        Settlement,
        TurnEvent,
        TurnOutcome;
import 'package:coqui_transport_web/coqui_transport_web.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'chat/agent_events.dart';
import 'chat/attachments.dart';
import 'chat/cache_lifetime.dart';
import 'chat/context_kinds.dart';
import 'chat/image_fit.dart';
import 'client/relation.dart';
import 'chat/git_changes.dart';
import 'chat/git_publish.dart';
import 'chat/modes.dart';
import 'chat/session_options.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart'
    show SecretCardEnvironment, shortLine;
import 'drafts/draft_store.dart';
import 'accounts/account_models.dart' show AccountMove, AccountOffer, LimitHint, TurnAdvice, TurnRefused, machineLogin;
import 'history/session_move.dart' show MoveReport, lossText;
import 'ledger/session_ledger.dart';
import 'permissions/permission_rules.dart';
import 'profile/report.dart';
import 'sources/source_choices.dart';
import 'sources/source_options.dart';
import 'sources/sources.dart';
import 'surfaces/a2ui/limits.dart';
import 'surfaces/a2ui/messages.dart' show a2uiCarried;
import 'surfaces/a2ui/overlay_views.dart';
import 'surfaces/a2ui/views.dart';
import 'voice/front_matter.dart';
import 'workspace/site_meta.dart' show siteSetup;
import 'workspace/workspace_template.dart' show newSessionUuid;
import 'worktree/worktree.dart';

export 'acp/delegate.dart' show DelegateTask, DelegateTasks, TaskPhase;
export 'acp/mcp_list.dart' show McpInventory, McpResource, McpServerStatus, McpTool;
export 'acp/secret_wire.dart' show SecretIdentity;
export 'chat/agent_events.dart' show MessageRole;
export 'chat/attachments.dart'
    show
        Attachment,
        AttachmentKind,
        imageTypeOf,
        maxAttachedImage,
        maxAttachedText,
        maxAttachments;
export 'ledger/session_ledger.dart' show SessionKind, SessionPlace;
export 'package:coqui_acp_chat/coqui_acp_chat.dart'
    show
        AgentQuestion,
        AttentionKind,
        ChatMessage,
        PermissionQuestion,
        QueueDelivery,
        QueuedMessage;


part 'session_store/session_models.dart';
part 'session_store/session_drafts.dart';
part 'session_store/session_approvals.dart';
part 'session_store/session_updates.dart';
part 'session_store/session_switching.dart';
part 'session_store/session_sending.dart';
part 'session_store/session_history.dart';
part 'session_store/session_a2ui.dart';
part 'session_store/session_limits.dart';

/// Shared UI state for desktop, PWA and packaged Chromium extension: Canoa's conversations, each a `coqui_acp_chat` conversation
/// with what is Canoa's own beside it. Transport, verified peers and persistence are intentionally outside it.
class SessionStore extends ChangeNotifier implements AgentSecrets {
  SessionStore({
    this.recorder,
    this.ledger,
    this.rules,
    this.defaultMode,
    this._bypassOnceOnly = false,
    this.drafts,
    this.embedLimit = maxAttachedText,
    Future<List<int>> Function(String path, {int? maxBytes})? readFile,
  }) : readFile = readFile ?? readLocalFile;

  /// Told the kind and shape of each update this store applies, for a test or the `CANOA_EVENT_LOG` setting.
  final UpdateRecorder? recorder;

  /// What the time is, for the lengths the conversation shows; a test gives its own.
  DateTime Function() clock = DateTime.now;

  /// Where every session opened or resumed here is noted, to be listed and opened again after the app ends.
  final SessionLedger? ledger;

  /// What the person told Canoa to allow without asking, per source and folder; null where nothing is kept.
  final PermissionRules? rules;

  /// Where drafts are kept between runs, with what is written in them and what goes with them.
  final DraftKeeper? drafts;

  /// Starts a draft's agent on its first message; set by the app. Without it a draft cannot start.
  DraftStarter? starter;

  /// The default profile as canoa.cfg has it now: what a new draft that was given no profile wants, as if it had been given it.
  /// A draft keeps what it starts with, so a later edit of the profile reaches new drafts only.
  Preset? Function()? defaultPresetOf;

  /// Whether the files of a session's folder are the ones the host the app asks about holds, so that links and repositories are
  /// asked of it and a rule for a path can be made; set by the app. Without it only a session on this computer in an app with
  /// files of its own can.
  bool Function(ChatSession session)? pathsOnHost;

  /// Reads the repository in a session's folder through the host its agent runs on; set by the app. Null where none can.
  GitReader? Function(ChatSession session)? gitReader;

  /// What commits, pushes and opens pull requests in [session]'s repository, when the host its agent runs on does for the person.
  GitPublisher? gitPublisherOf(ChatSession session) {
    final reader = gitReader?.call(session);
    return reader is GitPublisher ? reader as GitPublisher : null;
  }

  /// The folder a session's repository is read from: its worktree, else its folder, once it has started.
  String? gitFolderOf(ChatSession session) => _gitFolder(session);

  /// The folder a session's changes are read from: its worktree, else its folder, once it has started.
  String? _gitFolder(ChatSession session) =>
      session.isDraft ? null : (session.worktree?.path ?? session.cwd);

  /// Reads [session]'s repository again, for the changes the inspector shows.
  Future<void> refreshGit(ChatSession session) async {
    final folder = _gitFolder(session), reader = gitReader?.call(session);
    if (folder == null || reader == null) return;
    if (await session.git.refresh(reader, folder)) notifyListeners();
  }

  /// A turn of [session] begins: the disk as it is now is what the turn is measured from. Never delays the message.
  void _gitBeforeTurn(ChatSession session) {
    final folder = _gitFolder(session), reader = gitReader?.call(session);
    if (folder == null || reader == null) return;
    unawaited(
      session.git.beginTurn(reader, folder).then((_) => notifyListeners()),
    );
  }

  /// Asks the host for a source's models and efforts before its agent runs; set by the app. Without it nothing is asked.
  ChoicesLoader? choicesLoader;

  /// Whether the account [session] was started on names a pool of its agent; set by the app. Without it no session is on one.
  bool Function(ChatSession session)? runsOnPool;

  /// Asks the host which account the next turn of [session] runs on; set by the app. It throws [TurnRefused] when no account of a
  /// pool has usage left, and anything else when the host cannot be asked.
  Future<TurnAdvice> Function(
    ChatSession session, {
    required String account,
    String? model,
    String? current,
    int? idleSeconds,
    required String lifetime,
    required List<LimitHint> limited,
  })?
  poolTurn;

  /// Moves [session] to [account], as the picker does; set by the app. With [pool] the session goes on naming the pool and runs on
  /// [account], and the line it leaves in the conversation says why.
  Future<bool> Function(
    ChatSession session,
    String account, {
    ({String pool, String why})? pool,
  })?
  moveAccount;

  /// Asks for a source's choices as one of its accounts, so that a signed-in account lists what it may use; set by the app. When
  /// it is not set or says nothing, [choicesLoader] asks as the machine's own login.
  AccountChoicesLoader? accountChoicesLoader;

  /// The account to ask as while a draft has chosen none: the source's first usable one; set by the app.
  String? Function(AgentSource source)? defaultAccountOf;

  /// What each source's provider listed, by [sourceKey], as last asked.
  final Map<String, SourceChoices> _choices = {};
  final Map<String, String?> _askedAs = {};
  final Map<String, int> _asks = {};
  final Set<String> _asking = {};
  bool _disposed = false;

  /// What [source]'s provider listed before a session starts; null while it has not said.
  SourceChoices? choicesOf(AgentSource source) => _choices[sourceKey(source)];

  /// Whether the host is being asked for [source]'s choices now: the composer says it is listing them.
  bool isAskingChoices(AgentSource source) =>
      _asking.contains(sourceKey(source));

  Future<SourceChoices?> _loadChoices(
    AgentSource source,
    String? account,
  ) async {
    if (account != null && accountChoicesLoader != null) {
      final got = await accountChoicesLoader!(source, account);
      if (got != null) return got;
    }
    return choicesLoader?.call(source);
  }

  /// Asks for [source]'s choices as [account] (the source's default when null) unless they are held for that account and
  /// recent, or being asked now. A source whose provider cannot say, or a host that cannot be asked, leaves the composer
  /// saying why, or that its choices come after the agent starts.
  void _askChoices(AgentSource? source, {String? account}) {
    if (choicesLoader == null || source == null) return;
    final key = sourceKey(source);
    final as = account ?? defaultAccountOf?.call(source);
    final held = _choices[key];
    final same = _askedAs.containsKey(key) && _askedAs[key] == as;
    if (same && _asking.contains(key)) return;
    if (same &&
        held != null &&
        DateTime.now().difference(held.fetchedAt) < choicesLife) {
      return;
    }
    _askedAs[key] = as;
    final mine = _asks[key] = (_asks[key] ?? 0) + 1;
    _asking.add(key);
    unawaited(() async {
      try {
        final got = await _loadChoices(source, as);
        if (got != null && !_disposed && _asks[key] == mine) {
          _choices[key] = got.at(DateTime.now());
          _dropUnlistedModels(key, as, got);
          _settlePresets();
        }
      } catch (error) {
        debugPrint(
          'Canoa: choices of ${source.label} could not be asked: ${reasonOf(error)}',
        );
      } finally {
        if (_asks[key] == mine) _asking.remove(key);
        if (!_disposed) notifyListeners();
      }
    }());
  }

  /// A draft that runs on [account] and holds a model its choices do not list is taken back to the agent's own: an account lists
  /// only what it may use, so a model chosen while another account was chosen is not one this account can start on.
  void _dropUnlistedModels(String key, String? account, SourceChoices listing) {
    if (!listing.known || listing.models.isEmpty) return;
    final listed = {for (final model in listing.models) model.id};
    for (final session in _sessions) {
      final source = session.source;
      if (!session.isDraft || source == null || sourceKey(source) != key) continue;
      if ((session.account ?? defaultAccountOf?.call(source)) != account) continue;
      session.chat.dropChoices(where: (choice) => choice.role == OptionRole.model && !listed.contains(choice.value));
      _dropUnofferedEffort(session);
    }
  }

  /// Asks the host for [source]'s choices again now, whatever is held, and tells whoever listens; what it answers, or what was
  /// held when the host cannot be asked.
  Future<SourceChoices?> askChoicesAgain(AgentSource source) async {
    if (choicesLoader == null) return choicesOf(source);
    final key = sourceKey(source);
    try {
      final as = _askedAs.containsKey(key)
          ? _askedAs[key]
          : defaultAccountOf?.call(source);
      _askedAs[key] = as;
      final got = await _loadChoices(source, as);
      if (got != null && !_disposed) {
        _choices[key] = got.at(DateTime.now());
        notifyListeners();
      }
    } catch (error) {
      debugPrint(
        'Canoa: choices of ${source.label} could not be asked: ${reasonOf(error)}',
      );
    }
    return choicesOf(source);
  }

  /// The largest file, in bytes, sent as the agent's embedded context; a larger one goes as a link to it.
  int embedLimit;

  /// Whether what was queued goes as one message when a turn ends, or one message a turn (canoa.cfg's `[sessions] queue_delivery`).
  model.QueueDelivery get queueDelivery => _queueDelivery;
  model.QueueDelivery _queueDelivery = model.QueueDelivery.merge;
  set queueDelivery(model.QueueDelivery value) {
    if (value == _queueDelivery) return;
    _queueDelivery = value;
    for (final session in _sessions) {
      session.chat.queueDelivery = value;
    }
    notifyListeners();
  }

  /// A file's bytes, read when it is sent.
  final Future<List<int>> Function(String path, {int? maxBytes}) readFile;

  /// The mode a new session starts in, as an agent names it (one of [modeOrder] for the translators Canoa runs); null leaves
  /// each agent's own. It wins over the person's last choice for the source, and a profile's mode wins over it.
  String? defaultMode;

  bool _bypassOnceOnly;

  /// Whether Bypass answers only with an answer that allows once (`[permissions] allow_once_only`).
  bool get bypassOnceOnly => _bypassOnceOnly;
  set bypassOnceOnly(bool value) {
    _bypassOnceOnly = value;
    for (final session in _sessions) {
      session.chat.bypassOnceOnly = value;
    }
  }

  final _attention = StreamController<(ChatSession, AttentionKind)>.broadcast();

  /// Each time a session comes to need the person: a request or a question put to them, or a turn ended. Whether that reaches
  /// them outside the window is the app's to decide.
  Stream<(ChatSession, AttentionKind)> get attention => _attention.stream;

  /// How many sessions wait for the person's answer.
  int get waiting => _sessions.where((session) => session.needsYou).length;

  void _needs(ChatSession session, AttentionKind kind) {
    if (kind == AttentionKind.finished && session.id != _selectedId) {
      session.unread = true;
    }
    if (!_attention.isClosed) _attention.add((session, kind));
  }

  /// The mode [session] is in as shown: Canoa's own Bypass, else the agent's mode option, a choice still waiting first. Null
  /// before an agent that has not said.
  String? modeOf(ChatSession session) => session.chat.mode;

  /// The agent's own mode, which Canoa's Bypass sits on top of: what the agent says it is in now, never a choice waiting.
  String? underlyingMode(ChatSession session) => session.chat.underlyingMode;

  /// Whether [session] is in a mode that asks for nothing, to show it: Canoa's own Bypass, or the agent's own as it says it
  /// is in it now. A choice still waiting is not yet in effect.
  bool inBypass(ChatSession session) => session.chat.inBypass;

  /// Whether [session]'s mode can be changed now: not while it connects, nor while it is opened again and not yet loaded.
  bool canChangeMode(ChatSession session) =>
      session.status != SessionStatus.connecting &&
      session.status != SessionStatus.preview &&
      !session.opening;

  /// Puts [session] in [entry]'s mode. Canoa's own Bypass answers every request that offers an allow, those already waiting
  /// included, and sits on the agent's mode, which stays; a request with no allow, a question or a plan is still the person's.
  Future<void> chooseMode(ChatSession session, ModeEntry entry) async {
    if (session.isDraft) {
      _chooseDraftMode(session, entry);
      return;
    }
    await session.chat.chooseMode(entry);
    if (entry.own) session.bypassBeforeAgent = false;
    _keepDraft(session);
  }

  /// A draft has no agent to ask, so what is chosen waits for it, in the options its source is known to offer.
  void _chooseDraftMode(ChatSession session, ModeEntry entry) {
    if (!entry.selectable) return;
    final chat = session.chat;
    if (entry.own) {
      chat.dropChoices(where: (choice) => choice.role == OptionRole.mode);
      chat.presetBypass(true);
      session.bypassBeforeAgent = true;
    } else {
      chat.presetBypass(false);
      final option = modeOptionIn(optionsOf(session));
      if (option != null) {
        chat.holdChoice(
          PendingChoice(
            option.id,
            OptionRole.mode,
            entry.value,
            byPerson: true,
          ),
        );
      }
    }
    _keepDraft(session);
  }

  /// Takes back a draft's choice of mode, Bypass included, so its agent's own default holds.
  void clearMode(ChatSession session) {
    session.chat.dropChoices(where: (choice) => choice.role == OptionRole.mode);
    session.chat.presetBypass(false);
    _keepDraft(session);
    notifyListeners();
  }

  /// Puts [session] in the next of its modes, as Shift+Tab does. Presses queue against the mode last asked for, not the one
  /// the agent still shows, and a mode the agent refused is skipped.
  Future<void> cycleMode(ChatSession session) async {
    if (!canChangeMode(session)) return;
    if (!session.isDraft) return session.chat.cycleMode();
    final next = nextMode(
      modeEntries(modeOptionIn(optionsOf(session))),
      modeOf(session),
      skip: session.refusedModes,
    );
    if (next != null) _chooseDraftMode(session, next);
  }

  /// Takes [session] out of Bypass. Canoa's own sat on the agent's mode, which holds again; an agent's own goes to Manual,
  /// or the first mode it has that asks.
  Future<void> leaveBypass(ChatSession session) async {
    if (!session.isDraft) {
      await session.chat.leaveBypass();
      return;
    }
    final chat = session.chat;
    if (chat.bypass) {
      chat.presetBypass(false);
      chat.note('Bypass is off. Requests are asked again.');
      _keepDraft(session);
      return;
    }
    final entries = [
      for (final entry in modeEntries(modeOptionIn(optionsOf(session))))
        if (!entry.bypass && entry.selectable) entry,
    ];
    if (entries.isEmpty) return;
    _chooseDraftMode(
      session,
      entries
              .where(
                (entry) => const {'manual', 'default'}.contains(entry.value),
              )
              .firstOrNull ??
          entries.first,
    );
  }

  /// Puts a draft in the mode [value] names, which is Canoa's own Bypass where the agent has none that asks for nothing.
  void _startInMode(ChatSession session, String? value) {
    if (value == null) return;
    final option = modeOptionIn(optionsOf(session));
    final entries = modeEntries(option);
    final entry =
        entries.where((entry) => entry.value == value).firstOrNull ??
        (isBypassMode(value)
            ? entries.where((entry) => entry.bypass).firstOrNull
            : null);
    if (entry != null && entry.own) {
      session.chat.presetBypass(true);
      session.bypassBeforeAgent = true;
    } else {
      final id = option?.id ?? OptionRole.mode.name;
      session.chat.holdChoice(
        PendingChoice(id, OptionRole.mode, entry?.value ?? value),
      );
    }
  }

  /// A name an agent or a person gave, as it is shown and logged: one line, with nothing that could reorder what surrounds it.
  static String _named(String name) => shortLine(name) ?? 'an unnamed one';

  final List<ChatSession> _sessions = [];

  /// What an agent's A2UI views may cost; read when a session's first view arrives.
  A2uiLimits a2uiLimits = const A2uiLimits();
  final Map<String, A2uiSessionViews> _a2ui = {};

  /// Told when any session's views come, change or go, which is what the shell lists its view surfaces by.
  final ValueNotifier<int> a2uiChanged = ValueNotifier(0);
  final _a2uiOpened = StreamController<(ChatSession, A2uiView)>.broadcast(sync: true);

  /// A view an agent just drew, with the session it belongs to, for the app to open where its target says.
  Stream<(ChatSession, A2uiView)> get a2uiOpened => _a2uiOpened.stream;
  final Map<String, VerifiedAgent> _clients = {};
  final Map<String, Future<McpInventory?>> _mcpLoading = {};
  final Expando<VerifiedChatConnection> _connections = Expando();
  final Map<String, StreamSubscription<model.ChatEvent>> _eventSubscriptions =
      {};
  final Map<String, StreamSubscription<DelegateNotice>> _taskSubscriptions = {};
  final Map<String, StreamSubscription<AcpSessionUpdate>>
  _contextSubscriptions = {};
  final Map<String, List<StreamSubscription<Object?>>> _hubSubscriptions = {};
  int _nextId = 0;
  String? _selectedId;

  List<ChatSession> get sessions => List.unmodifiable(_sessions);

  /// The sessions the side list shows: every one that exists. A new session that has not been sent a first message is a page
  /// of choices, not a session, so it has no row until Send; one that is starting has its row from then.
  List<ChatSession> get listed => List.unmodifiable([
    for (final session in _sessions)
      if (session.status != SessionStatus.draft) session,
  ]);
  String? get selectedId => _selectedId;

  ChatSession? get selected {
    for (final session in _sessions) {
      if (session.id == _selectedId) return session;
    }
    return null;
  }

  ChatSession _newSession({
    required String title,
    required String agent,
    required SessionStatus status,
    String? peerId,
    String? remoteSessionId,
    Uri? endpoint,
    Uri? websocketFallback,
    String? cwd,
    SessionPlace place = SessionPlace.local,
    String? method,
    SessionWorktree? worktree,
    AgentSource? source,
    DateTime? created,
    bool agentNames = false,
  }) => ChatSession(
    id: 'local-${++_nextId}',
    title: title,
    agent: agent,
    status: status,
    peerId: peerId,
    remoteSessionId: remoteSessionId,
    endpoint: endpoint,
    websocketFallback: websocketFallback,
    cwd: cwd,
    place: place,
    method: method,
    worktree: worktree,
    source: source,
    created: created,
    agentNames: agentNames,
    recorder: recorder,
    clock: () => clock(),
  );


  void _notify() => notifyListeners();

  /// The title a draft has until its first message names it.
  static const draftTitle = 'New session';


  /// The host's limits, once the host grants them: a turn it refused is looked up here, and one turn let through.
  LimitsLink? limits;

  /// Opens the page of the limits, which the card of a refused turn offers.
  VoidCallback? onOpenLimits;

  /// The limit that refused the last turn of each session, by session id, until the next turn is sent.
  final Map<String, LimitStop> _limitStops = {};
  final Map<String, ({Object connection, List<StreamSubscription<Object?>> listening})> _refusals = {};

  /// What the sessions on each account last heard of its allowances, by the account's id as the host names it in a report.
  final Map<String, UsageReport> accountUsage = {};


  /// Sessions whose first message is on its way: started, or starting, and not yet delivered.
  final Set<String> _starting = {};


  /// This app's identity for secret prompts. Without one the host is offered nothing and every prompt is cancelled.
  SecretIdentity? secretIdentity;


  @override
  AcpSecretsCapability? get secretsCapability => secretIdentity?.offer;


  @override
  Future<AcpSecretRequestReply> askSecret(
    AcpSecretRequestParams request,
    VerifiedAgent from,
  ) async {
    final target = _secretTarget(request.context.sessionId, from);
    if (target == null) return const AcpSecretCancelled();
    // A key not bound to the host this connection proved is never drawn as a form: the host is told it was cancelled.
    try {
      await target.$3.verifier.collector(
        target.$2,
        AcpSecretMethod.request,
        request.context,
        request.recipient,
        DateTime.now().millisecondsSinceEpoch ~/ 1000,
      );
    } on AcpSecretException {
      return const AcpSecretCancelled();
    }
    final prompt = SecretRequestPrompt(request, target.$2, identity: target.$3);
    target.$1.pendingSecrets.add(prompt);
    _needs(target.$1, AttentionKind.question);
    notifyListeners();
    return prompt.reply.future;
  }


  @override
  Future<AcpSecretShowOpened> openSecretShow(
    AcpSecretShowOpen open,
    VerifiedAgent from,
  ) {
    final target = _secretTarget(open.context.sessionId, from);
    if (target == null) {
      return Future.value(const AcpSecretShowOpened.cancelled());
    }
    final prompt = SecretShowPrompt(open, target.$2, identity: target.$3);
    target.$1.pendingSecrets.add(prompt);
    _needs(target.$1, AttentionKind.question);
    notifyListeners();
    return prompt.ready.future;
  }


  @override
  Future<bool> deliverSecretShow(
    AcpSecretShowDeliver deliver,
    VerifiedAgent from,
  ) {
    final target = _secretTarget(deliver.sessionId, from);
    // A delivery nobody is waiting for, from another connection, or for a show the person has not agreed to look at, is refused.
    if (target != null) {
      for (final prompt in target.$1.pendingSecrets) {
        if (prompt is SecretShowPrompt &&
            prompt.ready.isCompleted &&
            prompt.deliver.value == null &&
            prompt.host == target.$2 &&
            prompt.open.context.requestId == deliver.requestId) {
          prompt.deliver.value = deliver;
          return prompt.done.future;
        }
      }
    }
    throw const AcpSecretException(
      AcpSecretFailure.malformed,
      'no show is waiting for this',
    );
  }


  @override
  void withdrawSecret(String sessionId, String requestId, VerifiedAgent from) {
    // Only the connection that carried the question can take it back.
    final session = _sessionOf(sessionId, from);
    if (session == null) return;
    final withdrawn = [
      for (final prompt in session.pendingSecrets)
        if (prompt is SecretRequestPrompt && prompt.request.context.requestId == requestId) prompt,
    ];
    if (withdrawn.isEmpty) return;
    for (final prompt in withdrawn) {
      session.pendingSecrets.remove(prompt);
      prompt.cancel();
    }
    notifyListeners();
  }


  /// What a rule calls [session]'s source: its identity (the key the session list remembers it under), not the name it is shown
  /// by, so two sources that share a label do not share rules.
  static String ruleSource(ChatSession session) =>
      session.source == null ? session.agent : sourceKey(session.source!);


  /// Whether [request]'s tool call id has the shape the host gives a task's: the task's id and a colon before the call's own.
  static bool _namesTask(AcpPermissionRequest request) =>
      request.toolCallId.indexOf(':') > 0;


  /// The task of [session] that [request] comes from, while it runs: the host puts the task's id and a colon before the tool call
  /// id of a task's request. Null for the conversation's own agent's.
  static DelegateTask? _taskOf(
    ChatSession session,
    AcpPermissionRequest request,
  ) {
    final id = request.toolCallId;
    final colon = id.indexOf(':');
    if (colon <= 0) return null;
    final task = session.tasks[id.substring(0, colon)];
    return task == null || task.phase.ended ? null : task;
  }


  /// What an "allow for the session" of a task's request covers: the kind, the title and what it would run or touch, so a task
  /// that titles every request alike is still asked about each different command or file.
  static String _allowanceKey(AcpPermissionRequest request) {
    final subject = subjectOf(
      kind: request.kind,
      rawInput: request.rawInput,
      toolCall: request.toolCall,
    );
    final what = switch (subject) {
      CommandSubject(:final toolName, :final command) =>
        'command\n$toolName\n$command',
      PathSubject(:final toolName, :final path) => 'path\n$toolName\n$path',
      McpSubject(:final name) => 'mcp\n$name',
      null => 'input\n${jsonEncode(request.rawInput)}',
    };
    return '${request.kind ?? ''}\n${request.title ?? ''}\n$what';
  }


  /// The `_meta` key under which `canoa` lists, in its answer to `session/new`, what a person should know about an agent
  /// before using it: what a command-line tool does without asking, and whether it remembers earlier messages.
  static const hostNoticesKey = 'dev.boriken.canoa/notices';


  /// What the host's answer comes to in words, for the line a move leaves in the conversation.
  static String _whyMoved(TurnAdvice advice, String? from) => advice.why == "exhausted"
      ? "${from ?? 'its account'} had run out${advice.left == null ? '' : ' (${advice.left})'}"
      : "the prompt cache had lapsed, so this turn goes to the next account in turn";


  /// The sessions being moved to another account now: what their agents report meanwhile is not about the account they end on.
  final Set<String> _moving = {};


  /// What the person and the agent have said in [session], which is what an agent's own copy of it holds.
  static List<({MessageRole role, String text})> _spoken(
    ChatSession session,
  ) => [
    for (final message in session.messages)
      if (message.role == MessageRole.user || message.role == MessageRole.agent)
        (role: message.role, text: message.text),
  ];


  /// The one line a switch leaves in the conversation: where it went, and what the move could not carry.
  static String _switchLine(String from, String to, MoveReport? report) {
    final head = 'Agent: $from → $to.';
    if (report == null) {
      return '$head $to had its own copy of this conversation, and nothing was said since it left, so that copy was picked up.';
    }
    final line = StringBuffer(
      '$head The conversation was exported from $from and imported into $to, which now has all of it.',
    );
    if (report.replayed) {
      line.write(
        ' $to keeps no session to seed, so what was said reaches it in your first message here.',
      );
    }
    if (report.compacted) {
      line.write(' Earlier turns were left out, as they did not fit.');
    }
    if (report.losses.isEmpty) {
      line.write(' Nothing was left out.');
    } else {
      line.write(
        ' Left out: ${report.losses.map((loss) => lossText(loss).replaceAll('\n', ' ')).join('; ')}.',
      );
    }
    return line.toString();
  }


  /// The session's options that take the profile's [values] for `model`, `effort` and `mode`, each by its id or category: as
  /// (option id, value, key). [options] are the session's, as (id, category, values, isMode).
  static List<(String id, String value, String key)> _choicesFor(
    Map<String, String> values,
    List<(String id, String? category, List<String> values, bool isMode)>
    options,
  ) {
    const ids = {
      'model': ['model'],
      'effort': ['effort', 'reasoning_effort', 'thought_level'],
      'mode': ['mode'],
    };
    const categories = {
      'model': 'model',
      'effort': 'thought_level',
      'mode': 'mode',
    };
    final out = <(String, String, String)>[];
    for (final MapEntry(key: key, value: value) in values.entries) {
      final option = options
          .where(
            (option) =>
                (option.$2 == categories[key] ||
                    ids[key]!.contains(option.$1) ||
                    (key == 'mode' && option.$4)) &&
                option.$3.contains(value),
          )
          .firstOrNull;
      if (option != null) out.add((option.$1, value, key));
    }
    return out;
  }


  static MessageRole _keptRole(String role) =>
      role == 'user' ? MessageRole.user : MessageRole.agent;


  /// Shows the thread of [session]'s task [taskId] in place of the conversation's own, which [closeChild] brings back. When the app heard
  /// nothing of the task, the host is asked for what it kept.
  void openChild(ChatSession session, String taskId) {
    final task = session.tasks[taskId];
    if (task == null) return;
    session.openChild = taskId;
    notifyListeners();
    loadChildHistory(session, taskId);
  }


  /// Asks the host for what it kept of [session]'s task [taskId], when the app heard nothing of it.
  void loadChildHistory(ChatSession session, String taskId) {
    final task = session.tasks[taskId];
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    if (task != null &&
        task.updates.isEmpty &&
        client != null &&
        remote != null) {
      unawaited(
        client.taskHistory(remote, taskId).then((kept) {
          if (kept != null && session.tasks.backfill(taskId, kept)) {
            notifyListeners();
          }
        }, onError: (Object _) {}),
      );
    }
  }


  /// Brings the conversation's own thread back from a task's.
  void closeChild(ChatSession session) {
    if (session.openChild == null) return;
    session.openChild = null;
    notifyListeners();
  }


  @override
  void dispose() {
    _disposed = true;
    for (final views in _a2ui.values) {
      views.dispose();
    }
    _a2ui.clear();
    a2uiChanged.dispose();
    unawaited(_a2uiOpened.close());
    for (final subscription in [
      ..._eventSubscriptions.values,
      ..._taskSubscriptions.values,
      ..._contextSubscriptions.values,
    ]) {
      unawaited(subscription.cancel());
    }
    for (final subscription in _hubSubscriptions.values.expand((each) => each)) {
      unawaited(subscription.cancel());
    }
    _eventSubscriptions.clear();
    _taskSubscriptions.clear();
    _contextSubscriptions.clear();
    _hubSubscriptions.clear();
    _clients.clear();
    for (final session in _sessions) {
      session.chat.onChange = null;
      session.chat.dispose();
    }
    unawaited(_attention.close());
    super.dispose();
  }
}
