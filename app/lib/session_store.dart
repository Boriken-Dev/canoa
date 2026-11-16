import 'dart:async';
import 'acp/chat_connection.dart';
import 'acp/client.dart';
import 'acp/delegate.dart';
import 'acp/local_agent.dart' show readLocalFile;
import 'chat/timing_log.dart';
import 'acp/secret_wire.dart' show SecretIdentity;
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show AttentionKind, ChatStatus, PermissionQuestion, Settlement, TurnEvent, TurnOutcome;
import 'package:coqui_web/coqui_web.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'chat/agent_events.dart';
import 'chat/attachments.dart';
import 'chat/cache_lifetime.dart';
import 'chat/image_fit.dart';
import 'chat/git_changes.dart';
import 'chat/modes.dart';
import 'chat/session_options.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretCardEnvironment, shortLine;
import 'drafts/draft_store.dart';
import 'accounts/account_models.dart' show AccountMove, machineLogin;
import 'ledger/session_ledger.dart';
import 'permissions/permission_rules.dart';
import 'profile/report.dart';
import 'sources/source_choices.dart';
import 'sources/source_options.dart';
import 'sources/sources.dart';
import 'workspace/site_meta.dart' show siteSetup;
import 'workspace/workspace_template.dart' show newSessionUuid;
import 'worktree/worktree.dart';

export 'acp/delegate.dart' show DelegateTask, DelegateTasks, TaskPhase;
export 'acp/secret_wire.dart' show SecretIdentity;
export 'chat/agent_events.dart' show MessageRole;
export 'chat/attachments.dart' show Attachment, AttachmentKind, imageTypeOf, maxAttachedImage, maxAttachedText, maxAttachments;
export 'ledger/session_ledger.dart' show SessionKind, SessionPlace;
export 'package:coqui_acp_chat/coqui_acp_chat.dart' show AgentQuestion, AttentionKind, ChatMessage, PermissionQuestion, QueueDelivery, QueuedMessage;

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
/// A [draft] is a session whose agent has not started: it starts on the first message, and is [connecting] meanwhile.
enum SessionStatus { preview, draft, disconnected, connecting, connected, working }

/// The name the window that holds a conversation goes by among its views: the person's answers in it are made as this client.
const String ownerClient = 'main';

/// What Canoa attached to a permission request, which the question carries as its `hostData`: the rule it could be remembered as,
/// the task whose agent asks, and what an "allow for the session" given to it covers.
final class RequestHost {
  const RequestHost({this.subject, this.task, this.allowanceKey});

  /// What it would do, in the terms a remembered rule states; null when no rule could cover it, or the card offers none.
  final RuleSubject? subject;

  /// The task whose agent asks, when it is not the conversation's own agent that does.
  final DelegateTask? task;

  /// What an "allow for the session" given to this request covers: the same kind of request from the same task, nothing else.
  final String? allowanceKey;
}

/// What Canoa knows of a permission request beyond the model's: see [RequestHost].
extension RequestOfQuestion on PermissionQuestion {
  RequestHost? get host => hostData is RequestHost ? hostData as RequestHost : null;
  RuleSubject? get subject => host?.subject;
  DelegateTask? get task => host?.task;
  String? get allowanceKey => host?.allowanceKey;
}

/// The most kinds of request an "allow for the session" covers in one task.
const int maxTaskAllowances = 64;

/// The most secret prompts one conversation holds waiting for the person.
const int maxPendingSecrets = 16;

/// A secret prompt of the host, waiting on the person: a request for values or a show of them. [host] is the host's proven identity,
/// which the answer's binding names. Nothing the person types or is shown passes through the agent's conversation.
sealed class SecretPrompt {
  SecretPrompt(this.host, {this.identity});
  final AcpIdentityId host;

  /// What binds this prompt's answer when it is not the store's own [SessionStore.secretIdentity]: the in-process link's.
  final SecretIdentity? identity;

  /// Ends the prompt as cancelled.
  void cancel();
}

final class SecretRequestPrompt extends SecretPrompt {
  SecretRequestPrompt(this.request, super.host, {super.identity});
  final AcpSecretRequestParams request;
  final Completer<AcpSecretRequestReply> reply = Completer();

  @override
  void cancel() {
    if (!reply.isCompleted) reply.complete(const AcpSecretCancelled());
  }
}

final class SecretShowPrompt extends SecretPrompt {
  SecretShowPrompt(this.open, super.host, {super.identity});
  final AcpSecretShowOpen open;

  /// The host's second step, once it arrives.
  final ValueNotifier<AcpSecretShowDeliver?> deliver = ValueNotifier(null);
  final Completer<AcpSecretShowOpened> ready = Completer();
  final Completer<bool> done = Completer();

  @override
  void cancel() {
    if (!ready.isCompleted) ready.complete(const AcpSecretShowOpened.cancelled());
    if (!done.isCompleted) done.complete(false);
  }
}

/// One conversation as Canoa holds it: the model's conversation ([chat]), which keeps what was said, the turn, the queue, what waits
/// for the person and the modes, and beside it what is Canoa's own: where the agent runs, the draft, the ledger's notes, the rules,
/// the tasks handed on, the secret prompts.
class ChatSession {
  ChatSession({
    required String id,
    required String title,
    required String agent,
    required SessionStatus status,
    String? peerId,
    String? remoteSessionId,
    this.endpoint,
    this.websocketFallback,
    String? cwd,
    this.place = SessionPlace.local,
    this.method,
    this.worktree,
    this.source,
    DateTime? created,
    bool agentNames = false,
    model.UpdateRecorder? recorder,
    DateTime Function()? clock,
  }) : created = created ?? DateTime.now(),
       _phase = const {SessionStatus.preview, SessionStatus.draft, SessionStatus.connecting}.contains(status) ? status : null,
       chat = model.ChatSession(
         id: id,
         title: title,
         agent: agent,
         peerId: peerId,
         sessionId: remoteSessionId,
         cwd: cwd,
         recorder: recorder,
         hostNoticesKey: SessionStore.hostNoticesKey,
         clock: clock,
         agentNames: agentNames,
       );

  /// The conversation: what was said, the turn, the queue, the requests waiting, the modes and options.
  final model.ChatSession chat;

  /// Where the agent works, on its machine, when this side knows. A draft's folder until it starts.
  String? get cwd => chat.cwd;
  set cwd(String? value) => chat.cwd = value;

  /// The git worktree [cwd] is, on the session's own branch, when it was started in one.
  SessionWorktree? worktree;

  /// Where the agent runs, as this app reached it.
  SessionPlace place;

  /// The proof a peer's key makes, for a session reached by its endpoint.
  String? method;

  /// The source a session made here runs; null for one reached any other way.
  AgentSource? source;

  /// Whether a draft starts in a new git worktree of its folder, which is made when it starts.
  bool wantsWorktree = false;

  /// Whether this is a chat, which works in a folder Canoa makes and never shows, or a session in a workspace the person chose. A
  /// draft's kind is the person's choice until it starts.
  SessionKind kind = SessionKind.workspace;

  /// Whether the person named it, which the ledger keeps so that no title an agent reports replaces the name later.
  bool namedByPerson = false;

  /// How long the person chose for this session's prompt cache to live; null when they chose none, and then the agent's entry
  /// in canoa.cfg decides, else the chat's default. Kept with the draft and with the session in the ledger.
  CacheLifetime? cacheLifetime;

  /// The account (the host's `[host.accounts]` name) the person chose for a session whose source offers some; null runs the source's first.
  /// Kept with the draft and, as a name, with the session in the ledger: no credential is ever held here.
  String? account;

  /// What is written in a draft's composer, kept with it.
  String draftText = '';

  /// When this app made it. A draft's scratch folder takes its `${date}`, `${time}` and `${datetime}` from it.
  final DateTime created;

  /// Which of canoa.cfg's overlays a draft starts with, by name; null for all of them.
  Set<String>? overlays;

  /// The `[loadouts]` name a draft starts with; null starts with `[defaults] loadout`. Choosing one starts [overlays] over.
  String? loadout;

  /// The id a draft is kept under between runs; null once it starts.
  String? draftId;

  /// Canoa's own id for the session, a UUID minted with its draft: it names the scratch folder before any agent runs, and the
  /// ledger keeps it beside the id the agent later reports. Null for a session not made here.
  String? workspaceId;

  /// The site a session started from a webpage belongs to (the page's host name, as `siteFqdn` writes it); null for any other.
  String? site;

  /// The source a restored draft was kept with, by kind and label, while no source of that name is here.
  String? keptSource;

  /// Whether [title] is taken from the first message: shown here, and never kept in the session list.
  bool titleFromMessage = false;

  /// The first message once it has reached the agent, for a composer still showing it to let it go.
  String? firstSent;

  /// What goes with the next message.
  final List<Attachment> attachments = [];

  /// The tasks this conversation handed to other sources, as the host reported them.
  final DelegateTasks tasks = DelegateTasks();

  /// The task whose thread the conversation shows in place of its own, by id; null while it shows its own.
  String? openChild;

  /// What the person allowed for the rest of a task, by task id and then request: it ends with the task and is never the
  /// conversation's own or another task's.
  final Map<String, Set<String>> taskAllowances = {};

  /// The host's requests for secrets, and shows of them, waiting for the person, oldest first.
  final List<SecretPrompt> pendingSecrets = [];

  /// What the repository holds in this session's folder, as git on its host last said.
  final SessionGit git = SessionGit();

  /// Whether Bypass was chosen before the agent was known, so it is the agent's own where the agent turns out to have one.
  bool bypassBeforeAgent = false;

  /// When the person last answered with the agent's own "always", for saying that a change of mode soon after is what it did.
  DateTime? alwaysAnsweredAt;

  /// Whether the last turn failed and the person has not looked since.
  bool failed = false;

  /// Whether the person pinned the session to the top of the list, or archived it out of the way. Archived hides it only
  /// while it needs nothing from the person.
  bool pinned = false, archived = false;

  /// The files of the changes pane the person marked viewed, each with the version of it they saw.
  final Map<String, int> viewedChanges = {};

  /// What this session was given of its profile, and what it was not; null where the host applied none.
  ProfileReport? profile;

  /// What the host reported of the profile when the session opened, until [SessionStore.applyHostProfile] takes it.
  Object? hostReport;

  /// What the session was opened with, which an earlier conversation opened from it is opened with too.
  AcpSessionSetup setup = AcpSessionSetup.none;

  /// Preferred HTTPS WebTransport or direct WSS endpoint.
  Uri? endpoint;
  Uri? websocketFallback;

  /// A status a draft, a connecting session or a preview has until an agent is bound; null once the conversation's own says.
  SessionStatus? _phase;

  String get id => chat.id;
  String get title => chat.title;
  set title(String value) => chat.retitle(value);
  String get agent => chat.agent;
  set agent(String value) => chat.agent = value;
  String? get peerId => chat.peerId;
  set peerId(String? value) => chat.peerId = value;
  String? get remoteSessionId => chat.sessionId;
  set remoteSessionId(String? value) => chat.sessionId = value;

  SessionStatus get status =>
      _phase ??
      switch (chat.status) {
        ChatStatus.disconnected => SessionStatus.disconnected,
        ChatStatus.connected => SessionStatus.connected,
        ChatStatus.working => SessionStatus.working,
      };
  set status(SessionStatus value) => _phase = value;

  /// Whether the agent may name the conversation, which it may until the person has.
  bool get agentNames => chat.agentNames;
  set agentNames(bool value) => chat.agentNames = value;

  /// Whether a turn ended here since the person last chose this session.
  bool get unread => chat.unread;
  set unread(bool value) => chat.unread = value;

  List<model.ChatMessage> get messages => chat.messages;
  Map<String, model.ChatMessage> get toolEntries => chat.toolEntries;
  Map<String, model.ChatMessage> get compactions => chat.compactions;
  Map<String, TerminalView> get terminals => chat.terminals;

  /// What the agent lets a person choose, its plan, its commands and what it has used, each as it last said.
  List<ConfigOption> get options => chat.options;
  List<PlanEntry> get plan => chat.plan;
  List<AgentCommandInfo> get commands => chat.commands;
  SessionUsage get usage => chat.usage;

  /// What the agent last said of the account's allowances, once it has said; null while it has not.
  UsageReport? get usageReport => chat.usageReport;

  /// Whether this session's agent reports usage, which decides whether the control shows at all.
  bool get reportsUsage => chat.reportsUsage;

  /// What this session has spent, as the agent last said.
  UsageSpend? get spend => chat.spend;

  /// What the agent is waiting for the person to answer, oldest first.
  List<PermissionQuestion> get pendingPermissions => chat.pendingPermissions;

  /// The agent's questions waiting for the person, oldest first.
  List<model.AgentQuestion> get pendingQuestions => chat.pendingQuestions;

  /// Choices made before the agent could take them, by option id: sent before the next prompt.
  Map<String, PendingChoice> get pending => chat.pendingChoices;

  /// Whether Canoa answers this session's permission requests that offer an allow, as the Bypass mode of an agent that has no mode
  /// of its own that asks for nothing. Off unless the person chose it. It sits on the agent's own mode, which stays.
  bool get bypass => chat.bypass;

  /// The mode the person, a profile or the agent last set, as the agent names it: what the ledger keeps. Never the agent's
  /// starting default, which nobody chose.
  String? get chosenMode => chat.chosenMode;

  /// Whether the session is being opened again and its agent has not loaded it yet: nothing about its mode changes then.
  bool get opening => chat.opening;

  /// The mode last asked for, until the agent has taken it or refused it, so a quick second Shift+Tab goes on from it.
  String? get targetMode => chat.targetMode;

  /// Modes the agent refused, which Shift+Tab then skips.
  Set<String> get refusedModes => chat.refusedModes;

  /// Messages written while a turn ran, oldest first. The first is sent when a turn ends as it should.
  List<model.QueuedMessage> get queue => chat.queue;

  /// Why the queue waits for the person rather than going on: a turn that was stopped or failed. Null while it goes on.
  String? get queueHeld => chat.queueHeld;

  /// When something last happened here: the newest message, else when it was made.
  DateTime get lastActivity => messages.isEmpty ? created : messages.last.at;

  /// Whether something here waits for the person's answer.
  bool get needsYou => chat.needsYou || pendingSecrets.isNotEmpty;

  /// Whether its agent has not started: it starts on the first message.
  bool get isDraft => status == SessionStatus.draft || (status == SessionStatus.connecting && remoteSessionId == null);
}

/// Starts [draft]'s agent and binds it with [SessionStore.bindDraft]: what the app does on a draft's first message. It
/// throws, leaving nothing behind, when the agent could not be started.
typedef DraftStarter = Future<void> Function(ChatSession draft);

//// Why [error] happened, in the words it carries for a person: for an agent's own error, the message it gave.
String reasonOf(Object error) => switch (error) {
  WorktreeFailure(:final message) => message,
  _ => model.reasonOf(error),
};

/// The title a draft takes from its first message when it was given none.
String titleFrom(String text) => model.titleFrom(text);

/// Decides what the host decides of a conversation's permission requests: what a remembered rule and a task's allowance answer,
/// and which requests are the person's alone, for the model to put or answer.
final class _Policy implements model.PermissionPolicy {
  _Policy(this.store, this.session);
  final SessionStore store;
  final ChatSession session;

  @override
  Future<void> prepare(AcpPermissionRequest request) async {
    // What another instance revoked since the file was read stops applying before this request is matched, and a file that could
    // not be read earlier is tried again. A read that waits too long applies no rule: the request is asked.
    final rules = store.rules;
    if (rules != null && (rules.rules.isNotEmpty || rules.problem != null)) await rules.refresh();
  }

  @override
  model.PermissionReview review(AcpPermissionRequest request, {required bool forPerson}) {
    final task = SessionStore._taskOf(session, request);
    // A question from an agent on another machine is the person's alone, as a plan is: Bypass, a remembered rule and a task's
    // allowance were each given for agents this person chose to run here. So is one that names a task that has ended or that was
    // never heard of: it is a task's, and what it is cannot be told.
    final late = task == null && SessionStore._namesTask(request);
    final alone = forPerson || late || (task?.remote ?? false);
    final allowanceKey = task == null || alone ? null : SessionStore._allowanceKey(request);
    final subject = alone ? null : store._ruleSubject(session, request);
    final folder = session.cwd;
    final once = request.options.where((option) => option.kind == 'allow_once').firstOrNull;
    final keepable = store.rules != null && subject != null && once != null && folder != null && folder.isNotEmpty;
    return model.PermissionReview(
      forPerson: alone,
      // A task's tool call is its own thread's, not this conversation's.
      ownCall: task == null && !late,
      label: task == null ? '' : ' for ${task.title}',
      hostData: RequestHost(subject: keepable ? subject : null, task: task, allowanceKey: allowanceKey),
    );
  }

  @override
  model.PermissionAnswer? answer(AcpPermissionRequest request, model.PermissionReview review) {
    final host = review.hostData as RequestHost;
    final title = shortLine(request.title) ?? shortLine(request.kind) ?? 'A tool';
    final once = request.options.where((option) => option.kind == 'allow_once').firstOrNull;
    final task = host.task, key = host.allowanceKey;
    if (task != null && key != null && (session.taskAllowances[task.id]?.contains(key) ?? false) && once != null) {
      return model.PermissionAnswer(AcpSelect(once.optionId), 'Allowed for the session of ${task.title}: $title.');
    }
    final subject = host.subject, rules = store.rules, folder = session.cwd;
    if (rules != null && subject != null && once != null && folder != null) {
      final rule = rules.match(source: SessionStore.ruleSource(session), peer: session.peerId ?? '', folder: folder, kind: request.kind, subject: subject);
      if (rule != null) {
        rules.recordUse(rule, switch (subject) {
          CommandSubject(:final command) => command,
          PathSubject(:final path) => path,
          McpSubject(:final name) => name,
        });
        return model.PermissionAnswer(
          AcpSelect(once.optionId),
          'A remembered rule allowed: $title (${rule.describe}). Settings, Permissions shows it and revokes it.',
        );
      }
    }
    return null;
  }
}

/// Shared UI state for desktop, PWA and packaged Chromium extension: Canoa's conversations, each a `coqui_acp_chat` conversation
/// with what is Canoa's own beside it. Transport, verified peers and persistence are intentionally outside it.
class SessionStore extends ChangeNotifier implements AgentSecrets {
  SessionStore({this.recorder, this.ledger, this.rules, this.defaultMode, bool bypassOnceOnly = false,
      this.drafts, this.embedLimit = maxAttachedText, Future<List<int>> Function(String path)? readFile})
      : _bypassOnceOnly = bypassOnceOnly,
        readFile = readFile ?? readLocalFile;

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

  /// Reads the repository in a session's folder through the host its agent runs on; set by the app. Null where none can.
  GitReader? Function(ChatSession session)? gitReader;

  /// The folder a session's changes are read from: its worktree, else its folder, once it has started.
  String? _gitFolder(ChatSession session) => session.isDraft ? null : (session.worktree?.path ?? session.cwd);

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
    unawaited(session.git.beginTurn(reader, folder).then((_) => notifyListeners()));
  }

  /// Asks the host for a source's models and efforts before its agent runs; set by the app. Without it nothing is asked.
  ChoicesLoader? choicesLoader;

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
  bool isAskingChoices(AgentSource source) => _asking.contains(sourceKey(source));

  Future<SourceChoices?> _loadChoices(AgentSource source, String? account) async {
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
    if (same && held != null && DateTime.now().difference(held.fetchedAt) < choicesLife) return;
    _askedAs[key] = as;
    final mine = _asks[key] = (_asks[key] ?? 0) + 1;
    _asking.add(key);
    unawaited(() async {
      try {
        final got = await _loadChoices(source, as);
        if (got != null && !_disposed && _asks[key] == mine) {
          _choices[key] = got.at(DateTime.now());
        }
      } catch (error) {
        debugPrint('Canoa: choices of ${source.label} could not be asked: ${reasonOf(error)}');
      } finally {
        if (_asks[key] == mine) _asking.remove(key);
        if (!_disposed) notifyListeners();
      }
    }());
  }

  /// Asks the host for [source]'s choices again now, whatever is held, and tells whoever listens; what it answers, or what was
  /// held when the host cannot be asked.
  Future<SourceChoices?> askChoicesAgain(AgentSource source) async {
    if (choicesLoader == null) return choicesOf(source);
    final key = sourceKey(source);
    try {
      final as = _askedAs.containsKey(key) ? _askedAs[key] : defaultAccountOf?.call(source);
      _askedAs[key] = as;
      final got = await _loadChoices(source, as);
      if (got != null && !_disposed) {
        _choices[key] = got.at(DateTime.now());
        notifyListeners();
      }
    } catch (error) {
      debugPrint('Canoa: choices of ${source.label} could not be asked: ${reasonOf(error)}');
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
  final Future<List<int>> Function(String path) readFile;

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
    if (kind == AttentionKind.finished && session.id != _selectedId) session.unread = true;
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
      session.status != SessionStatus.connecting && session.status != SessionStatus.preview && !session.opening;

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
    final wasAnswering = chat.bypass;
    if (entry.own) {
      chat.dropChoices(where: (choice) => choice.role == OptionRole.mode);
      chat.presetBypass(true);
      session.bypassBeforeAgent = true;
    } else {
      chat.presetBypass(false);
      final option = modeOptionIn(optionsOf(session));
      if (option != null) chat.holdChoice(PendingChoice(option.id, OptionRole.mode, entry.value, byPerson: true));
    }
    if (chat.bypass && !wasAnswering) {
      chat.note('Mode: Bypass. A request that offers to allow is allowed without asking you.');
    } else if (!chat.bypass && wasAnswering) {
      chat.note('Mode: ${shortLine(entry.label) ?? entry.value}. Requests are asked again.');
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
    final next = nextMode(modeEntries(modeOptionIn(optionsOf(session))), modeOf(session), skip: session.refusedModes);
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
    final entries = [for (final entry in modeEntries(modeOptionIn(optionsOf(session)))) if (!entry.bypass && entry.selectable) entry];
    if (entries.isEmpty) return;
    _chooseDraftMode(session, entries.where((entry) => const {'manual', 'default'}.contains(entry.value)).firstOrNull ?? entries.first);
  }

  /// Puts a draft in the mode [value] names, which is Canoa's own Bypass where the agent has none that asks for nothing.
  void _startInMode(ChatSession session, String? value) {
    if (value == null) return;
    final option = modeOptionIn(optionsOf(session));
    final entries = modeEntries(option);
    final entry = entries.where((entry) => entry.value == value).firstOrNull ??
        (isBypassMode(value) ? entries.where((entry) => entry.bypass).firstOrNull : null);
    if (entry != null && entry.own) {
      session.chat.presetBypass(true);
      session.bypassBeforeAgent = true;
    } else {
      final id = option?.id ?? OptionRole.mode.name;
      session.chat.holdChoice(PendingChoice(id, OptionRole.mode, entry?.value ?? value));
    }
  }

  /// A name an agent or a person gave, as it is shown and logged: one line, with nothing that could reorder what surrounds it.
  static String _named(String name) => shortLine(name) ?? 'an unnamed one';

  final List<ChatSession> _sessions = [];
  final Map<String, VerifiedAgent> _clients = {};
  final Expando<VerifiedChatConnection> _connections = Expando();
  final Map<String, StreamSubscription<model.ChatEvent>> _eventSubscriptions = {};
  final Map<String, StreamSubscription<DelegateNotice>> _taskSubscriptions = {};
  int _nextId = 0;
  String? _selectedId;

  List<ChatSession> get sessions => List.unmodifiable(_sessions);

  /// The sessions the side list shows: every one that exists. A new session that has not been sent a first message is a page
  /// of choices, not a session, so it has no row until Send; one that is starting has its row from then.
  List<ChatSession> get listed => List.unmodifiable([for (final session in _sessions) if (session.status != SessionStatus.draft) session]);
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

  /// What a conversation tells the store, and what the store decides of it for the conversation.
  void _wire(ChatSession session) {
    final chat = session.chat;
    chat.attachClient(ownerClient, name: 'Canoa');
    chat.onChange = () => _changed(session);
    chat.onAttention = (kind) => _needs(session, kind);
    chat.onModeKept = (mode, bypass) => _keepMode(session, mode, bypass);
    chat.onPermissionSettled = (question) => _permissionSettled(session, question);
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
      if (event is TurnEvent) _turnEnded(session, event.turn.outcome ?? TurnOutcome.completed);
    });
  }

  /// A conversation changed: when its connection ended, what Canoa held for that connection ends with it.
  void _changed(ChatSession session) {
    session.tasks.syncNative(session.chat.children, clock(), session.chat.agent);
    if (_clients[session.id] != null && session.chat.connection == null) {
      _clients.remove(session.id);
      _strand(session);
    }
    final report = session.usageReport, account = report?.account;
    if (report != null && account != null && (accountUsage[account]?.revision ?? -1) <= report.revision) {
      accountUsage[account] = report;
    }
    notifyListeners();
  }

  ChatSession createPreviewSession({
    String title = 'New conversation',
    String agent = 'Canoa preview',
  }) {
    final session = _newSession(title: title, agent: agent, status: SessionStatus.preview);
    _sessions.insert(0, session);
    _wire(session);
    _selectedId = session.id;
    notifyListeners();
    return session;
  }

  /// The title a draft has until its first message names it.
  static const draftTitle = 'New session';

  /// A new session that has started nothing: its agent starts on its first message, from [source] in [cwd], and a worktree
  /// is made then if [worktree]. It is kept between runs until it starts or is closed, and never noted in the ledger before.
  ChatSession createDraft({AgentSource? source, String? cwd, bool worktree = false, Set<String>? overlays, String? id,
      String text = '', Map<String, PendingChoice> pending = const {}, List<Attachment> attachments = const [], String? title,
      String? keptSource, bool bypass = false, String? workspaceId, DateTime? created,
      CacheLifetime? cacheLifetime, String? account, SessionKind? kind, String? loadout}) {
    final session = _newSession(
      title: title ?? draftTitle,
      agent: source?.label ?? draftTitle,
      status: SessionStatus.draft,
      cwd: cwd,
      source: source,
      created: created,
      agentNames: title == null,
    )
      ..kind = kind ?? (cwd == null ? SessionKind.chat : SessionKind.workspace)
      ..wantsWorktree = worktree
      ..cacheLifetime = cacheLifetime
      ..account = account
      ..overlays = overlays
      ..loadout = loadout
      ..draftId = id ?? newDraftId()
      ..workspaceId = workspaceId ?? newSessionUuid()
      ..draftText = text
      ..keptSource = source == null ? keptSource : null
      ..bypassBeforeAgent = bypass;
    _insert(session);
    session.chat.presetBypass(bypass);
    for (final choice in pending.values) {
      session.chat.holdChoice(choice);
    }
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
    if (choices != null && choices.cacheLifetime != null && !choices.offersCache(wanted)) {
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
    if (asked == null || choices == null || choices.cacheLifetime == null || choices.offersCache(asked)) return null;
    return asked;
  }

  /// Sets how long [session]'s prompt cache should live; null takes the choice back. A session that has started keeps its own.
  void setCacheLifetime(ChatSession session, CacheLifetime? lifetime) {
    if (!session.isDraft) return;
    session.cacheLifetime = lifetime;
    _keepDraft(session);
    notifyListeners();
  }

  /// Sets the account [session] will run on, by its name in the host's `[host.accounts]`; null takes the choice back, which runs
  /// the source's first. A session that has started keeps its own.
  void setAccount(ChatSession session, String? account) {
    if (!session.isDraft) return;
    session.account = account;
    _keepDraft(session);
    // What an account may use is what its own login lists: the choices are asked again as the one chosen.
    _askChoices(session.source, account: account);
    notifyListeners();
  }

  /// What the sessions on each account last heard of its allowances, by the account's id as the host names it in a report.
  final Map<String, UsageReport> accountUsage = {};

  /// Changes what a draft will start with. Once it has started, its source, folder, worktree and overlays stay as they were.
  void updateDraft(ChatSession session, {AgentSource? source, String? cwd, bool? worktree,
      Set<String>? overlays, bool allOverlays = false, String? loadout, bool defaultLoadout = false, String? text, SessionKind? kind}) {
    if (text != null) session.draftText = text;
    if (session.status == SessionStatus.draft) {
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
      if (overlays != null || allOverlays) session.overlays = allOverlays ? null : overlays;
    }
    _keepDraft(session);
    notifyListeners();
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
        account: record['account'] is String ? record['account'] as String : null,
        overlays: record['overlays'] is List ? {for (final name in record['overlays'] as List) if (name is String) name} : null,
        loadout: record['loadout'] is String ? record['loadout'] as String : null,
        text: record['text'] is String ? record['text'] as String : '',
        title: record['title'] is String ? record['title'] as String : null,
        bypass: record['bypass'] == true,
        workspaceId: record['workspaceId'] is String ? record['workspaceId'] as String : null,
        created: DateTime.tryParse('${record['created']}'),
        pending: {
          for (final choice in [
            for (final json in record['pending'] is List ? record['pending'] as List : const []) ?PendingChoice.fromJson(json),
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
    if (changed) notifyListeners();
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
      'text': session.draftText,
      if (!session.agentNames) 'title': session.title,
      'pending': [for (final choice in session.pending.values) choice.toJson()],
      if (session.bypass) 'bypass': true,
    }, session.attachments);
  }

  /// What a draft's composer holds, kept with it.
  void keepDraftText(ChatSession session, String text) {
    if (!session.isDraft || session.draftText == text) return;
    session.draftText = text;
    _keepDraft(session);
  }

  /// Binds [client], which [opened] a session in [cwd] for [draft], as [attachVerifiedClient] does: from here the session is
  /// started and the ledger notes it. Throws when the draft was closed meanwhile, and [client] is then the caller's to end.
  void bindDraft(ChatSession draft, VerifiedAgent client, AcpOpenedSession opened,
      {required String cwd, SessionWorktree? worktree, SessionPlace place = SessionPlace.local, Uri? endpoint,
      Uri? websocketFallback, String? method}) {
    if (!_sessions.contains(draft) || !draft.isDraft) throw StateError('The session was closed before it started');
    draft
      ..peerId = client.peerId
      ..remoteSessionId = opened.id
      ..cwd = cwd
      ..worktree = worktree
      ..place = place
      ..endpoint = endpoint
      ..websocketFallback = websocketFallback
      ..method = method;
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
    ledger?.rememberSource(sourceKey(source), options: session.options, images: client.takesImages, files: client.takesFiles);
  }

  /// What the agent of [session] is known to take with a prompt: its own word once it runs, else what its source's agent said
  /// last time; null when neither is known.
  PromptTakes? takesOf(ChatSession session) {
    final client = _clients[session.id];
    // A link names a path on this computer, which only an agent running here can read.
    final here = session.place == SessionPlace.local || session.place == SessionPlace.host;
    if (client != null && !client.isClosed) {
      return PromptTakes(images: client.takesImages, files: client.takesFiles, links: here);
    }
    final memory = session.source == null ? null : ledger?.source(sourceKey(session.source!));
    final images = memory?.images, files = memory?.files;
    return images == null || files == null ? null : PromptTakes(images: images, files: files);
  }

  /// The choices shown for [session]: the agent's own once it runs. Before, the models and efforts its source's provider
  /// lists (each model's own efforts, following the model chosen), then what the agent offered last time, and for a role
  /// neither has, the modes the app knows the source's agent offers.
  List<ConfigOption> optionsOf(ChatSession session) {
    if (!session.isDraft) return session.options;
    final source = session.source;
    if (source == null) return const [];
    final model = session.pending.values.where((choice) => choice.role == OptionRole.model).firstOrNull?.value;
    final listed = choicesOf(source)?.options(model: model) ?? const <ConfigOption>[];
    final remembered = ledger?.source(sourceKey(source))?.options ?? const <ConfigOption>[];
    return overlaid(overlaid(listed, remembered), knownModes(source));
  }

  /// When [session]'s source's choices were last seen, for a draft showing them; null otherwise, and when they are only what
  /// the app knows.
  DateTime? optionsSeen(ChatSession session) =>
      session.isDraft && session.source != null && (ledger?.source(sourceKey(session.source!))?.options.isNotEmpty ?? false)
          ? ledger?.source(sourceKey(session.source!))?.seen
          : null;

  /// Puts a person's choice for a draft as its source's default: the profile's [hints] by role where it gives one, else the
  /// last choice made for that source. Choices already made in the draft stay.
  void defaultChoices(ChatSession session, {Map<OptionRole, String> hints = const {}}) {
    if (!session.isDraft) return;
    final source = session.source;
    final memory = source == null ? null : ledger?.source(sourceKey(source));
    for (final role in const [OptionRole.mode, OptionRole.model, OptionRole.effort]) {
      if (session.pending.values.any((choice) => choice.role == role) || (role == OptionRole.mode && session.bypass)) continue;
      final value = hints[role] ?? (role == OptionRole.mode ? defaultMode : null) ?? memory?.chosen[role.name];
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
    notifyListeners();
  }

  /// Bypass chosen before the agent was known is the agent's own, once it turns out to have one that asks for nothing.
  void _resolveBypass(ChatSession session) {
    if (!session.bypass || !session.bypassBeforeAgent || session.isDraft) return;
    session.bypassBeforeAgent = false;
    final option = modeOptionIn(session.options);
    final native = modeEntries(option).where((entry) => entry.bypass && !entry.own).firstOrNull;
    if (option != null && native != null) {
      session.chat.presetBypass(false);
      session.chat.holdChoice(PendingChoice(option.id, OptionRole.mode, native.value));
    }
  }

  /// Puts a person's message to a draft: its agent starts, then takes the message. Until it has started, the message stays the
  /// person's to send again; a start that fails says why in the conversation and leaves the draft as it was.
  Future<void> _startThenDeliver(ChatSession session, String text) async {
    final starter = this.starter;
    if (starter == null) throw StateError('This app cannot start an agent here');
    if (session.status != SessionStatus.draft) throw StateError('It is starting already');
    if (session.source == null) throw StateError('Choose where its agent comes from first');
    session.status = SessionStatus.connecting;
    _starting.add(session.id);
    // The conversation shows the message at once; the composer's attachments go with it, and come back if it cannot start.
    final taken = List.of(session.attachments);
    session.chat.holdFirst(text, attachments: [for (final item in taken) item.name]);
    session.attachments.clear();
    final named = session.agentNames && session.title == draftTitle && text.trim().isNotEmpty;
    if (named) {
      session
        ..title = titleFrom(text)
        ..titleFromMessage = true;
    }
    notifyListeners();
    try {
      try {
        await timed('start', () => starter(session));
      } catch (error) {
        if (session.remoteSessionId == null) {
          session.status = SessionStatus.draft;
          session.chat.releaseFirst();
          session.attachments.addAll(taken);
          if (named) {
            session
              ..title = draftTitle
              ..titleFromMessage = false;
          }
          debugPrint('Canoa: ${session.source?.label ?? 'an agent'} was not started: ${reasonOf(error)}');
          if (_sessions.contains(session)) session.chat.note('Not started: ${reasonOf(error)}');
        }
        // A tab closed while it connected has nobody to tell.
        if (!_sessions.contains(session)) return;
        rethrow;
      }
      if (!_sessions.contains(session)) return;
      if (_clients[session.id] == null) throw StateError('The agent did not start');
      try {
        await _send(session, text, taken, onSent: () {
          // Sent: what follows is queued or sent as usual, even while a v1 agent's first turn runs.
          _starting.remove(session.id);
          session.firstSent = text;
        });
      } catch (_) {
        // The agent runs, but never took the message: it goes back to the composer with what came with it.
        session.chat.releaseFirst();
        for (final item in taken) {
          if (!session.attachments.contains(item)) session.attachments.add(item);
        }
        rethrow;
      }
      session.chat.releaseFirst();
    } finally {
      _starting.remove(session.id);
      notifyListeners();
    }
  }

  /// Sessions whose first message is on its way: started, or starting, and not yet delivered.
  final Set<String> _starting = {};

  /// Whether [session]'s first message is still on its way, during which nothing else is sent.
  bool isStarting(ChatSession session) => _starting.contains(session.id);

  /// The conversation the client [from] serves under the agent's session id [remote], if one is here.
  ChatSession? _sessionOf(String remote, VerifiedAgent from) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.remoteSessionId == remote && identical(_clients[session.id], from)) target = session;
    }
    return target;
  }

  /// Puts a permission request to the person, in the conversation it belongs to: the one [from] serves under the request's
  /// session id. A request about no session here is refused, and one past the conversation's limit is cancelled.
  Future<AcpDecision> askPermission(AcpPermissionRequest request, VerifiedAgent from) {
    final target = _sessionOf(request.sessionId, from);
    if (target == null) return Future.value(const AcpDeny('not a session here'));
    return target.chat.askPermission(request);
  }

  /// Puts an agent's question to the person, in the conversation the client [from] serves under its session id. A question about
  /// no session here is cancelled, and so is one past the conversation's limit.
  Future<AcpElicitationAnswer> askQuestion(AcpElicitation request, VerifiedAgent from) {
    final target = _sessionOf(request.sessionId, from);
    if (target == null) return Future.value(const AcpElicitCancel());
    return target.chat.askQuestion(request);
  }

  /// This app's identity for secret prompts. Without one the host is offered nothing and every prompt is cancelled.
  SecretIdentity? secretIdentity;

  @override
  AcpSecretsCapability? get secretsCapability => secretIdentity?.offer;

  /// The conversation the client [from] serves under the session id [remote], the host's identity, which the proven peer id of [from]
  /// is, and what binds the answer. The link to the host in this process has the secret they share and the host identity derived from
  /// it; any other program this app started has no identity, so it can ask for no secret.
  (ChatSession, AcpIdentityId, SecretIdentity)? _secretTarget(String remote, VerifiedAgent from) {
    final linked = from.linkSecrets;
    // A connection a Noise key proved binds by that key alone, never by the store's WebCrypto identity.
    final identity = from.provedByNoise ? from.noiseSecrets : linked?.identity ?? secretIdentity;
    if (identity == null) return null;
    final AcpIdentityId host;
    try {
      host = linked?.host ?? AcpIdentityId.parse(from.peerId);
    } on AcpSecretException {
      return null;
    }
    final session = _sessionOf(remote, from);
    if (session == null || session.pendingSecrets.length >= maxPendingSecrets) return null;
    identity.verifier.trust(host);
    return (session, host, identity);
  }

  /// What the secret cards of [prompt] check with and bind to.
  SecretCardEnvironment secretEnvironment(SecretPrompt prompt) {
    final identity = prompt.identity ?? secretIdentity!;
    return SecretCardEnvironment(prover: identity.prover, verifier: identity.verifier, expected: prompt.host, host: 'Canoa');
  }

  @override
  Future<AcpSecretRequestReply> askSecret(AcpSecretRequestParams request, VerifiedAgent from) async {
    final target = _secretTarget(request.context.sessionId, from);
    if (target == null) return const AcpSecretCancelled();
    // A key not bound to the host this connection proved is never drawn as a form: the host is told it was cancelled.
    try {
      await target.$3.verifier.collector(
        target.$2, AcpSecretMethod.request, request.context, request.recipient, DateTime.now().millisecondsSinceEpoch ~/ 1000,
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
  Future<AcpSecretShowOpened> openSecretShow(AcpSecretShowOpen open, VerifiedAgent from) {
    final target = _secretTarget(open.context.sessionId, from);
    if (target == null) return Future.value(const AcpSecretShowOpened.cancelled());
    final prompt = SecretShowPrompt(open, target.$2, identity: target.$3);
    target.$1.pendingSecrets.add(prompt);
    _needs(target.$1, AttentionKind.question);
    notifyListeners();
    return prompt.ready.future;
  }

  @override
  Future<bool> deliverSecretShow(AcpSecretShowDeliver deliver, VerifiedAgent from) {
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
    throw const AcpSecretException(AcpSecretFailure.malformed, 'no show is waiting for this');
  }

  /// The person's answer to [prompt], a request of [session]'s.
  void answerSecret(ChatSession session, SecretRequestPrompt prompt, AcpSecretRequestReply reply) {
    if (!session.pendingSecrets.remove(prompt)) return;
    if (!prompt.reply.isCompleted) prompt.reply.complete(reply);
    notifyListeners();
  }

  /// The person's answer to the open step of [prompt], a show of [session]'s. A show that is not going ahead ends here.
  void readySecretShow(ChatSession session, SecretShowPrompt prompt, AcpSecretShowOpened reply) {
    if (!session.pendingSecrets.contains(prompt)) return;
    if (!prompt.ready.isCompleted) prompt.ready.complete(reply);
    if (reply.recipient == null) {
      session.pendingSecrets.remove(prompt);
      if (!prompt.done.isCompleted) prompt.done.complete(false);
    }
    notifyListeners();
  }

  /// The person closed [prompt], a show of [session]'s, having seen the values or not.
  void finishSecretShow(ChatSession session, SecretShowPrompt prompt, bool shown) {
    if (!session.pendingSecrets.remove(prompt)) return;
    if (!prompt.done.isCompleted) prompt.done.complete(shown);
    notifyListeners();
  }

  /// The person's answer to the oldest of [session]'s questions.
  void answerQuestion(ChatSession session, AcpElicitationAnswer answer, {String by = ownerClient}) =>
      session.chat.answerQuestion(answer, by: by);

  /// The person's answer to the oldest question in [session]: the agent's first option that allows once, or refuses once.
  void answerPermission(ChatSession session, {required bool allow}) => session.chat.answerPermission(allow: allow, by: ownerClient);

  /// Marks [path] viewed at [version] in [session]'s changes, or not viewed when [version] is null.
  void markViewed(ChatSession session, String path, int? version) {
    if (version == null) {
      session.viewedChanges.remove(path);
    } else {
      session.viewedChanges[path] = version;
    }
    notifyListeners();
  }

  /// Answers the oldest question in [session] with the agent's own refusal of this one request (else a plain refusal), and
  /// leaves the turn running.
  void _refuseOnce(ChatSession session) {
    final question = session.pendingPermissions.first;
    final refusal = question.options.where((option) => option.kind == 'reject_once').firstOrNull ??
        question.options.where((option) => option.rejects).firstOrNull;
    session.chat.settle(
      question,
      refusal == null ? const AcpDeny('the person declined and said what to do instead') : AcpSelect(refusal.optionId),
      by: ownerClient,
    );
  }

  /// The person's choice of one of the agent's own answers to the oldest question in [session]. What an "always" answer
  /// remembers is the agent's to keep.
  void choosePermission(ChatSession session, AcpPermissionOption option, {String by = ownerClient}) {
    if (session.pendingPermissions.isEmpty) return;
    final question = session.pendingPermissions.first;
    if (!session.chat.choosePermission(option, by: by)) return;
    // A plain Deny stops the turn, as it does in Claude; a subagent's request is that subagent's to lose.
    if (option.rejects && question.task == null && session.status == SessionStatus.working) {
      unawaited(cancelSession(session).catchError((Object _) {}));
    }
  }

  /// An answer settled a request: an "always" is a task's own or the conversation's.
  void _permissionSettled(ChatSession session, PermissionQuestion question) {
    final outcome = question.outcome;
    final answer = outcome?.answer;
    if (outcome == null || outcome.how != Settlement.answered || outcome.by == model.bypassClient || answer is! AcpSelect) return;
    if (question.options.where((option) => option.optionId == answer.optionId).firstOrNull?.kind != 'allow_always') return;
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

  /// What a rule could state of [request] in [session]: commands only while the mode asks about them; changes to files only for a
  /// session on this computer, in a folder where links can be followed and that is neither home nor Canoa's settings; an MCP tool
  /// always. Null when no rule can cover it.
  RuleSubject? _ruleSubject(ChatSession session, AcpPermissionRequest request) {
    final subject = subjectOf(kind: request.kind, rawInput: request.rawInput, toolCall: request.toolCall);
    final folder = session.cwd;
    return switch (subject) {
      PathSubject() when kIsWeb || session.place != SessionPlace.local || folder == null || !(rules?.pathsAllowedIn(folder) ?? false) => null,
      CommandSubject() || McpSubject() when !_modeAsks(session) => null,
      _ => subject,
    };
  }

  /// What a rule calls [session]'s source: its identity (the key the session list remembers it under), not the name it is shown
  /// by, so two sources that share a label do not share rules.
  static String ruleSource(ChatSession session) => session.source == null ? session.agent : sourceKey(session.source!);

  /// The scope the person set, as [type] and [value], for [question], the request the dialog was opened for: remembered as a rule
  /// for this source in this folder, and that request answered with the agent's allow-once answer. Returns why it cannot be made,
  /// or null once it is.
  String? rememberHere(ChatSession session, PermissionQuestion question, RuleType type, String value) {
    final store = rules, folder = session.cwd;
    if (store == null || folder == null || folder.isEmpty) return 'There is nowhere to keep a rule.';
    if (!session.pendingPermissions.contains(question)) return 'That request is no longer waiting.';
    final subject = question.subject, kind = question.kind;
    final once = question.options.where((option) => option.kind == 'allow_once').firstOrNull;
    if (subject == null || kind == null || once == null) return 'No rule can cover this request.';
    if (store.problem != null) return 'The rules file cannot be read, so no rule is made: ${store.problem}';
    final problem = store.checkScope(type, value, subject, folder) ?? store.conflict(folder, type);
    if (problem != null) return problem;
    final rule = store.add(
        source: ruleSource(session), peer: session.peerId ?? '', folder: folder, kind: kind, toolName: subject.toolName, type: type, value: value);
    if (rule == null) return 'Canoa keeps at most ${PermissionRules.maxRules} rules: revoke one in Settings, Permissions first.';
    session.chat.note('Always allow here: ${rule.describe}, for ${shortLine(session.agent) ?? 'this source'} in $folder. Canoa now '
        'answers a matching request with allow once, without asking you. Settings, Permissions revokes it.');
    session.chat.settle(question, AcpSelect(once.optionId), by: ownerClient);
    return null;
  }

  /// The person's answer to a question offering nothing they could choose: it is answered cancelled.
  void cancelPermission(ChatSession session, {String by = ownerClient}) => session.chat.cancelPermission(by: by);

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
    String? workspaceId,
    bool agentNames = false,
  }) {
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    return _insert(_newSession(
      title: title,
      agent: agent,
      status: SessionStatus.disconnected,
      peerId: peerId,
      remoteSessionId: remoteSessionId,
      cwd: cwd,
      place: peerId.startsWith('local:') ? SessionPlace.local : SessionPlace.browser,
      worktree: worktree,
      agentNames: agentNames,
    )..site = site
      ..kind = site == null ? SessionKind.workspace : SessionKind.chat
      ..workspaceId = workspaceId);
  }

  ChatSession _insert(ChatSession session) {
    _sessions.insert(0, session);
    _wire(session);
    _selectedId = session.id;
    notifyListeners();
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
      WebEndpoints(webTransport: endpoint, webSocket: websocketFallback, allowLocalPlainWebSocket: true);
    } else if (endpoint.scheme == 'wss' || endpoint.scheme == 'ws') {
      if (websocketFallback != null) {
        throw ArgumentError('A direct WSS endpoint cannot have a fallback');
      }
      // Plain WebSocket only on loopback, where canoa listens.
      WebEndpoints(webSocket: endpoint, allowLocalPlainWebSocket: true);
    } else {
      throw ArgumentError.value(endpoint, 'endpoint', 'Use HTTPS WebTransport or WSS');
    }
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    return _insert(_newSession(
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
    ));
  }

  /// The model's connection over [client], one for each so that a conversation can tell its own.
  VerifiedChatConnection _connectionOf(VerifiedAgent client) => _connections[client] ??= VerifiedChatConnection(client);

  /// A verified and initialized ACP client can be bound to one session, which the ledger then notes unless [record] is false.
  /// Opening and peer authentication remain the connector's responsibility.
  void attachVerifiedClient(String localId, VerifiedAgent client, {bool record = true}) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.id == localId) target = session;
    }
    if (target == null || target.remoteSessionId == null ||
        target.peerId != client.peerId || client.isClosed) {
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
    if (record) _record(session, client);
    unawaited(refreshGit(session));
    notifyListeners();
  }

  /// A turn of [session] ended, stopped or failed. Failing in a session not shown is told to the person.
  void _turnEnded(ChatSession session, TurnOutcome outcome) {
    if (outcome == TurnOutcome.failed && session.id != _selectedId) session.failed = true;
    _keepText(session);
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
    if (session.peerId != null && session.remoteSessionId != null) ledger?.touch(session.peerId!, session.remoteSessionId!);
  }

  /// The agent took a choice the person made: it is the source's default from now on.
  void _chosen(ChatSession session, OptionRole role, String value) {
    // A switch inside a conversation is one quiet line in it; the first choices, made before the conversation began, are not.
    if ((role == OptionRole.model || role == OptionRole.effort) && !session.isDraft && !isStarting(session) && !session.opening && session.messages.isNotEmpty) {
      final choice = optionIn(session.options, role)?.choices.where((each) => each.value == value).firstOrNull;
      session.chat.note('${role == OptionRole.model ? 'Model' : 'Effort'}: ${choiceLabel(role, choice ?? Choice(value, value))}');
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
    if (ledger == null || remote == null || peerId == null || session.isDraft || session.opening) return;
    ledger!.setMode(peerId, remote, mode: mode, bypass: bypass);
  }

  /// The agent changed its own mode, which is where the session now is: said when it follows the person's "always" answer.
  void _agentChangedMode(ChatSession session, String now) {
    final answered = session.alwaysAnsweredAt;
    if (answered == null || DateTime.now().difference(answered) >= const Duration(seconds: 30)) return;
    session.alwaysAnsweredAt = null;
    final label = modeEntries(modeOptionIn(session.options)).where((entry) => entry.value == now).firstOrNull?.label ?? now;
    session.chat.note('After your "always" answer, ${shortLine(session.agent) ?? 'the agent'} switched this session to $label. '
        'The mode control changes it back.');
  }

  /// Follows the tasks the host reports for [session]. The tasks of an earlier connection ended with it, and the host keeps none for
  /// the next, so a reconnect asks for nothing.
  void _watchTasks(ChatSession session, VerifiedAgent client) {
    unawaited(_taskSubscriptions.remove(session.id)?.cancel());
    _taskSubscriptions[session.id] = client.taskEvents.listen((notice) => _taskEvent(session, notice));
  }

  void _taskEvent(ChatSession session, DelegateNotice notice) {
    if (notice['sessionId'] != session.remoteSessionId || !session.tasks.apply(notice, at: clock())) return;
    final ended = [for (final task in session.tasks.all) if (task.phase.ended) task.id];
    for (final id in ended) {
      session.taskAllowances.remove(id);
      // The host gave the task's question up with the task, so a card still waiting for it has nothing to answer.
      for (final question in [...session.pendingPermissions]) {
        if (question.task?.id == id) session.chat.settle(question, const AcpCancel());
      }
    }
    notifyListeners();
  }

  /// Whether [request]'s tool call id has the shape the host gives a task's: the task's id and a colon before the call's own.
  static bool _namesTask(AcpPermissionRequest request) => request.toolCallId.indexOf(':') > 0;

  /// The task of [session] that [request] comes from, while it runs: the host puts the task's id and a colon before the tool call
  /// id of a task's request. Null for the conversation's own agent's.
  static DelegateTask? _taskOf(ChatSession session, AcpPermissionRequest request) {
    final id = request.toolCallId;
    final colon = id.indexOf(':');
    if (colon <= 0) return null;
    final task = session.tasks[id.substring(0, colon)];
    return task == null || task.phase.ended ? null : task;
  }

  /// What an "allow for the session" of a task's request covers: the kind, the title and what it would run or touch, so a task
  /// that titles every request alike is still asked about each different command or file.
  static String _allowanceKey(AcpPermissionRequest request) {
    final subject = subjectOf(kind: request.kind, rawInput: request.rawInput, toolCall: request.toolCall);
    final what = switch (subject) {
      CommandSubject(:final toolName, :final command) => 'command\n$toolName\n$command',
      PathSubject(:final toolName, :final path) => 'path\n$toolName\n$path',
      McpSubject(:final name) => 'mcp\n$name',
      null => 'input\n${jsonEncode(request.rawInput)}',
    };
    return '${request.kind ?? ''}\n${request.title ?? ''}\n$what';
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
      grokShell: identity.grokShell,
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
      cacheLifetime: session.cacheLifetime?.id,
      account: session.account,
      overlays: session.overlays?.toList(),
    );
  }

  /// Keeps what was said in [session] with its ledger entry, which the ledger does only when the person turned that on.
  void _keepText(ChatSession session) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    if (ledger == null || remote == null || peerId == null || !ledger!.settings.keepText) return;
    ledger!.keepText(peerId, remote, [
      for (final message in session.messages)
        if (message.role == MessageRole.user || message.role == MessageRole.agent)
          KeptMessage(message.role.name, message.text, message.at),
    ]);
  }

  /// The `_meta` key under which `canoa` lists, in its answer to `session/new`, what a person should know about an agent
  /// before using it: what a command-line tool does without asking, and whether it remembers earlier messages.
  static const hostNoticesKey = 'dev.boriken.canoa/notices';

  /// What the agent said when it opened [session].
  void applyOpened(ChatSession session, AcpOpenedSession opened) {
    session.chat.applyOpened(opened);
    _takeHostReport(session, opened);
    notifyListeners();
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
    if (session.status == SessionStatus.preview || session.site != null || session.place == SessionPlace.browser) {
      return 'This conversation cannot be moved to another account.';
    }
    if (session.status == SessionStatus.connecting || session.opening) return 'Wait for the agent to finish starting.';
    if (session.status == SessionStatus.working) return 'Wait for the turn to end, or stop it.';
    if (session.needsYou) return 'Answer what the agent asked first.';
    if (client == null || client.isClosed) return 'The agent is not connected.';
    if (session.remoteSessionId == null || session.cwd == null || client.identity.reopenBy == null || !client.canReopenSessions) {
      return 'The agent cannot open this conversation again, which a new account needs.';
    }
    if (_clients.entries.any((each) => each.key != session.id && identical(each.value, client))) {
      return 'Another conversation uses the same agent. Close it first.';
    }
    return null;
  }

  /// An account as a note names it: its name, or what the login the agent has where it runs is called.
  String _accountWords(String account) => account == machineLogin ? "this machine's login" : account;

  /// Moves the running [session] to [account], which is [machineLogin] for the login the agent has where it runs. Its agent ends, [move] carries the conversation to the account's own folder when
  /// the accounts keep theirs apart (answering the id it has there), [start] starts the agent again for the account, and the
  /// conversation is opened on it with the model, effort and mode it was in. Where the accounts share a folder it is the same
  /// conversation, resumed as it is. If any step fails the agent is started again on the account it had and the conversation is
  /// opened there, and the transcript says why. Answers whether it moved.
  Future<bool> switchAccount(
    ChatSession session,
    String account, {
    required Future<VerifiedAgent> Function(ChatSession session) start,
    Future<AccountMove> Function(ChatSession session, String? from, String to)? move,
    bool expected = false,
  }) async {
    final before = session.account;
    if (before == account) return true;
    final block = accountSwitchBlock(session);
    if (block != null) {
      session.chat.note('Account: not moved to ${_accountWords(account)}. $block');
      return false;
    }
    final old = _clients[session.id]!;
    final oldId = session.remoteSessionId!, cwd = session.cwd!;
    for (final role in const [OptionRole.model, OptionRole.effort]) {
      final option = optionIn(session.options, role);
      if (option != null && option.current.isNotEmpty) session.chat.holdChoice(PendingChoice(option.id, role, option.current));
    }
    final keptMode = session.chat.underlyingMode ?? session.chat.mode, keptBypass = session.bypass;
    final oldEntry = session.peerId == null ? null : ledger?.find(session.peerId!, oldId);

    /// Starts the agent for the session's account and opens [id] on it.
    Future<VerifiedAgent> reopenOn(String id) async {
      final client = await start(session);
      try {
        session.remoteSessionId = id;
        attachVerifiedClient(session.id, client, record: false);
        final opened = await session.chat.reopen(() => client.reopenSession(id, cwd: cwd, setup: session.setup), keptMode: keptMode, keptBypass: keptBypass);
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
      moved = move == null ? null : await move(session, before, account);
      session.account = account;
      final client = await reopenOn(moved != null && moved.moved ? moved.session : oldId);
      if (moved != null && moved.moved) {
        // The conversation has a new id in the new folder: the list follows it, with what the person had made of it.
        if (oldEntry != null) ledger?.remove(oldEntry);
        _record(session, client);
        if (oldEntry != null && session.peerId != null) {
          ledger?.setPlacement(session.peerId!, session.remoteSessionId!, pinned: oldEntry.pinned, archived: oldEntry.archived);
        }
      } else {
        _record(session, client);
      }
    } catch (error) {
      session.account = before;
      try {
        await reopenOn(oldId);
        session.chat.note('Account: not moved to ${_accountWords(account)}: ${reasonOf(error)}. This conversation stays on ${before == null ? 'its first account' : _accountWords(before)}.');
      } catch (again) {
        session.chat.note('Account: not moved to ${_accountWords(account)}: ${reasonOf(error)}. Its agent could not be started again on ${before == null ? 'its first account' : _accountWords(before)} either: ${reasonOf(again)}. Open it again from Earlier conversations.');
      }
      notifyListeners();
      return false;
    }
    final how = moved != null && moved.moved
        ? 'moved to that account\'s own folder (${moved.via == 'record' ? 'from what this host recorded of it' : 'by the agent\'s own export'}) and picked up'
        : 'picked up as it was';
    session.chat.note('Account: ${before == null ? 'the first' : _accountWords(before)} → ${_accountWords(account)}. The conversation was $how.');
    notifyListeners();
    return true;
  }

  /// A person's choice of one of the agent's options. It goes to the agent now when it can take it; before the agent starts, or
  /// while a turn runs, it waits and goes before the next prompt. [value] null takes back a choice still waiting.
  /// Answers whether the agent refused it, which a choice still waiting has not yet.
  Future<bool> choose(ChatSession session, ConfigOption option, String? value) async {
    if (value != null && (session.isDraft || isStarting(session))) {
      final role = roleOf(option);
      session.chat.holdChoice(PendingChoice(option.id, role, value, byPerson: true));
      if (session.isDraft && role == OptionRole.model) _dropUnofferedEffort(session);
      _keepDraft(session);
      return false;
    }
    final refused = await session.chat.choose(option, value);
    if (value == null) _keepDraft(session);
    return refused;
  }

  /// A new model's efforts replace the old one's: an effort chosen for the draft that the model does not take is taken back.
  void _dropUnofferedEffort(ChatSession session) {
    final effort = optionIn(optionsOf(session), OptionRole.effort);
    session.chat.dropChoices(where: (choice) => choice.role == OptionRole.effort && effort != null && !effort.choices.any((each) => each.value == choice.value));
  }

  /// Finishes applying the profile the host prepared for [session], from what it reported when the session opened: the model,
  /// effort and mode are set where the agent offers an option with that value, and the report goes on the session and, in one
  /// line, into it. A reopened session's [keptMode], the one it was left in and which is put back first, wins over the profile's
  /// mode, which is applied only if the agent did not take the kept one. With [expected] and no report, the session says it was
  /// given none.
  Future<void> applyHostProfile(ChatSession session, {bool expected = false, String? keptMode}) async {
    final chat = session.chat;
    final reported = hostProfileOf(session.hostReport, fallback: session.agent);
    session.hostReport = null;
    if (reported == null || reported.report.items.isEmpty) {
      if (reported == null && expected) chat.note('The host gave this agent no profile.');
      notifyListeners();
      return;
    }
    final chosen = _choicesFor(reported.values, [
      for (final option in chat.options)
        (option.id, option.category, [for (final choice in option.choices) choice.value], option.isMode),
    ]);
    final keptTaken = keptMode != null && (keptMode == 'custom' || modeOptionIn(chat.options)?.current == keptMode);
    final set = <String, String>{};
    for (final (id, value, key) in chosen) {
      if (keptTaken && key == 'mode') continue;
      final option = chat.options.firstWhere((option) => option.id == id);
      // Canoa's own Bypass stays where it is: the mode set is the agent's, which it sits on.
      if (option.isMode && !chat.bypass) {
        await chat.chooseMode(ModeEntry(value, value));
      } else {
        await chat.choose(option, value);
      }
      if (chat.options.where((option) => option.id == id).firstOrNull?.current == value) {
        set[key] = option.isMode ? 'session/set_mode' : 'session/set_config_option $id';
      }
    }
    ProfileItem settled(ProfileItem item) {
      if (!const {'model', 'effort', 'mode'}.contains(item.key)) return item;
      final how = set[item.key];
      if (keptTaken && item.key == 'mode') {
        return ProfileItem(item.key, item.layer, ItemState.off, 'the mode this session was left in is applied instead');
      }
      return how != null
          ? ProfileItem(item.key, item.layer, ItemState.applied, how)
          : ProfileItem(item.key, item.layer, ItemState.unsupported, 'the agent offers no ${item.key} option with this value');
    }
    session.profile = ProfileReport(reported.report.harness, [for (final item in reported.report.items) settled(item)]);
    chat.note(session.profile!.summary);
    notifyListeners();
  }

  /// The session's options that take the profile's [values] for `model`, `effort` and `mode`, each by its id or category: as
  /// (option id, value, key). [options] are the session's, as (id, category, values, isMode).
  static List<(String id, String value, String key)> _choicesFor(
    Map<String, String> values,
    List<(String id, String? category, List<String> values, bool isMode)> options,
  ) {
    const ids = {
      'model': ['model'],
      'effort': ['effort', 'reasoning_effort', 'thought_level'],
      'mode': ['mode'],
    };
    const categories = {'model': 'model', 'effort': 'thought_level', 'mode': 'mode'};
    final out = <(String, String, String)>[];
    for (final MapEntry(key: key, value: value) in values.entries) {
      final option = options.where((option) =>
          (option.$2 == categories[key] || ids[key]!.contains(option.$1) || (key == 'mode' && option.$4)) &&
          option.$3.contains(value)).firstOrNull;
      if (option != null) out.add((option.$1, value, key));
    }
    return out;
  }

  /// A v2 prompt response confirms insertion, not agent turn completion.
  /// The prompt is never retried automatically after a disconnect. Written while a turn runs, the message is queued instead,
  /// with what goes with it, and sent when the turn ends.
  ///
  /// With [steer], a turn that runs is stopped and the message goes first, as [model.ChatSession.steer] says: this completes once
  /// the agent has the message, and throws, with the message and its attachments back in the draft, when it could not be sent.
  Future<void> sendText(String text, {bool steer = false}) async {
    final session = selected;
    if (session == null) return;
    // A message written while a card waits declines it once and says what to do instead; the turn is not stopped.
    if (session.pendingPermissions.isNotEmpty && (text.trim().isNotEmpty || session.attachments.isNotEmpty)) _refuseOnce(session);
    await sendTextTo(session, text, steer: steer);
  }

  /// [sendText] for [session], which need not be the selected one. With [attached] false what the composer holds for the session
  /// does not go: a message written in another window of it has none of its own.
  Future<void> sendTextTo(ChatSession session, String text, {bool attached = true, String? from, bool steer = false}) async {
    // A file named by an @ goes only while the message still names it.
    if (attached) session.attachments.removeWhere((item) => item.mention && !text.contains('@${item.name}'));
    final held = attached ? List.of(session.attachments) : <Attachment>[];
    if (text.trim().isEmpty && held.isEmpty) return;
    if (session.status == SessionStatus.preview) {
      sendPreviewText(text);
      return;
    }
    if (isStarting(session)) throw StateError('Its first message is still on its way');
    if (session.isDraft) {
      await _startThenDeliver(session, text);
      return;
    }
    await _send(session, text, held, from: from, steer: steer);
  }

  /// Sends the first queued message of [session] now, and lets the queue go on after it.
  Future<void> sendQueued(ChatSession session) => session.chat.queue.isEmpty ? Future.value() : session.chat.sendQueued(session.chat.queue.first);

  /// Sends everything queued for [session] now as one message, as many as one message holds, and lets the rest go on after it.
  Future<void> sendMerged(ChatSession session) => session.chat.sendMerged();

  /// Stops the running turn and sends [message], one of the queue, first.
  Future<void> steerQueued(ChatSession session, model.QueuedMessage message) => session.chat.steerQueued(message);

  /// What a queued message's attachments were when the person added them, or null when this window did not add them (another
  /// client's message) and they cannot be put back.
  List<Attachment>? _originOf(model.QueuedMessage message) => switch (message.origin) {
    final List<Object?> list when list.every((item) => item is Attachment) => list.cast<Attachment>(),
    _ when message.attachments.isEmpty => const [],
    _ => null,
  };

  /// Joins [message] to the one above it, as [model.ChatSession.mergeQueued] does, keeping what its attachments were. Null when it
  /// was joined, else the reason, in words, why not.
  String? mergeQueued(ChatSession session, model.QueuedMessage message) {
    final index = session.chat.queue.indexOf(message);
    if (index < 1) return 'There is nothing above it to join.';
    final above = session.chat.queue[index - 1];
    final aboveOrigin = _originOf(above), messageOrigin = _originOf(message);
    try {
      final joined = session.chat.mergeQueued(message);
      if (joined != null) {
        joined.origin = aboveOrigin == null || messageOrigin == null
            ? null
            : [
                ...aboveOrigin,
                for (final item in messageOrigin)
                  if (!aboveOrigin.any((other) => other.name == item.name)) item,
              ];
      }
      return null;
    } on StateError catch (error) {
      return error.message;
    }
  }

  /// Joins each queued message to the one above it as far as one message holds them. Null when any were joined, else why not.
  String? mergeAllQueued(ChatSession session) {
    var joined = 0;
    String? problem;
    var index = 1;
    while (index < session.chat.queue.length) {
      final why = mergeQueued(session, session.chat.queue[index]);
      if (why == null) {
        joined++;
      } else {
        problem ??= why;
        index++;
      }
    }
    return joined > 0 ? null : problem;
  }

  /// Moves [message] [by] places in the queue: negative toward the front.
  void moveQueued(ChatSession session, model.QueuedMessage message, int by) => session.chat.moveQueued(message, by);

  /// Takes [message] out of the queue for the person to write again: its attachments go back among the draft's, and [into] is
  /// given its text. Null when that was done, else the reason, in words, it stays queued.
  String? takeQueued(ChatSession session, model.QueuedMessage message, {required void Function(String text) into}) {
    if (!session.chat.queue.contains(message)) return 'It has been sent already.';
    final origin = _originOf(message);
    if (origin == null) return 'It came with attachments this window cannot take back.';
    final fresh = [for (final item in origin) if (!session.attachments.any((other) => other.name == item.name)) item];
    if (session.attachments.length + fresh.length > maxAttachments) return 'The draft has no room for its attachments.';
    if (!session.chat.unqueue(message)) return 'It has been sent already.';
    session.attachments.addAll(fresh);
    _keepDraft(session);
    notifyListeners();
    into(message.text);
    return null;
  }

  /// Changes the text of a queued message.
  void editQueued(ChatSession session, model.QueuedMessage message, String text) => session.chat.editQueued(message, text);

  /// Takes one queued message out, or every one.
  void removeQueued(ChatSession session, [model.QueuedMessage? message]) => session.chat.removeQueued(message);

  /// Hands [text] and what goes with it to the conversation, which sends it, or queues it while a turn runs. Each attachment is
  /// checked and read before anything is sent: one the agent cannot be given stops the message, which stays to be changed.
  /// [onSent] is told once the agent has the message, which for a v1 agent is before its turn ends.
  Future<void> _send(ChatSession session, String text, List<Attachment> attached,
      {void Function()? onSent, String? from, bool steer = false}) async {
    final client = _clients[session.id];
    if (client == null || client.isClosed || session.remoteSessionId == null) {
      throw StateError('A verified ACP peer must be connected');
    }
    final takes = takesOf(session)!;
    final attachments = <AcpAttachment>[
      for (final attachment in attached)
        await toAcp(attachment, takes, embedLimit: embedLimit, read: readFile, agent: _named(session.agent)),
    ];
    _resolveBypass(session);
    final origin = List<Attachment>.unmodifiable(attached);
    if (steer) {
      // The agent is asked to stop first; a message it cannot be asked about stays in the draft.
      final queued = await session.chat.steer(text, attachments: attachments, from: from, origin: origin);
      onSent?.call();
      session.attachments.removeWhere(attached.contains);
      session.draftText = '';
      notifyListeners();
      try {
        await queued?.handedOver;
      } catch (_) {
        // It could not be sent after the turn ended: what the person wrote is theirs again.
        for (final item in attached) {
          if (session.attachments.length < maxAttachments && !session.attachments.any((other) => other.name == item.name)) {
            session.attachments.add(item);
          }
        }
        _keepDraft(session);
        notifyListeners();
        rethrow;
      }
      return;
    }
    await session.chat.send(text, attachments: attachments, from: from, origin: origin);
    onSent?.call();
    session.attachments.removeWhere(attached.contains);
    session.draftText = '';
    notifyListeners();
  }

  /// Whether anything can go with this session's messages: a draft's, or one whose agent this app reaches.
  bool canAttach(ChatSession session) {
    if (session.isDraft) return true;
    final client = _clients[session.id];
    return client != null && !client.isClosed;
  }

  /// Why [attachment] cannot go with [session]'s next message, or null; known only once its agent, or its source's last
  /// time, has said what it takes.
  String? refusalOf(ChatSession session, Attachment attachment) {
    final takes = takesOf(session);
    if (takes == null) return null;
    final (route, why) = routeOf(attachment, takes, embedLimit: embedLimit, agent: _named(session.agent));
    return route == AttachmentRoute.refused ? why : null;
  }

  /// How [attachment] will reach [session]'s agent when it is not as it is, such as a link in place of an embedded file; null
  /// when it goes as it is, or that is not known yet.
  String? routeNote(ChatSession session, Attachment attachment) {
    final takes = takesOf(session);
    return takes == null ? null : routeOf(attachment, takes, embedLimit: embedLimit, agent: _named(session.agent)).$2;
  }

  String? _add(ChatSession session, Attachment attachment) {
    if (!canAttach(session)) return 'This conversation has no agent.';
    if (session.attachments.length >= maxAttachments) return 'That is as many as one message can take.';
    if (session.attachments.any((item) => item.name == attachment.name)) return '${attachment.name} is already there.';
    final refused = refusalOf(session, attachment);
    if (refused != null) return '$refused.';
    session.attachments.add(attachment);
    _keepDraft(session);
    notifyListeners();
    return null;
  }

  /// Takes what a person chose or pasted, by its bytes, to go with their next message: an image, or text. With [path], a file
  /// on this computer that the agent can be given a link to. What is wrong with it, for them to read, or null.
  String? addAttachment(ChatSession session, String name, List<int> bytes, {String? mimeType, String? path}) {
    if (bytes.isEmpty) return '$name is empty.';
    final image = imageTypeOf(name, mimeType);
    if (image != null) {
      if (bytes.length > maxAttachedImage) return '$name is larger than an image can be here.';
      return _add(session, Attachment.image(name, image, bytes, path: path));
    }
    if (path != null) return _add(session, Attachment.file(name, path, bytes.length, mimeType: mimeType));
    if (bytes.length > maxAttachedText) return '$name is larger than a file can be here.';
    final String text;
    try {
      text = utf8.decode(bytes);
    } on FormatException {
      return '$name is not text, and only text files and images can be sent.';
    }
    return _add(session, Attachment.text(name, text, mimeType: mimeType));
  }

  /// [addAttachment], except that an image over the cap is first made smaller (a PNG at a 2048 px, else a 1536 px long edge)
  /// and added as that, under the same name with a `.png` ending; an image that fits, and anything else, goes as it is. What
  /// is wrong, for them to read, when it still cannot be added.
  Future<String?> addAttachmentFitted(ChatSession session, String name, List<int> bytes, {String? mimeType, String? path}) async {
    if (bytes.length <= maxAttachedImage || imageTypeOf(name, mimeType) == null) {
      return addAttachment(session, name, bytes, mimeType: mimeType, path: path);
    }
    final fitted = await fitImage(bytes);
    if (fitted == null) return '$name is larger than an image can be here, and could not be made smaller.';
    final dot = name.lastIndexOf('.');
    return addAttachment(session, '${dot > 0 ? name.substring(0, dot) : name}.png', fitted.bytes, mimeType: 'image/png', path: path);
  }

  /// Takes the file at [path] on this computer, [size] bytes, read only when it is sent: embedded where the agent takes files
  /// and it is small enough text, else linked.
  String? addFile(ChatSession session, String name, String path, int size, {String? mimeType}) {
    if (size == 0) return '$name is empty.';
    return _add(session, Attachment.file(name, path, size, mimeType: mimeType));
  }

  /// Takes the file or folder at [path], which [label] names after an `@` in the message, to go as a link. Nothing happens, and
  /// there is nothing to tell, when it is already there.
  String? addMention(ChatSession session, String label, String path, int size, {bool folder = false}) {
    if (session.attachments.any((item) => item.mention && item.name == label)) return null;
    return _add(session, Attachment.mention(label, path, size, folder: folder));
  }

  /// Takes the folder at [path] on this computer, which the agent is given a link to.
  String? addFolder(ChatSession session, String name, String path) => _add(session, Attachment.folder(name, path));

  /// Takes pasted [text] as an attachment of its own.
  String? addText(ChatSession session, String text) {
    final lines = '\n'.allMatches(text).length + 1;
    var name = 'Pasted text, $lines line${lines == 1 ? '' : 's'}';
    for (var n = 2; session.attachments.any((item) => item.name == name); n++) {
      name = 'Pasted text $n, $lines line${lines == 1 ? '' : 's'}';
    }
    if (utf8.encode(text).length > maxAttachedText) return 'That paste is larger than a file can be here.';
    return _add(session, Attachment.text(name, text, mimeType: 'text/plain'));
  }

  void removeAttachment(ChatSession session, Attachment attachment) {
    session.attachments.remove(attachment);
    _keepDraft(session);
    notifyListeners();
  }

  /// Whether [session] ran on one of this computer's own sources and has started, so the host has its history to move.
  bool canMove(ChatSession session) =>
      session.place == SessionPlace.local && !session.isDraft && session.remoteSessionId != null;

  /// Whether a person can ask this session's agent for its earlier conversations.
  bool canListEarlier(ChatSession session) {
    final client = _clients[session.id];
    return client != null && !client.isClosed && client.canReopenSessions;
  }

  /// The agent's other conversations, where this one is working when that is known. It lists them, and they are its own to name.
  Future<AcpSessionPage> earlierSessions(ChatSession from, {String? cursor}) async {
    final client = _clients[from.id];
    if (client == null || client.isClosed) throw StateError('A verified ACP peer must be connected');
    final page = await client.listSessions(cwd: from.cwd, cursor: cursor);
    return AcpSessionPage(
      page.sessions.where((session) => session.id != from.remoteSessionId).toList(),
      page.nextCursor,
    );
  }

  /// Opens an earlier conversation beside this one, on the same agent. One already open is selected, not opened twice.
  Future<ChatSession> openEarlier(ChatSession from, AcpSessionInfo earlier) async {
    final client = _clients[from.id];
    final cwd = earlier.cwd ?? from.cwd;
    if (client == null || client.isClosed) throw StateError('A verified ACP peer must be connected');
    if (cwd == null) throw StateError('The agent did not say where that conversation was');
    for (final session in _sessions) {
      if (session.remoteSessionId == earlier.id && _clients[session.id] == client) {
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
      final opened = await session.chat.reopen(() => client.reopenSession(earlier.id, cwd: cwd, setup: from.setup));
      _takeHostReport(session, opened);
      // The profile the host reports for this session, else the one of the conversation it opens beside, on the same agent.
      await applyHostProfile(session);
      if (from.profile case final profile? when identical(session.profile, profile)) session.chat.note(profile.summary);
      _record(session, client);
      notifyListeners();
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
    if (!client.canReopenSessions) throw AcpFailure('$agent offers neither session/load nor session/resume');
    final session = addProviderSession(
      title: from.title,
      agent: agent,
      peerId: client.peerId,
      remoteSessionId: sessionId,
      cwd: cwd,
    );
    attachVerifiedClient(session.id, client, record: false);
    try {
      final opened = await session.chat.reopen(() => client.reopenSession(sessionId, cwd: cwd));
      _takeHostReport(session, opened);
      // An agent that sent all of what was said before has shown it; one that sent less is shown the host's copy of it instead.
      bool said(model.ChatMessage m) => m.role == MessageRole.user || m.role == MessageRole.agent;
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
      notifyListeners();
      return session;
    } catch (_) {
      forget(session);
      rethrow;
    }
  }

  /// Opens a session from the ledger again on [client], an agent just started or connected for it, by `session/load` or
  /// `session/resume`, whichever the agent offers. One already open is selected instead, and [client] is left to the caller.
  /// The profile the host reports for it is applied as [applyHostProfile] does; [expected] says whether one should come.
  Future<ChatSession> reopen(LedgerEntry entry, VerifiedAgent client, {bool expected = false}) async {
    for (final session in _sessions) {
      if (session.peerId == entry.peerId && session.remoteSessionId == entry.sessionId) {
        select(session.id);
        return session;
      }
    }
    if (client.peerId != entry.peerId) throw StateError('That agent is not the one this session was opened with');
    if (client.identity.reopenBy == null || !client.canReopenSessions) {
      throw AcpFailure('${entry.harness} offers neither session/load nor session/resume, so this session cannot be opened again');
    }
    final cwd = entry.cwd;
    if (cwd == null) throw StateError('Canoa did not note where this session was, which the agent needs to open it');
    // What the agent lists now names the session, when it still has it under the same name.
    var title = entry.title ?? 'Earlier conversation';
    try {
      if (await client.describe() case AcpDescription(sessions: AcpKnown(value: final listed))) {
        for (final info in listed.sessions) {
          if (info.id == entry.sessionId && !entry.named) title = shortLine(info.title) ?? title;
        }
      }
    } on AcpFailure {
      // Only the name depends on it.
    }
    final endpoint = entry.endpoint == null ? null : Uri.tryParse(entry.endpoint!);
    final session = _insert(_newSession(
      title: title,
      agent: entry.entry,
      status: SessionStatus.disconnected,
      peerId: client.peerId,
      remoteSessionId: entry.sessionId,
      cwd: cwd,
      place: entry.place,
      endpoint: endpoint,
      websocketFallback: entry.fallback == null ? null : Uri.tryParse(entry.fallback!),
      method: entry.method,
      worktree: entry.worktree,
      agentNames: !entry.named,
    ))
      ..site = entry.site
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
        () => client.reopenSession(entry.sessionId, cwd: cwd, setup: entry.site == null ? AcpSessionSetup.none : siteSetup(entry.site!)),
        keptMode: keptMode,
        keptBypass: keptBypass,
      );
      _takeHostReport(session, opened);
      await applyHostProfile(session, expected: expected, keptMode: keptMode);
      if (!opened.replayed && entry.messages.isNotEmpty) {
        session.chat.note('What Canoa kept of it follows, from its own list, not from the agent.');
        for (final kept in entry.messages) {
          session.chat.addMessage(_keptRole(kept.role), kept.text, at: kept.at);
        }
      }
      _record(session, client);
      notifyListeners();
      return session;
    } catch (_) {
      forget(session);
      rethrow;
    }
  }

  static MessageRole _keptRole(String role) => role == 'user' ? MessageRole.user : MessageRole.agent;

  /// Shows what Canoa kept of a session it cannot open again, as an offline preview that sends nothing.
  ChatSession showKept(LedgerEntry entry) {
    final session = createPreviewSession(title: entry.title ?? 'Earlier conversation', agent: entry.harness);
    session.chat.note('What Canoa kept of this session. ${entry.whyNotReopened ?? ''} Nothing typed here reaches an agent.');
    for (final kept in entry.messages) {
      session.chat.addMessage(_keptRole(kept.role), kept.text, at: kept.at);
    }
    return session;
  }

  /// Shows a session the host brought in from another program's own folder, as what was said in it and nothing more. It is a
  /// preview: nothing typed here reaches an agent, and the harness's own files are not touched.
  ChatSession showImported({required String title, required String agent, required List<({MessageRole role, String text})> said}) {
    final session = createPreviewSession(title: title, agent: agent);
    session.chat.note('Brought in from $agent\'s own folder. Nothing typed here reaches an agent.');
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
    _clients.remove(session.id);
    session.chat.detach();
    _cancelSecrets(session);
    session.chat.onChange = null;
    _sessions.remove(session);
    if (_selectedId == session.id) _selectedId = _sessions.isEmpty ? null : _sessions.first.id;
    notifyListeners();
  }

  /// Ends a conversation here: a question still open is answered cancelled, and its agent ends once no other conversation here
  /// uses it. What the agent saved stays with the agent, for Earlier conversations to open again.
  Future<void> close(ChatSession session) async {
    final client = _clients[session.id];
    session.chat.close();
    forget(session);
    if (client != null && !_clients.values.contains(client)) await client.close();
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
    if (client == null || remote == null || session.tasks.hostLive.isEmpty) return;
    if (await client.stopTasks(remote)) _tasksStopped(session);
  }

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
    if (task != null && task.updates.isEmpty && client != null && remote != null) {
      unawaited(client.taskHistory(remote, taskId).then((kept) {
        if (kept != null && session.tasks.backfill(taskId, kept)) notifyListeners();
      }, onError: (Object _) {}));
    }
  }

  /// Brings the conversation's own thread back from a task's.
  void closeChild(ChatSession session) {
    if (session.openChild == null) return;
    session.openChild = null;
    notifyListeners();
  }

  /// Ends one of [session]'s tasks, which need not be running its turn.
  Future<void> stopTask(ChatSession session, String taskId) async {
    final client = _clients[session.id];
    final remote = session.remoteSessionId;
    final task = session.tasks[taskId];
    if (client == null || remote == null || task == null || task.native || task.phase.ended) return;
    if (await client.stopTasks(remote, taskId: taskId)) _tasksStopped(session, only: taskId);
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
      if (question.task != null && (only == null || question.task!.id == only)) {
        session.chat.settle(question, const AcpCancel());
      }
    }
    notifyListeners();
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
      final title = decoded['title'] is String ? decoded['title'] as String : '';
      final selection = decoded['selection'] is String ? decoded['selection'] as String : '';
      final suffix = selection.isEmpty ? '' : '\\nSelected text: ${selection.length > 8000 ? selection.substring(0, 8000) : selection}';
      session.chat.note('Attached page context (not sent): ${title.isEmpty ? uri.host : title}\\n$uri$suffix');
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
    notifyListeners();
  }

  /// Pins [session] to the top of the list, or takes it down; kept with its ledger note where it has one.
  void setPinned(ChatSession session, bool pinned) {
    session.pinned = pinned;
    _keepPlacement(session);
    notifyListeners();
  }

  /// Archives [session], out of the way of the list, or brings it back. One that needs the person stays where it is seen.
  void setArchived(ChatSession session, bool archived) {
    session.archived = archived;
    _keepPlacement(session);
    notifyListeners();
  }

  void _keepPlacement(ChatSession session) {
    final peer = session.peerId, remote = session.remoteSessionId;
    if (peer != null && remote != null) {
      ledger?.setPlacement(peer, remote, pinned: session.pinned, archived: session.archived);
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
    if (peer != null && remote != null) ledger?.touch(peer, remote, title: session.title, byPerson: true);
    notifyListeners();
  }

  /// A new session that starts as [session] did: the same source, overlays and folder (the repository, in a worktree of its own
  /// where [session] had one). What was said in [session] is not copied: an agent's history is its own.
  ChatSession fork(ChatSession session) => createDraft(
    source: session.source,
    kind: session.kind,
    cwd: session.kind == SessionKind.chat ? null : session.worktree?.repo ?? session.cwd,
    worktree: session.worktree != null || session.wantsWorktree,
    overlays: session.overlays,
    loadout: session.loadout,
    title: 'Fork of ${session.title}',
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

  @override
  void dispose() {
    _disposed = true;
    for (final subscription in [..._eventSubscriptions.values, ..._taskSubscriptions.values]) {
      unawaited(subscription.cancel());
    }
    _eventSubscriptions.clear();
    _taskSubscriptions.clear();
    _clients.clear();
    for (final session in _sessions) {
      session.chat.onChange = null;
      session.chat.dispose();
    }
    unawaited(_attention.close());
    super.dispose();
  }
}
