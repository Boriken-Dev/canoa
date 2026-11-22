part of '../session_store.dart';

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
/// A [draft] is a session whose agent has not started: it starts on the first message, and is [connecting] meanwhile.
enum SessionStatus {
  preview,
  draft,
  disconnected,
  connecting,
  connected,
  working,
}

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
  RequestHost? get host =>
      hostData is RequestHost ? hostData as RequestHost : null;
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
    if (!ready.isCompleted) {
      ready.complete(const AcpSecretShowOpened.cancelled());
    }
    if (!done.isCompleted) done.complete(false);
  }
}

/// One conversation as Canoa holds it: the model's conversation ([chat]), which keeps what was said, the turn, the queue, what waits
/// for the person and the modes, and beside it what is Canoa's own: where the agent runs, the draft, the ledger's notes, the rules,
/// the tasks handed on, the secret prompts.
/// When the app first heard of a terminal of the agent's, and of its end: the protocol carries no clock, so this is the time the
/// updates arrived.
final class TerminalSpan {
  TerminalSpan(this.started);

  final DateTime started;
  DateTime? ended;
}

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
       _phase =
           const {
             SessionStatus.preview,
             SessionStatus.draft,
             SessionStatus.connecting,
           }.contains(status)
           ? status
           : null,
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

  /// The account the host ran the session on, and the pool it took it from when [account] names a pool, as the host said them when
  /// the session started; null before then and for a host that names none. Not kept: the next start says them again.
  String? ranOnAccount, ranOnPool;

  /// The account a start or a move of the session names to its agent when that is not [account]: the member of the pool the host
  /// took the turn from. Only while the agent is being started; null otherwise.
  String? startAccount;

  /// When the session's last turn ended, by the store's clock, and whether it failed; null before its first. The host reads how
  /// long ago it was against how long the prompt cache lives.
  DateTime? lastTurnEnded;
  bool lastTurnFailed = false;

  /// The move the person is offered because the account the session runs on has run out; null when there is none. Nothing moves
  /// until they take it.
  AccountOffer? accountOffer;

  /// Whether the account's running out has been dealt with (offered, or dismissed) or is being asked about: it is not offered
  /// again until the account has had room.
  bool offerHeld = false;

  /// What is written in a draft's composer, kept with it.
  String draftText = '';

  /// The message a draft is sending while its agent starts: kept with the draft in place of what the composer holds, so a
  /// crash before the agent answers loses nothing.
  ({String text, List<Attachment> attachments})? sending;

  /// When this app made it. A draft's scratch folder takes its `${date}`, `${time}` and `${datetime}` from it.
  final DateTime created;

  /// Which of canoa.cfg's overlays a draft starts with, by name; null for all of them.
  Set<String>? overlays;

  /// The `[loadouts]` name a draft starts with; null starts with `[defaults] loadout`. Choosing one starts [overlays] over.
  String? loadout;

  /// The saved connection a draft starts on, by name; null for this app's own host (or the one that served the page). A started
  /// session's host is the place and peer it was reached at, never this.
  String? host;

  /// The `[profiles]` name a draft was given its values from; null when it was given none. What it started as is read from
  /// canoa.cfg each time it is compared, so a profile edited meanwhile shows.
  String? preset;

  /// The model, effort and mode (by key) that [preset] asks for and the draft's source has not yet listed, which are chosen
  /// when it does.
  final Map<String, String> presetWants = {};

  /// The values of [preset] the draft cannot take, by key, with why: the agent offers no such model, or no source has the name.
  final Map<String, String> presetUnsupported = {};

  /// The id a draft is kept under between runs; null once it starts.
  String? draftId;

  /// Canoa's own id for the session, a UUID minted with its draft: it names the scratch folder before any agent runs, and the
  /// ledger keeps it beside the id the agent later reports. Null for a session not made here.
  String? workspaceId;

  /// The site a session started from a webpage belongs to (the page's host name, as `siteFqdn` writes it); null for any other.
  String? site;

  /// What the session was started from, which a small client lists the sessions of first; kept in the ledger, never sent to the
  /// agent. Null for a session started from nothing.
  Relation? relation;

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

  /// The other clients attached to this session through the host's hub, as it last said; empty when the host shares none.
  List<HubClient> others = const [];

  /// The cursor of the last message of this session the host's hub gave this app, from which an attach replays only what follows;
  /// null for a session the host does not share.
  String? cursor;

  /// Whether the person pinned the session to the top of the list, or archived it out of the way. Archived hides it only
  /// while it needs nothing from the person.
  bool pinned = false, archived = false;

  /// The files of the changes pane the person marked viewed, each with the version of it they saw.
  final Map<String, int> viewedChanges = {};

  /// What this session was given of its profile, and what it was not; null where the host applied none.
  ProfileReport? profile;

  /// Tools, resources, and statuses reported for MCP servers attached to this session; null when not yet queried.
  McpInventory? mcpInventory;

  /// What the host reported of the profile when the session opened, until [SessionStore.applyHostProfile] takes it.
  Object? hostReport;

  /// What the session was opened with, which an earlier conversation opened from it is opened with too.
  AcpSessionSetup setup = AcpSessionSetup.none;

  /// The agents this conversation has run on and left, by source name: the session it had there and how many things had been
  /// said by then. Switching back to one that has heard nothing since needs no new copy; any other gets the whole conversation
  /// again, since an agent's own session is never added to from outside. Kept while the app runs.
  final Map<String, ({String id, int said})> agentLinks = {};

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

  /// When each terminal of the agent's started and ended as this app saw it, by terminal id.
  final Map<String, TerminalSpan> terminalSpans = {};

  /// What the agent last said its context holds, by kind; empty while it says nothing of the kind.
  List<ContextKind> contextKinds = const [];

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
  bool get isDraft =>
      status == SessionStatus.draft ||
      (status == SessionStatus.connecting && remoteSessionId == null);
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
    // Where links lead and where a repository starts is the host's to say, and the questions put to the rules cannot wait for it.
    final folder = session.cwd;
    final subject = subjectOf(kind: request.kind, rawInput: request.rawInput, toolCall: request.toolCall);
    if (rules != null && folder != null && subject != null && store._pathsHere(session)) {
      await rules.prepare(() {
        rules.pathsAllowedIn(folder);
        rules.match(source: SessionStore.ruleSource(session), peer: session.peerId ?? '', folder: folder, kind: request.kind, subject: subject);
      });
    }
  }

  @override
  model.PermissionReview review(
    AcpPermissionRequest request, {
    required bool forPerson,
  }) {
    final task = SessionStore._taskOf(session, request);
    // A question from an agent on another machine is the person's alone, as a plan is: Bypass, a remembered rule and a task's
    // allowance were each given for agents this person chose to run here. So is one that names a task that has ended or that was
    // never heard of: it is a task's, and what it is cannot be told.
    final late = task == null && SessionStore._namesTask(request);
    final alone = forPerson || late || (task?.remote ?? false);
    final allowanceKey = task == null || alone
        ? null
        : SessionStore._allowanceKey(request);
    final subject = alone ? null : store._ruleSubject(session, request);
    final folder = session.cwd;
    final once = request.options
        .where((option) => option.kind == 'allow_once')
        .firstOrNull;
    final keepable =
        store.rules != null &&
        subject != null &&
        once != null &&
        folder != null &&
        folder.isNotEmpty;
    return model.PermissionReview(
      forPerson: alone,
      // A task's tool call is its own thread's, not this conversation's.
      ownCall: task == null && !late,
      label: task == null ? '' : ' for ${task.title}',
      hostData: RequestHost(
        subject: keepable ? subject : null,
        task: task,
        allowanceKey: allowanceKey,
      ),
    );
  }

  @override
  model.PermissionAnswer? answer(
    AcpPermissionRequest request,
    model.PermissionReview review,
  ) {
    final host = review.hostData as RequestHost;
    final title =
        shortLine(request.title) ?? shortLine(request.kind) ?? 'A tool';
    final once = request.options
        .where((option) => option.kind == 'allow_once')
        .firstOrNull;
    final task = host.task, key = host.allowanceKey;
    if (task != null &&
        key != null &&
        (session.taskAllowances[task.id]?.contains(key) ?? false) &&
        once != null) {
      return model.PermissionAnswer(
        AcpSelect(once.optionId),
        'Allowed for the session of ${task.title}: $title.',
      );
    }
    final subject = host.subject, rules = store.rules, folder = session.cwd;
    if (rules != null && subject != null && once != null && folder != null) {
      final rule = rules.match(
        source: SessionStore.ruleSource(session),
        peer: session.peerId ?? '',
        folder: folder,
        kind: request.kind,
        subject: subject,
      );
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
