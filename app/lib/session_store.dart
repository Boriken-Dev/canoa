import 'dart:async';
import 'acp/client.dart';
import 'acp/local_agent.dart' show readLocalFile;
import 'package:coqui_web/coqui_web.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'chat/agent_events.dart';
import 'chat/attachments.dart';
import 'chat/modes.dart';
import 'chat/session_options.dart';
import 'chat/update_record.dart';
import 'drafts/draft_store.dart';
import 'ledger/session_ledger.dart';
import 'profile/apply.dart';
import 'profile/profile_layers.dart' show LayerNote;
import 'sources/sources.dart';
import 'worktree/worktree.dart';

export 'chat/attachments.dart' show Attachment, AttachmentKind, imageTypeOf, maxAttachedImage, maxAttachedText, maxAttachments;
export 'ledger/session_ledger.dart' show SessionPlace;

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
/// A [draft] is a session whose agent has not started: it starts on the first message, and is [connecting] meanwhile.
enum SessionStatus { preview, draft, disconnected, connecting, connected, working }

enum MessageRole { user, agent, notice, thought, tool, compaction }

/// The most a conversation keeps. An agent's turn can run a very long time, and the oldest entries are what go.
const int maxEntries = 5000;

/// The most one message keeps, since an agent can send any amount.
const int maxMessageText = 2 * 1024 * 1024;

/// The most questions one conversation holds waiting for the person; an agent asking more has the rest answered cancelled.
const int maxPendingPermissions = 64;

class ChatMessage {
  ChatMessage({required this.role, required this.text, required this.at, this.messageId, this.tool, this.compaction});

  final MessageRole role;
  String text;
  final DateTime at;
  final String? messageId;
  final ToolCall? tool;
  final Compaction? compaction;
}

/// A tool an agent wants to use, waiting on the person. Nothing answers it for them but Bypass, which answers a request that
/// offers an allow before it would wait here.
final class PermissionQuestion {
  PermissionQuestion(this.title, this.kind,
      {this.detail, this.options = const [], this.bypassPassed = false, this.forPerson = false});
  final String title;
  final String? kind;

  /// What it would run or touch, when the agent said.
  final String? detail;

  /// The answers the agent offers, in its order.
  final List<AcpPermissionOption> options;

  /// Whether Bypass was on and left this to the person.
  final bool bypassPassed;

  /// Whether it is a question or a plan for the person, which Bypass never answers.
  final bool forPerson;
  final Completer<AcpDecision> _answer = Completer();
}

/// The most questions one conversation holds waiting for the person; an agent asking more has the rest answered cancelled.
const int maxPendingQuestions = 16;

/// An agent's question, waiting on the person. Nothing answers it for them; Bypass is for permission requests only.
final class AgentQuestion {
  AgentQuestion(this.request);
  final AcpElicitation request;
  final Completer<AcpElicitationAnswer> _answer = Completer();
}

/// The most messages one conversation holds waiting for its turn to end.
const int maxQueued = 20;

/// A message written while the agent was working, sent when its turn ends.
final class QueuedMessage {
  QueuedMessage(this.text, this.attachments);
  String text;
  final List<Attachment> attachments;
}

/// What a session needs the person for: a permission request, a question, or a turn it finished.
enum AttentionKind { permission, question, finished }

class ChatSession {
  ChatSession({
    required this.id,
    required this.title,
    required this.agent,
    required this.status,
    this.peerId,
    this.remoteSessionId,
    this.endpoint,
    this.websocketFallback,
    this.cwd,
    this.place = SessionPlace.local,
    this.method,
    this.worktree,
    this.source,
    List<ChatMessage>? messages,
  }) : messages = messages ?? <ChatMessage>[];

  /// Where the agent works, on its machine, when this side knows. A draft's folder until it starts.
  String? cwd;

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

  /// What is written in a draft's composer, kept with it.
  String draftText = '';

  /// When this app made it.
  final DateTime created = DateTime.now();

  /// Which of canoa.cfg's overlays a draft starts with, by name; null for all of them.
  Set<String>? overlays;

  /// The id a draft is kept under between runs; null once it starts.
  String? draftId;

  /// The source a restored draft was kept with, by kind and label, while no source of that name is here.
  ({String kind, String label})? keptSource;

  /// Whether [title] is taken from the first message: shown here, and never kept in the session list.
  bool titleFromMessage = false;

  /// The first message once it has reached the agent, for a composer still showing it to let it go.
  String? firstSent;

  /// Choices made before the agent could take them, by option id: sent before the next prompt.
  final Map<String, PendingChoice> pending = {};

  /// What goes with the next message.
  final List<Attachment> attachments = [];

  /// What the agent is waiting for the person to answer, oldest first.
  final List<PermissionQuestion> pendingPermissions = [];

  /// The agent's questions waiting for the person, oldest first.
  final List<AgentQuestion> pendingQuestions = [];

  /// What the agent lets a person choose, its plan, its commands and what it has used, each as it last said.
  List<ConfigOption> options = const [];
  List<PlanEntry> plan = const [];
  List<AgentCommandInfo> commands = const [];
  SessionUsage usage = const SessionUsage();

  /// Commands the agent ran, which a tool's card shows.
  final Map<String, TerminalView> terminals = {};
  final Map<String, ChatMessage> toolEntries = {};
  final Map<String, ChatMessage> compactions = {};

  /// The person's messages whose attachments the conversation has named, so a message sent here and echoed back is named once.
  final Set<String> attachmentsNamed = {};

  /// Whether the agent may name the conversation, which it may until the person has.
  bool agentNames = false;

  /// Whether Canoa answers this session's permission requests that offer an allow, as the Bypass mode of an agent that has no
  /// mode of its own that asks for nothing. Off unless the person chose it. It sits on the agent's own mode, which stays.
  bool bypass = false;

  /// Whether Bypass was chosen before the agent was known, so it is the agent's own where the agent turns out to have one.
  bool bypassBeforeAgent = false;

  /// The mode the person, a profile or the agent last set, as the agent names it: what the ledger keeps. Never the agent's
  /// starting default, which nobody chose.
  String? chosenMode;

  /// Whether the session is being opened again and its agent has not loaded it yet: nothing about its mode changes then.
  bool opening = false;

  /// The mode last asked for, until the agent has taken it or refused it, so a quick second Shift+Tab goes on from it.
  String? targetMode;

  /// Modes the agent refused, which Shift+Tab then skips.
  final Set<String> refusedModes = {};

  /// How many changes of mode are running or waiting their turn.
  int modesWaiting = 0;

  /// Changes of mode wait their turn here, so each starts from where the one before ended.
  Future<void> modeQueue = Future.value();

  /// Whether a turn ended here since the person last chose this session.
  bool unread = false;

  /// The files of the changes pane the person marked viewed, each with the version of it they saw.
  final Map<String, int> viewedChanges = {};

  /// What this session was given of its profile, and what it was not; null where Canoa applied none.
  ProfileReport? profile;

  /// What the session was opened with, which an earlier conversation opened from it is opened with too.
  AcpSessionSetup setup = AcpSessionSetup.none;

  /// Messages written while a turn ran, oldest first. The first is sent when a turn ends as it should.
  final List<QueuedMessage> queue = [];

  /// Why the queue waits for the person rather than going on: a turn that was stopped or failed. Null while it goes on.
  String? queueHeld;

  /// Whether something here waits for the person's answer.
  bool get needsYou => pendingPermissions.isNotEmpty || pendingQuestions.isNotEmpty;

  final String id;
  String title;
  String agent;
  SessionStatus status;
  String? peerId;
  String? remoteSessionId;
  /// Preferred HTTPS WebTransport or direct WSS endpoint.
  Uri? endpoint;
  Uri? websocketFallback;
  final List<ChatMessage> messages;

  /// Whether its agent has not started: it starts on the first message.
  bool get isDraft => status == SessionStatus.draft || (status == SessionStatus.connecting && remoteSessionId == null);
}

/// Starts [draft]'s agent and binds it with [SessionStore.bindDraft]: what the app does on a draft's first message. It
/// throws, leaving nothing behind, when the agent could not be started.
typedef DraftStarter = Future<void> Function(ChatSession draft);

//// Why [error] happened, in the words it carries for a person: for an agent's own error, the message it gave.
String reasonOf(Object error) => switch (error) {
  StateError(:final message) => message,
  AcpFailure(:final message) => RegExp(r'^Agent error: \{.*\bmessage: (.*)\}$', dotAll: true).firstMatch(message)?.group(1) ?? message,
  WorktreeFailure(:final message) => message,
  _ => '$error',
};

/// The title a draft takes from its first message when it was given none.
String titleFrom(String text) {
  final line = text.trim().split('\n').first.trim();
  return line.length > 60 ? '${line.substring(0, 59).trimRight()}…' : line;
}

/// Shared UI state for desktop, PWA and packaged Chromium extension.
/// Transport, verified peers and persistence are intentionally outside it.
class SessionStore extends ChangeNotifier {
  SessionStore({bool includePreview = true, this.recorder, this.ledger, this.defaultMode,
      this.drafts, this.embedLimit = maxAttachedText, Future<List<int>> Function(String path)? readFile})
      : readFile = readFile ?? readLocalFile {
    if (includePreview) {
      final first = createPreviewSession(title: 'Getting started', agent: 'Canoa preview');
      first.messages.addAll([
        ChatMessage(
          role: MessageRole.agent,
          text: 'This is the Canoa interface preview. Connect a verified ACP peer to use a real agent.',
          at: DateTime.utc(2026, 10, 31),
        ),
      ]);
      final second = createPreviewSession(title: 'Another conversation', agent: 'Canoa preview');
      second.messages.add(ChatMessage(
        role: MessageRole.notice,
        text: 'Separate chats will retain their own agent-issued session IDs when ACP is connected.',
        at: DateTime.utc(2026, 10, 31),
      ));
      select(first.id);
    }
  }

  /// Told the kind and shape of each update this store applies, for a test or the `CANOA_EVENT_LOG` setting.
  final UpdateRecorder? recorder;

  /// Where every session opened or resumed here is noted, to be listed and opened again after the app ends.
  final SessionLedger? ledger;

  /// Where drafts are kept between runs, with what is written in them and what goes with them.
  final DraftKeeper? drafts;

  /// Starts a draft's agent on its first message; set by the app. Without it a draft cannot start.
  DraftStarter? starter;

  /// The largest file, in bytes, sent as the agent's embedded context; a larger one goes as a link to it.
  int embedLimit;

  /// A file's bytes, read when it is sent.
  final Future<List<int>> Function(String path) readFile;

  /// The mode a new session starts in, as an agent names it (one of [modeOrder] for the translators Canoa runs); null leaves
  /// each agent's own. It wins over the person's last choice for the source, and a profile's mode wins over it.
  String? defaultMode;

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
  String? modeOf(ChatSession session) {
    if (session.bypass) return ownBypass;
    final pending = session.pending.values.where((choice) => choice.role == OptionRole.mode).firstOrNull;
    return pending?.value ?? (session.isDraft ? null : modeOptionIn(session.options)?.current);
  }

  /// The agent's own mode, which Canoa's Bypass sits on top of: what the agent says it is in now, never a choice waiting.
  String? underlyingMode(ChatSession session) => session.isDraft ? null : modeOptionIn(session.options)?.current;

  /// Whether [session] is in a mode that asks for nothing, to show it: Canoa's own Bypass, or the agent's own as it says it
  /// is in it now. A choice still waiting is not yet in effect.
  bool inBypass(ChatSession session) {
    if (session.bypass) return true;
    final value = underlyingMode(session);
    if (value == null) return false;
    final label = modeOptionIn(session.options)?.choices.where((choice) => choice.value == value).firstOrNull?.label;
    return isBypassMode(value, label);
  }

  /// Runs one change of mode after those before it, whether the person's menu, Shift+Tab or the banner asked, so each starts
  /// from where the one before ended; a failure of one leaves the next able to run.
  Future<void> _inMode(ChatSession session, Future<void> Function() run) {
    // The first runs at once, so it is as direct as it was before they queued; the rest wait for those before them.
    session.modesWaiting++;
    final idle = session.modesWaiting == 1;
    Future<void> go() async {
      try {
        await run();
      } finally {
        session.modesWaiting--;
      }
    }
    final next = idle ? go() : session.modeQueue.then((_) => go());
    session.modeQueue = next.then<void>((_) {}, onError: (Object _) {});
    return next;
  }

  /// Puts [session] in [entry]'s mode. Canoa's own Bypass answers every request that offers an allow, those already waiting
  /// included, and sits on the agent's mode, which stays; a request with no allow, a question or a plan is still the person's.
  Future<void> chooseMode(ChatSession session, ModeEntry entry) => _inMode(session, () => _chooseMode(session, entry));

  Future<void> _chooseMode(ChatSession session, ModeEntry entry) async {
    if (!entry.selectable) return;
    final wasAnswering = session.bypass;
    if (entry.own) {
      session.pending.removeWhere((_, choice) => choice.role == OptionRole.mode);
      session.bypass = true;
      session.bypassBeforeAgent = session.isDraft;
    } else {
      session.bypass = false;
      final option = modeOptionIn(optionsOf(session));
      if (option != null) {
        // Only an agent that refused it is not asked for it again by Shift+Tab: not one that could not be reached.
        final refused = await choose(session, option, entry.value);
        if (refused) {
          session.refusedModes.add(entry.value);
        } else {
          session.chosenMode = entry.value;
        }
      }
    }
    if (session.bypass && !wasAnswering) {
      _note(session, 'Mode: Bypass. A request that offers to allow is allowed without asking you.');
      _answerWaiting(session);
    } else if (!session.bypass && wasAnswering) {
      _note(session, 'Mode: ${shortText(entry.label) ?? entry.value}. Requests are asked again.');
    }
    _keepMode(session);
    _keepDraft(session);
    notifyListeners();
  }

  /// Takes back a draft's choice of mode, Bypass included, so its agent's own default holds.
  void clearMode(ChatSession session) {
    session.pending.removeWhere((_, choice) => choice.role == OptionRole.mode);
    session.bypass = false;
    _keepDraft(session);
    notifyListeners();
  }

  /// Whether [session]'s mode can be changed now: not while it connects, nor while it is opened again and not yet loaded.
  bool canChangeMode(ChatSession session) =>
      session.status != SessionStatus.connecting && session.status != SessionStatus.preview && !session.opening;

  /// Puts [session] in the next of its modes, as Shift+Tab does. Presses queue against the mode last asked for, not the one
  /// the agent still shows, and a mode the agent refused is skipped.
  Future<void> cycleMode(ChatSession session) {
    if (!canChangeMode(session)) return Future.value();
    final entries = modeEntries(modeOptionIn(optionsOf(session)));
    final next = nextMode(entries, session.targetMode ?? modeOf(session), skip: session.refusedModes);
    if (next == null) return Future.value();
    session.targetMode = next.value;
    return _inMode(session, () => _chooseMode(session, next)).whenComplete(() {
      if (session.targetMode == next.value) session.targetMode = null;
    });
  }

  /// Takes [session] out of Bypass. Canoa's own sat on the agent's mode, which holds again; an agent's own goes to Manual,
  /// or the first mode it has that asks.
  Future<void> leaveBypass(ChatSession session) => _inMode(session, () async {
    if (session.bypass) {
      session.bypass = false;
      _note(session, 'Bypass is off. Requests are asked again.');
      _keepMode(session);
      _keepDraft(session);
      notifyListeners();
      return;
    }
    final entries = [for (final entry in modeEntries(modeOptionIn(optionsOf(session)))) if (!entry.bypass && entry.selectable) entry];
    if (entries.isEmpty) return;
    final preferred = entries.where((entry) => const {'manual', 'default'}.contains(entry.value)).firstOrNull ?? entries.first;
    await _chooseMode(session, preferred);
  });

  /// Whether a request is a question or a plan for the person, whatever mode: Bypass leaves it to them.
  static bool _isForPerson(AcpPermissionRequest request) {
    if (request.kind == 'switch_mode') return true;
    final input = request.rawInput;
    return input is Map && input['questions'] is List;
  }

  /// Answers the requests waiting in [session] that offer an allow and are not a question or a plan.
  void _answerWaiting(ChatSession session) {
    for (final question in [...session.pendingPermissions]) {
      final allow = question.forPerson ? null : bypassAnswer(question.options);
      if (allow == null) continue;
      session.pendingPermissions.remove(question);
      question._answer.complete(AcpSelect(allow.optionId));
      _note(session, 'Bypass allowed: ${question.title} (${shortText(allow.name) ?? allow.kind}).');
    }
  }

  /// Notes in the ledger the mode that was set, and whether Canoa's Bypass is on, to put both back when [session] is opened
  /// again. The agent's starting default was set by nobody and is not kept; nor is `custom`, which cannot be chosen.
  void _keepMode(ChatSession session) {
    final remote = session.remoteSessionId;
    final peerId = session.peerId;
    if (ledger == null || remote == null || peerId == null || session.isDraft || session.opening) return;
    ledger!.setMode(peerId, remote, mode: session.chosenMode, bypass: session.bypass);
  }

  /// The agent changed its own mode, which is where the session now is.
  void _agentChangedMode(ChatSession session, String? before) {
    final now = modeOptionIn(session.options)?.current;
    if (before == null || now == null || now == before || now == 'custom') return;
    session.chosenMode = now;
    _keepMode(session);
  }

  /// Puts a draft in the mode [value] names, which is Canoa's own Bypass where the agent has none that asks for nothing.
  void _startInMode(ChatSession session, String? value) {
    if (value == null) return;
    final option = modeOptionIn(optionsOf(session));
    final entries = modeEntries(option);
    final entry = entries.where((entry) => entry.value == value).firstOrNull ??
        (isBypassMode(value) ? entries.where((entry) => entry.bypass).firstOrNull : null);
    if (entry != null && entry.own) {
      session.bypass = true;
      session.bypassBeforeAgent = true;
    } else {
      final id = option?.id ?? OptionRole.mode.name;
      session.pending[id] = PendingChoice(id, OptionRole.mode, entry?.value ?? value);
    }
  }

  /// A name an agent or a person gave, as it is shown and logged: one line, with nothing that could reorder what surrounds it.
  static String _named(String name) => shortText(name) ?? 'an unnamed one';

  final List<ChatSession> _sessions = [];
  final Map<String, VerifiedAgent> _verifiedClients = {};
  final Map<String, StreamSubscription<AcpSessionUpdate>> _subscriptions = {};
  final Map<String, StreamSubscription<AcpEvent>> _turnSubscriptions = {};
  int _nextId = 0;
  String? _selectedId;

  List<ChatSession> get sessions => List.unmodifiable(_sessions);
  String? get selectedId => _selectedId;

  ChatSession? get selected {
    for (final session in _sessions) {
      if (session.id == _selectedId) return session;
    }
    return null;
  }

  ChatSession createPreviewSession({
    String title = 'New conversation',
    String agent = 'Canoa preview',
  }) {
    final session = ChatSession(
      id: 'local-${++_nextId}',
      title: title,
      agent: agent,
      status: SessionStatus.preview,
    );
    _sessions.insert(0, session);
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
      ({String kind, String label})? keptSource, bool bypass = false}) {
    final session = ChatSession(
      id: 'local-${++_nextId}',
      title: title ?? draftTitle,
      agent: source?.label ?? draftTitle,
      status: SessionStatus.draft,
      cwd: cwd,
      source: source,
    )
      ..wantsWorktree = worktree
      ..overlays = overlays
      ..draftId = id ?? newDraftId()
      ..draftText = text
      ..keptSource = source == null ? keptSource : null
      ..agentNames = title == null
      ..bypass = bypass
      ..bypassBeforeAgent = bypass;
    session.pending.addAll(pending);
    session.attachments.addAll(attachments.take(maxAttachments));
    _insert(session);
    _keepDraft(session);
    return session;
  }

  /// Changes what a draft will start with. Once it has started, its source, folder, worktree and overlays stay as they were.
  void updateDraft(ChatSession session, {AgentSource? source, String? cwd, bool? worktree, Set<String>? overlays,
      bool allOverlays = false, String? text}) {
    if (text != null) session.draftText = text;
    if (session.status == SessionStatus.draft) {
      if (source != null && source != session.source) {
        session
          ..source = source
          ..keptSource = null
          ..agent = source.label;
        // Choices for another source's options mean nothing to this one's.
        session.pending.clear();
        session.bypass = false;
      }
      if (cwd != null) session.cwd = cwd;
      if (worktree != null) session.wantsWorktree = worktree;
      if (overlays != null || allOverlays) session.overlays = allOverlays ? null : overlays;
    }
    _keepDraft(session);
    notifyListeners();
  }

  /// Every draft kept from an earlier run, as sessions; [source] finds each one's source again by its kind and label.
  Future<void> restoreDrafts(AgentSource? Function(String kind, String label) source) async {
    final kept = await drafts?.load() ?? const <KeptDraft>[];
    for (final draft in kept) {
      final record = draft.record;
      final from = record['source'] is Map ? record['source'] as Map : const {};
      final kept = from['kind'] is String && from['label'] is String
          ? (kind: from['kind'] as String, label: from['label'] as String)
          : null;
      createDraft(
        id: draft.id,
        source: kept == null ? null : source(kept.kind, kept.label),
        keptSource: kept,
        cwd: record['cwd'] is String ? record['cwd'] as String : null,
        worktree: record['worktree'] == true,
        overlays: record['overlays'] is List ? {for (final name in record['overlays'] as List) if (name is String) name} : null,
        text: record['text'] is String ? record['text'] as String : '',
        title: record['title'] is String ? record['title'] as String : null,
        bypass: record['bypass'] == true,
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

  void _keepDraft(ChatSession session) {
    final id = session.draftId;
    if (drafts == null || id == null || !session.isDraft) return;
    drafts!.save(id, {
      'created': session.created.toUtc().toIso8601String(),
      'source': ?switch ((session.source, session.keptSource)) {
        (final source?, _) => {'kind': source.kind.name, 'label': source.label},
        (null, final kept?) => {'kind': kept.kind, 'label': kept.label},
        _ => null,
      },
      'cwd': ?session.cwd,
      'worktree': session.wantsWorktree,
      'overlays': ?session.overlays?.toList(),
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
    final client = _verifiedClients[session.id];
    if (source == null || client == null) return;
    ledger?.rememberSource(sourceKey(source), options: session.options, images: client.takesImages, files: client.takesFiles);
  }

  /// What the agent of [session] is known to take with a prompt: its own word once it runs, else what its source's agent said
  /// last time; null when neither is known.
  PromptTakes? takesOf(ChatSession session) {
    final client = _verifiedClients[session.id];
    // A link names a path on this computer, which only an agent running here can read.
    final here = session.place == SessionPlace.local || session.place == SessionPlace.host;
    if (client != null && !client.isClosed) {
      return PromptTakes(images: client.takesImages, files: client.takesFiles, links: here);
    }
    final memory = session.source == null ? null : ledger?.source(sourceKey(session.source!));
    final images = memory?.images, files = memory?.files;
    return images == null || files == null ? null : PromptTakes(images: images, files: files);
  }

  /// The choices shown for [session]: the agent's own once it runs, else what its source's agent offered last time.
  List<ConfigOption> optionsOf(ChatSession session) {
    if (!session.isDraft) return session.options;
    final source = session.source;
    return source == null ? const [] : ledger?.source(sourceKey(source))?.options ?? const [];
  }

  /// When [session]'s source's choices were last seen, for a draft showing them; null otherwise.
  DateTime? optionsSeen(ChatSession session) =>
      session.isDraft && session.source != null ? ledger?.source(sourceKey(session.source!))?.seen : null;

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
      final option = memory == null ? null : optionIn(memory.options, role);
      final id = option?.id ?? role.name;
      session.pending[id] = PendingChoice(id, role, value);
    }
    _keepDraft(session);
    notifyListeners();
  }

  /// Sends each choice made before the agent could take it, and says in [session] what it would not take.
  Future<void> _applyPending(ChatSession session) async {
    // Bypass chosen before the agent was known is the agent's own, once it turns out to have one that asks for nothing.
    if (session.bypass && session.bypassBeforeAgent && !session.isDraft) {
      session.bypassBeforeAgent = false;
      final option = modeOptionIn(session.options);
      final native = modeEntries(option).where((entry) => entry.bypass && !entry.own).firstOrNull;
      if (option != null && native != null) {
        session.bypass = false;
        session.pending[option.id] = PendingChoice(option.id, OptionRole.mode, native.value);
      }
    }
    if (session.pending.isEmpty) return;
    final waiting = [...session.pending.values];
    session.pending.clear();
    for (final choice in waiting) {
      // `custom` only says what the agent is in: it is never chosen.
      if (choice.role == OptionRole.mode && choice.value == 'custom') continue;
      final option = session.options.where((option) => option.id == choice.id && option.choices.any((each) => each.value == choice.value)).firstOrNull ??
          session.options.where((option) => roleOf(option) == choice.role && option.choices.any((each) => each.value == choice.value)).firstOrNull;
      if (option == null) {
        _note(session, 'The agent offers no ${choice.role == OptionRole.other ? choice.id : choice.role.name} of ${shortText(choice.value) ?? 'that'} now, so it was not set.');
        continue;
      }
      // A mode is sent even when it is the one shown: what shows may not be what is in effect.
      if (option.current != choice.value || choice.role == OptionRole.mode) await _set(session, option, choice.value);
      final took = session.options.where((each) => each.id == option.id).firstOrNull?.current == choice.value;
      if (took && choice.role == OptionRole.mode) session.chosenMode = choice.value;
      if (took && choice.byPerson && session.source != null) {
        ledger?.rememberChoice(sourceKey(session.source!), choice.role, choice.value);
      }
    }
    _keepMode(session);
    notifyListeners();
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
    final named = session.agentNames && session.title == draftTitle && text.trim().isNotEmpty;
    if (named) {
      session
        ..title = titleFrom(text)
        ..titleFromMessage = true;
    }
    notifyListeners();
    try {
      try {
        await starter(session);
      } catch (error) {
        if (session.remoteSessionId == null) {
          session.status = SessionStatus.draft;
          if (named) {
            session
              ..title = draftTitle
              ..titleFromMessage = false;
          }
          if (_sessions.contains(session)) _note(session, 'Not started: ${reasonOf(error)}');
        }
        rethrow;
      }
      if (_verifiedClients[session.id] == null) throw StateError('The agent did not start');
      await _deliver(session, text, List.of(session.attachments), onSent: () {
        // Sent: what follows is queued or sent as usual, even while a v1 agent's first turn runs.
        _starting.remove(session.id);
        session.firstSent = text;
      });
    } finally {
      _starting.remove(session.id);
      notifyListeners();
    }
  }

  /// Sessions whose first message is on its way: started, or starting, and not yet delivered.
  final Set<String> _starting = {};

  /// Whether [session]'s first message is still on its way, during which nothing else is sent.
  bool isStarting(ChatSession session) => _starting.contains(session.id);

  /// Puts a permission request to the person, in the conversation it belongs to: the one [from] serves under the request's
  /// session id. A request about no session here is refused, and one past [maxPendingPermissions] is cancelled.
  Future<AcpDecision> askPermission(AcpPermissionRequest request, VerifiedAgent from) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.remoteSessionId == request.sessionId && identical(_verifiedClients[session.id], from)) target = session;
    }
    if (target == null) return Future.value(const AcpDeny('not a session here'));
    if (target.pendingPermissions.length >= maxPendingPermissions) {
      _note(target, 'The agent asked more questions than the $maxPendingPermissions a conversation holds waiting, so the rest '
          'were answered cancelled.');
      notifyListeners();
      return Future.value(const AcpCancel());
    }
    // ACP has the tool call applied before the prompt shows, so the conversation shows what is being asked about.
    if (request.toolCall != null) _applyTool(target, request.toolCall!);
    final title = shortText(request.title) ?? shortText(request.kind) ?? 'A tool';
    final forPerson = _isForPerson(request);
    if (target.bypass && !forPerson) {
      final allow = bypassAnswer(request.options);
      if (allow != null) {
        _note(target, 'Bypass allowed: $title (${shortText(allow.name) ?? allow.kind}).');
        notifyListeners();
        return Future.value(AcpSelect(allow.optionId));
      }
    }
    final question = PermissionQuestion(
      title,
      request.kind,
      detail: [
        ?shortText(request.description),
        ?describeToolInput(request.rawInput),
      ].join('\n').trim(),
      options: request.options,
      bypassPassed: target.bypass,
      forPerson: forPerson,
    );
    target.pendingPermissions.add(question);
    _needs(target, AttentionKind.permission);
    notifyListeners();
    return question._answer.future;
  }

  /// Puts an agent's question to the person, in the conversation the client [from] serves under its session id. A question about
  /// no session here is cancelled, and so is one past [maxPendingQuestions].
  Future<AcpElicitationAnswer> askQuestion(AcpElicitation request, VerifiedAgent from) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.remoteSessionId == request.sessionId && identical(_verifiedClients[session.id], from)) target = session;
    }
    if (target == null) return Future.value(const AcpElicitCancel());
    if (target.pendingQuestions.length >= maxPendingQuestions) {
      _note(target, 'The agent asked more questions than the $maxPendingQuestions a conversation holds waiting, so the rest '
          'were answered cancelled.');
      notifyListeners();
      return Future.value(const AcpElicitCancel());
    }
    final question = AgentQuestion(request);
    target.pendingQuestions.add(question);
    _needs(target, AttentionKind.question);
    notifyListeners();
    return question._answer.future;
  }

  /// The person's answer to the oldest of [session]'s questions.
  void answerQuestion(ChatSession session, AcpElicitationAnswer answer) {
    if (session.pendingQuestions.isEmpty) return;
    session.pendingQuestions.removeAt(0)._answer.complete(answer);
    notifyListeners();
  }

  /// The person's answer to the oldest question in [session]: the agent's first option that allows once, or refuses once.
  void answerPermission(ChatSession session, {required bool allow}) {
    if (session.pendingPermissions.isEmpty) return;
    final question = session.pendingPermissions.removeAt(0);
    question._answer.complete(allow ? const AcpAllow() : const AcpDeny('the person refused'));
    notifyListeners();
  }

  /// Marks [path] viewed at [version] in [session]'s changes, or not viewed when [version] is null.
  void markViewed(ChatSession session, String path, int? version) {
    if (version == null) {
      session.viewedChanges.remove(path);
    } else {
      session.viewedChanges[path] = version;
    }
    notifyListeners();
  }

  /// The person's choice of one of the agent's own answers to the oldest question in [session]. What an "always" answer
  /// remembers is the agent's to keep.
  void choosePermission(ChatSession session, AcpPermissionOption option) {
    if (session.pendingPermissions.isEmpty) return;
    final question = session.pendingPermissions.first;
    if (!question.options.contains(option)) return;
    session.pendingPermissions.removeAt(0);
    question._answer.complete(AcpSelect(option.optionId));
    notifyListeners();
  }

  /// The person's answer to a question offering nothing they could choose: it is answered cancelled.
  void cancelPermission(ChatSession session) {
    if (session.pendingPermissions.isEmpty) return;
    session.pendingPermissions.removeAt(0)._answer.complete(const AcpCancel());
    notifyListeners();
  }

  /// Answers everything still waiting in [session]: its permission requests with [answer], and its questions cancelled.
  void _endWaiting(ChatSession session, AcpDecision answer) {
    for (final question in session.pendingPermissions) {
      question._answer.complete(answer);
    }
    session.pendingPermissions.clear();
    for (final question in session.pendingQuestions) {
      question._answer.complete(const AcpElicitCancel());
    }
    session.pendingQuestions.clear();
  }

  /// A session over a browser provider's connection, which has no endpoint of Canoa's own.
  ChatSession addProviderSession({
    required String title,
    required String agent,
    required String peerId,
    required String remoteSessionId,
    String? cwd,
    SessionWorktree? worktree,
  }) {
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    return _insert(ChatSession(
      id: 'local-${++_nextId}',
      title: title,
      agent: agent,
      status: SessionStatus.disconnected,
      peerId: peerId,
      remoteSessionId: remoteSessionId,
      cwd: cwd,
      place: peerId.startsWith('local:') ? SessionPlace.local : SessionPlace.browser,
      worktree: worktree,
    ));
  }

  ChatSession _insert(ChatSession session) {
    _sessions.insert(0, session);
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
    return _insert(ChatSession(
      id: 'local-${++_nextId}',
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
    ));
  }

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
    unawaited(_subscriptions.remove(localId)?.cancel());
    _verifiedClients[localId] = client;
    session.status = SessionStatus.connected;
    _subscriptions[localId] = client.updates.listen(
      (update) {
        if (update.sessionId == session.remoteSessionId) {
          _applyUpdate(session, update);
        }
      },
      onDone: () {
        _verifiedClients.remove(localId);
        session.status = SessionStatus.disconnected;
        _endWaiting(session, const AcpDeny('the agent went away'));
        notifyListeners();
      },
    );
    // A v1 turn has no state updates: its end arrives as an event.
    unawaited(_turnSubscriptions.remove(localId)?.cancel());
    _turnSubscriptions[localId] = client.events.listen((event) {
      if (event is AcpOutputLost) {
        final messages = event.lines == 1 ? '1 message' : '${event.lines} messages';
        _note(session, 'While nobody was connected, $messages from the agent (${event.bytes} bytes) were dropped, '
            'so something it said or did is missing here.');
        notifyListeners();
        return;
      }
      if (event is AcpBlocked) {
        // The agent was told the question was cancelled, and its turn goes on until it ends it.
        _note(session, 'A question from the agent was answered cancelled: ${event.message}.');
        notifyListeners();
        return;
      }
      if (client.protocolVersion != 1) return;
      if (event is AcpCompleted || event is AcpCancelled) {
        session.status = SessionStatus.connected;
        if (event is AcpCancelled) _note(session, 'Stopped.');
        if (event is AcpCompleted) _needs(session, AttentionKind.finished);
        _keepText(session);
        _turnEnded(session, event is AcpCompleted ? null : 'the turn was stopped');
        notifyListeners();
      } else if (event is AcpFailed) {
        session.status = SessionStatus.connected;
        session.messages.add(ChatMessage(role: MessageRole.notice, text: event.message, at: DateTime.now()));
        _needs(session, AttentionKind.finished);
        _keepText(session);
        _turnEnded(session, 'the turn failed');
        notifyListeners();
      }
    });
    if (record) _record(session, client);
    notifyListeners();
  }

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

  /// Notices taken from one answer; the rest are passed over.
  static const maxHostNotices = 4;

  /// What the agent said when it opened [session].
  void applyOpened(ChatSession session, AcpOpenedSession opened) {
    _setOptions(session, opened.raw['configOptions'], opened.raw['modes']);
    session.commands = parseCommands(opened.raw['availableCommands']) ?? session.commands;
    final meta = opened.raw['_meta'];
    if (meta is Map && meta[hostNoticesKey] is List) {
      // Any agent can send these, not only the host, so each is shown as the agent's word, never as Canoa's.
      for (final notice in (meta[hostNoticesKey] as List).take(maxHostNotices)) {
        final text = shortText(notice);
        if (text != null) _note(session, 'The agent says: $text');
      }
    }
    notifyListeners();
  }

  void _setOptions(ChatSession session, Object? configOptions, Object? modes) {
    final parsed = parseConfigOptions(configOptions);
    // An agent with both says the same thing twice, and ACP prefers the options.
    final mode = parsed == null || parsed.isEmpty ? parseModes(modes) : null;
    if (parsed != null && parsed.isNotEmpty) {
      session.options = parsed;
    } else if (mode != null) {
      session.options = [mode];
    }
  }

  /// A person's choice of one of the agent's options. It goes to the agent now when it can take it; before the agent starts, or
  /// while a turn runs, it waits and goes before the next prompt. [value] null takes back a choice still waiting.
  /// Answers whether the agent refused it, which a choice still waiting has not yet.
  Future<bool> choose(ChatSession session, ConfigOption option, String? value) async {
    final role = roleOf(option);
    if (value == null) {
      session.pending.remove(option.id);
      _keepDraft(session);
      notifyListeners();
      return false;
    }
    if (session.isDraft || session.status == SessionStatus.working || isStarting(session)) {
      session.pending[option.id] = PendingChoice(option.id, role, value, byPerson: true);
      _keepDraft(session);
      notifyListeners();
      return false;
    }
    session.pending.remove(option.id);
    final refused = await _set(session, option, value);
    if (session.options.where((each) => each.id == option.id).firstOrNull?.current == value && session.source != null) {
      ledger?.rememberChoice(sourceKey(session.source!), role, value);
      _rememberSource(session);
    }
    return refused;
  }

  /// Sets one of the agent's choices. It stays as it was unless the agent agrees, which it says in its answer or an update; a
  /// refusal is said in the conversation, with the agent's reason.
  /// Answers whether the agent refused it: an agent that could not be reached has refused nothing.
  Future<bool> _set(ChatSession session, ConfigOption option, String value) async {
    final client = _verifiedClients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return false;
    var refused = false;
    try {
      if (option.isMode) {
        await client.setMode(remote, value);
        _setCurrent(session, option.id, value);
      } else {
        final reply = await client.setConfigOption(remote, option.id, value);
        final told = parseConfigOptions(reply['configOptions']);
        if (told != null && told.isNotEmpty) {
          session.options = told;
        } else {
          _setCurrent(session, option.id, value);
        }
      }
    } on AcpFailure catch (error) {
      final shown = option.choices.where((choice) => choice.value == value).firstOrNull?.label ?? value;
      _note(session, 'The agent did not change ${option.label} to ${shortText(shown) ?? 'that'}: ${reasonOf(error)}');
      refused = true;
    }
    notifyListeners();
    return refused;
  }

  /// Finishes applying a profile to [session], just opened with [launch]'s setup: the model, effort and mode are set where
  /// the agent offers an option with that value, and the report goes on the session and, in one line, into it. [notes]
  /// are what the person was not shown of the layers, such as one left out, and go into it too.
  Future<void> applyProfile(ChatSession session, ProfileLaunch launch, PreparedProfile prepared,
      {required String harness, List<LayerNote> notes = const [], bool keepOwnMode = false}) async {
    session.setup = launch.setup;
    for (final note in notes) {
      _note(session, 'Profile, ${note.layer}: ${note.text}');
    }
    if (launch.items.isEmpty) {
      notifyListeners();
      return;
    }
    final chosen = choicesFor(prepared, [
      for (final option in session.options)
        (option.id, option.category, [for (final choice in option.choices) choice.value], option.isMode),
    ]);
    final set = <String, String>{};
    for (final (id, value, key) in chosen) {
      // A reopened session's own mode, kept from before, is put back after this and wins: it is not set twice.
      if (keepOwnMode && key == 'mode') continue;
      final option = session.options.firstWhere((option) => option.id == id);
      await _set(session, option, value);
      final now = session.options.where((option) => option.id == id).firstOrNull;
      if (now?.current == value) {
        set[key] = option.isMode ? 'session/set_mode' : 'session/set_config_option $id';
        if (key == 'mode') session.chosenMode = value;
      }
    }
    ProfileItem settled(ProfileItem item) {
      if (!const {'model', 'effort', 'mode'}.contains(item.key)) return item;
      final how = set[item.key];
      if (keepOwnMode && item.key == 'mode') {
        return ProfileItem(item.key, item.layer, ItemState.off, 'the mode this session was left in is applied instead');
      }
      return how != null
          ? ProfileItem(item.key, item.layer, ItemState.applied, how)
          : ProfileItem(item.key, item.layer, ItemState.unsupported, 'the agent offers no ${item.key} option with this value');
    }
    session.profile = ProfileReport(harness, [for (final item in launch.items) settled(item)]);
    _note(session, session.profile!.summary);
    _keepMode(session);
    notifyListeners();
  }

  /// Says in [session] that its agent, reached elsewhere, was given no profile.
  void noteNoProfile(ChatSession session) {
    _note(session, 'Canoa applies a profile only to agents it starts on this computer: this one runs elsewhere and was '
        'given none of it.');
    notifyListeners();
  }

  void _setCurrent(ChatSession session, String id, String value) {
    session.options = [
      for (final option in session.options)
        option.id == id
            ? ConfigOption(id: id, label: option.label, current: value, choices: option.choices, isMode: option.isMode,
                category: option.category)
            : option,
    ];
  }

  void _note(ChatSession session, String text) {
    session.messages.add(ChatMessage(role: MessageRole.notice, text: text, at: DateTime.now()));
  }

  void _trim(ChatSession session) {
    final over = session.messages.length - maxEntries;
    if (over <= 0) return;
    for (final gone in session.messages.take(over)) {
      if (gone.tool != null) session.toolEntries.remove(gone.tool!.id);
      if (gone.compaction != null) session.compactions.remove(gone.compaction!.id);
    }
    session.messages.removeRange(0, over);
  }

  void _applyTool(ChatSession session, Map<String, dynamic> raw) {
    final id = raw['toolCallId'];
    if (id is! String || id.isEmpty) return;
    var entry = session.toolEntries[id];
    if (entry == null) {
      entry = ChatMessage(role: MessageRole.tool, text: '', at: DateTime.now(), tool: ToolCall(id));
      session.toolEntries[id] = entry;
      session.messages.add(entry);
    }
    entry.tool!.apply(raw);
  }

  void _applyTerminal(ChatSession session, Map<String, dynamic> raw) {
    final id = raw['terminalId'];
    if (id is! String || id.isEmpty) return;
    if (session.terminals.length >= maxTerminals && !session.terminals.containsKey(id)) {
      session.terminals.remove(session.terminals.keys.first);
    }
    (session.terminals[id] ??= TerminalView()).apply(raw);
  }

  /// A compaction's entry stays where its first update put it, and later ones change it there.
  void _applyCompaction(ChatSession session, Map<String, dynamic> raw) {
    final id = raw['compactionId'];
    if (id is! String || id.isEmpty) return;
    var entry = session.compactions[id];
    if (entry == null) {
      entry = ChatMessage(role: MessageRole.compaction, text: '', at: DateTime.now(), compaction: Compaction(id));
      session.compactions[id] = entry;
      session.messages.add(entry);
    }
    entry.compaction!.apply(raw);
  }

  /// Names what went with a person's message, once per message.
  void _nameAttachments(ChatSession session, String? messageId, List<String> names) {
    if (names.isEmpty || (messageId != null && !session.attachmentsNamed.add(messageId))) return;
    _note(session, 'Sent with this: ${names.join(', ')}');
  }

  void _applyUpdate(ChatSession session, AcpSessionUpdate update) {
    final rendering = renderingOf(update);
    recorder?.record(update, rendering);
    switch (rendering) {
      case Rendering.toolCard:
        _applyTool(session, update.raw);
      case Rendering.terminal:
        _applyTerminal(session, update.raw);
      case Rendering.plan:
        final entries = update.kind == 'plan_update' ? planItems(update.raw['plan']) : update.raw['entries'];
        session.plan = parsePlan(entries) ?? session.plan;
      case Rendering.notice:
        final title = shortText(update.raw['title']);
        if (title != null) {
          // Any agent can send these, so each is shown as the agent's word, never as Canoa's.
          final from = switch (update.raw['severity']) {
            'warning' => 'Warning from the agent',
            'error' => 'Error from the agent',
            _ => 'From the agent',
          };
          final detail = shortText(update.raw['description']);
          _note(session, '$from: $title${detail == null ? '' : '\n$detail'}');
        }
      case Rendering.compaction:
        _applyCompaction(session, update.raw);
      case Rendering.commands:
        session.commands = parseCommands(update.raw['availableCommands']) ?? session.commands;
      case Rendering.usage:
        session.usage = session.usage.applied(update.raw);
      case Rendering.choices when update.kind == 'config_option_update':
        final before = modeOptionIn(session.options)?.current;
        _setOptions(session, update.raw['configOptions'], null);
        _rememberSource(session);
        _agentChangedMode(session, before);
      case Rendering.choices:
        if (update.raw['currentModeId'] case final String mode) {
          final before = modeOptionIn(session.options)?.current;
          for (final option in session.options) {
            if (option.isMode) _setCurrent(session, option.id, mode);
          }
          _agentChangedMode(session, before);
        }
      case Rendering.title:
        final title = shortText(update.raw['title']);
        if (title != null && session.agentNames) {
          session
            ..title = title
            ..titleFromMessage = false;
          if (session.peerId != null && session.remoteSessionId != null) {
            ledger?.touch(session.peerId!, session.remoteSessionId!, title: title);
          }
        }
      case Rendering.status:
        if (update.state == 'running' || update.state == 'requires_action') {
          session.status = SessionStatus.working;
        } else if (update.state == 'idle') {
          // A turn the person stopped is not news to them, and an idle state outside a turn ends nothing.
          final ended = session.status == SessionStatus.working;
          if (ended && update.reason != 'cancelled') _needs(session, AttentionKind.finished);
          session.status = SessionStatus.connected;
          if (ended) {
            _turnEnded(session, switch (update.reason) {
              'cancelled' => 'the turn was stopped',
              'error' || 'refusal' => 'the turn failed',
              _ => null,
            });
          }
          if (update.reason == 'cancelled') {
            _note(session, 'Stopped.');
          } else if (update.reason == 'error') {
            session.messages.add(ChatMessage(
              role: MessageRole.notice,
              text: 'Agent reported an operation failure.',
              at: DateTime.now(),
            ));
          }
          _keepText(session);
        }
      case Rendering.message || Rendering.thought:
        final chunk = update.kind.endsWith('_chunk');
        // A v2 message or thought without content patches its other fields and keeps its text; null content clears it.
        final text = !chunk && !update.raw.containsKey('content') ? null : update.text ?? '';
        final role = update.kind.startsWith('user')
            ? MessageRole.user
            : rendering == Rendering.thought ? MessageRole.thought : MessageRole.agent;
        if (text != null) {
          ChatMessage? existing;
          if (update.messageId != null) {
            for (final message in session.messages) {
              if (message.messageId == update.messageId) {
                existing = message;
                break;
              }
            }
          }
          // A v1 chunk carries no message id: it continues the last message of its role.
          if (existing == null && update.messageId == null && chunk &&
              session.messages.isNotEmpty && session.messages.last.role == role &&
              session.messages.last.messageId == null) {
            existing = session.messages.last;
          }
          if (existing != null) {
            existing.text = boundedHead(chunk ? existing.text + text : text, maxMessageText);
          } else if (text.isNotEmpty) {
            session.messages.add(ChatMessage(
              role: role, text: boundedHead(text, maxMessageText),
              at: DateTime.now(), messageId: update.messageId,
            ));
          }
        }
        // A replayed message carries its attachments as content blocks, which have no text of their own.
        if (role == MessageRole.user) _nameAttachments(session, update.messageId, attachedNames(update.raw['content']));
      case Rendering.unshown:
        break;
    }
    _trim(session);
    notifyListeners();
  }

  /// A v2 prompt response confirms insertion, not agent turn completion.
  /// The prompt is never retried automatically after a disconnect. Written while a turn runs, the message is queued instead,
  /// with what goes with it, and sent when the turn ends.
  Future<void> sendText(String text) async {
    final session = selected;
    if (session == null || (text.trim().isEmpty && session.attachments.isEmpty)) return;
    if (session.status == SessionStatus.preview) {
      sendPreviewText(text);
      return;
    }
    if (isStarting(session)) throw StateError('Its first message is still on its way');
    if (session.isDraft) {
      await _startThenDeliver(session, text);
      return;
    }
    if (session.status == SessionStatus.working || session.queue.isNotEmpty) {
      if (session.queue.length >= maxQueued) throw StateError('$maxQueued messages already wait for this turn to end');
      session.queue.add(QueuedMessage(text, List.of(session.attachments)));
      session.attachments.clear();
      notifyListeners();
      // A queue waiting on nothing, after a turn that ended as it should, goes on at once.
      if (session.status != SessionStatus.working && session.queueHeld == null) await sendQueued(session);
      return;
    }
    await _deliver(session, text, List.of(session.attachments));
  }

  /// A turn of [session] ended, stopped or failed when [held] says why. The next queued message goes, unless the turn did not
  /// end as it should: then the queue waits for the person, who may send it, edit it or clear it.
  void _turnEnded(ChatSession session, String? held) {
    if (session.queue.isEmpty) return;
    session.queueHeld = held;
    // After the update that ended the turn has been dispatched: an answer to the next prompt could otherwise arrive inside it.
    if (held == null) {
      scheduleMicrotask(() => unawaited(sendQueued(session).catchError((Object error) {
        _note(session, 'A queued message was not sent: $error');
        notifyListeners();
      })));
    }
  }

  /// Sends the first queued message of [session] now, and lets the queue go on after it.
  Future<void> sendQueued(ChatSession session) async {
    if (session.queue.isEmpty || session.status == SessionStatus.working) return;
    session.queueHeld = null;
    final next = session.queue.removeAt(0);
    notifyListeners();
    try {
      await _deliver(session, next.text, next.attachments);
    } catch (error) {
      // Not sent, so it stays first in the queue, which waits for the person.
      if (!session.queue.contains(next)) session.queue.insert(0, next);
      session.queueHeld = 'it could not be sent: ${reasonOf(error)}';
      notifyListeners();
      rethrow;
    }
  }

  /// Changes the text of a queued message.
  void editQueued(ChatSession session, QueuedMessage message, String text) {
    if (!session.queue.contains(message) || (text.trim().isEmpty && message.attachments.isEmpty)) return;
    message.text = text;
    notifyListeners();
  }

  /// Takes one queued message out, or every one.
  void removeQueued(ChatSession session, [QueuedMessage? message]) {
    if (message == null) {
      session.queue.clear();
    } else {
      session.queue.remove(message);
    }
    if (session.queue.isEmpty) session.queueHeld = null;
    notifyListeners();
  }

  /// [onSent] is told once the agent has the message, which for a v1 agent is before its turn ends.
  Future<void> _deliver(ChatSession session, String text, List<Attachment> attached, {void Function()? onSent}) async {
    final client = _verifiedClients[session.id];
    if (client == null || client.isClosed || session.remoteSessionId == null) {
      throw StateError('A verified ACP peer must be connected');
    }
    // Each is checked before anything is sent: one the agent cannot be given stops the message, which stays to be changed.
    final takes = takesOf(session)!;
    final attachments = <AcpAttachment>[
      for (final attachment in attached)
        await toAcp(attachment, takes, embedLimit: embedLimit, read: readFile, agent: _named(session.agent)),
    ];
    await _applyPending(session);
    if (session.peerId != null) ledger?.touch(session.peerId!, session.remoteSessionId!);
    final sent = attachments.isEmpty ? null : 'Sent with this: ${attachments.map((item) => item.name).join(', ')}';
    if (client.protocolVersion == 1) {
      // A v1 answer arrives when the whole turn ends, so the message shows now.
      if (text.trim().isNotEmpty) session.messages.add(ChatMessage(role: MessageRole.user, text: text, at: DateTime.now()));
      if (sent != null) _note(session, sent);
      session.attachments.removeWhere(attached.contains);
      session.status = SessionStatus.working;
      session.draftText = '';
      final turn = client.prompt(session.remoteSessionId!, text, attachments: attachments);
      onSent?.call();
      notifyListeners();
      await turn;
      return;
    }
    final messageId = await client.prompt(session.remoteSessionId!, text, attachments: attachments);
    onSent?.call();
    session.attachments.removeWhere(attached.contains);
    if (text.trim().isNotEmpty && !session.messages.any((message) => message.messageId == messageId)) {
      session.messages.add(ChatMessage(
        role: MessageRole.user,
        text: text, at: DateTime.now(), messageId: messageId,
      ));
    }
    _nameAttachments(session, messageId, [for (final item in attachments) item.name]);
    session.draftText = '';
    notifyListeners();
  }

  /// Whether anything can go with this session's messages: a draft's, or one whose agent this app reaches.
  bool canAttach(ChatSession session) {
    if (session.isDraft) return true;
    final client = _verifiedClients[session.id];
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

  /// Takes the file at [path] on this computer, [size] bytes, read only when it is sent: embedded where the agent takes files
  /// and it is small enough text, else linked.
  String? addFile(ChatSession session, String name, String path, int size, {String? mimeType}) {
    if (size == 0) return '$name is empty.';
    return _add(session, Attachment.file(name, path, size, mimeType: mimeType));
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

  /// Whether a person can ask this session's agent for its earlier conversations.
  bool canListEarlier(ChatSession session) {
    final client = _verifiedClients[session.id];
    return client != null && !client.isClosed && client.canReopenSessions;
  }

  /// The agent's other conversations, where this one is working when that is known. It lists them, and they are its own to name.
  Future<AcpSessionPage> earlierSessions(ChatSession from, {String? cursor}) async {
    final client = _verifiedClients[from.id];
    if (client == null || client.isClosed) throw StateError('A verified ACP peer must be connected');
    final page = await client.listSessions(cwd: from.cwd, cursor: cursor);
    return AcpSessionPage(
      page.sessions.where((session) => session.id != from.remoteSessionId).toList(),
      page.nextCursor,
    );
  }

  /// Opens an earlier conversation beside this one, on the same agent. One already open is selected, not opened twice.
  Future<ChatSession> openEarlier(ChatSession from, AcpSessionInfo earlier) async {
    final client = _verifiedClients[from.id];
    final cwd = earlier.cwd ?? from.cwd;
    if (client == null || client.isClosed) throw StateError('A verified ACP peer must be connected');
    if (cwd == null) throw StateError('The agent did not say where that conversation was');
    for (final session in _sessions) {
      if (session.remoteSessionId == earlier.id && _verifiedClients[session.id] == client) {
        select(session.id);
        return session;
      }
    }
    final session = addProviderSession(
      title: shortText(earlier.title) ?? 'Earlier conversation',
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
      final opened = await client.reopenSession(earlier.id, cwd: cwd, setup: from.setup);
      applyOpened(session, opened);
      if (from.profile case final profile?) _note(session, profile.summary);
      if (!opened.replayed) _note(session, 'The agent resumed this without sending what was said before.');
      session.status = SessionStatus.connected;
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
  /// With [launch], the session is opened with its setup and [prepared]'s profile is applied as [applyProfile] does.
  Future<ChatSession> reopen(LedgerEntry entry, VerifiedAgent client,
      {ProfileLaunch? launch, PreparedProfile? prepared, String? harness, List<LayerNote> notes = const []}) async {
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
          if (info.id == entry.sessionId) title = shortText(info.title) ?? title;
        }
      }
    } on AcpFailure {
      // Only the name depends on it.
    }
    final endpoint = entry.endpoint == null ? null : Uri.tryParse(entry.endpoint!);
    final session = _insert(ChatSession(
      id: 'local-${++_nextId}',
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
    ))
      ..bypass = entry.bypass
      ..chosenMode = entry.mode
      ..opening = true;
    attachVerifiedClient(session.id, client, record: false);
    try {
      final opened = await client.reopenSession(entry.sessionId, cwd: cwd, setup: launch?.setup ?? AcpSessionSetup.none);
      applyOpened(session, opened);
      if (launch != null && prepared != null) {
        await applyProfile(session, launch, prepared,
            harness: harness ?? entry.harness, notes: notes, keepOwnMode: entry.mode != null);
      }
      // The mode the session was left in goes back, once, before anything is sent: it wins over the profile's. Under Canoa's
      // own Bypass it is the agent's mode that Bypass sat on.
      if (entry.mode case final mode?) {
        final id = modeOptionIn(session.options)?.id ?? OptionRole.mode.name;
        session.pending[id] = PendingChoice(id, OptionRole.mode, mode);
      }
      await _applyPending(session);
      session.opening = false;
      // A change made while it was being opened, such as leaving Bypass, is kept now.
      _keepMode(session);
      if (!opened.replayed) {
        _note(session, 'The agent resumed this without sending what was said before.');
        if (entry.messages.isNotEmpty) {
          _note(session, 'What Canoa kept of it follows, from its own list, not from the agent.');
          for (final kept in entry.messages) {
            session.messages.add(ChatMessage(role: _keptRole(kept.role), text: kept.text, at: kept.at));
          }
        }
      }
      session.status = SessionStatus.connected;
      _record(session, client);
      notifyListeners();
      return session;
    } catch (_) {
      session.opening = false;
      forget(session);
      rethrow;
    }
  }

  static MessageRole _keptRole(String role) => role == 'user' ? MessageRole.user : MessageRole.agent;

  /// Shows what Canoa kept of a session it cannot open again, as an offline preview that sends nothing.
  ChatSession showKept(LedgerEntry entry) {
    final session = createPreviewSession(title: entry.title ?? 'Earlier conversation', agent: entry.harness);
    _note(session, 'What Canoa kept of this session. ${entry.whyNotReopened ?? ''} Nothing typed here reaches an agent.');
    for (final kept in entry.messages) {
      session.messages.add(ChatMessage(role: _keptRole(kept.role), text: kept.text, at: kept.at));
    }
    notifyListeners();
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
    unawaited(_subscriptions.remove(session.id)?.cancel());
    unawaited(_turnSubscriptions.remove(session.id)?.cancel());
    _verifiedClients.remove(session.id);
    _sessions.remove(session);
    if (_selectedId == session.id) _selectedId = _sessions.isEmpty ? null : _sessions.first.id;
    notifyListeners();
  }

  /// Ends a conversation here: a question still open is answered cancelled, and its agent ends once no other conversation here
  /// uses it. What the agent saved stays with the agent, for Earlier conversations to open again.
  Future<void> close(ChatSession session) async {
    final client = _verifiedClients[session.id];
    _endWaiting(session, const AcpCancel());
    forget(session);
    if (client != null && !_verifiedClients.values.contains(client)) await client.close();
  }

  Future<void> cancelActiveSession() async {
    final session = selected;
    final client = session == null ? null : _verifiedClients[session.id];
    if (session == null || session.remoteSessionId == null || client == null) {
      throw StateError('No active verified ACP session');
    }
    // ACP has a request still open at cancellation answered as cancelled, and the agent's turn cannot end until it is.
    _endWaiting(session, const AcpCancel());
    notifyListeners();
    await client.cancel(session.remoteSessionId!);
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
      session.messages.add(ChatMessage(
        role: MessageRole.notice,
        text: 'Attached page context (not sent): ${title.isEmpty ? uri.host : title}\\n$uri$suffix',
        at: DateTime.now(),
      ));
      notifyListeners();
    } on FormatException {
      // Ignore malformed extension messages.
    }
  }

  void select(String id) {
    final chosen = _sessions.where((session) => session.id == id).firstOrNull;
    if (chosen == null) return;
    _selectedId = id;
    chosen.unread = false;
    notifyListeners();
  }

  /// This does not send data to an agent. Only the clearly labeled preview
  /// session accepts text until the authenticated ACP transport is connected.
  void sendPreviewText(String text) {
    final session = selected;
    final trimmed = text.trim();
    if (session == null || trimmed.isEmpty) return;
    if (session.status != SessionStatus.preview) {
      throw StateError('A verified ACP transport is required to send');
    }
    session.messages.add(ChatMessage(
      role: MessageRole.user,
      text: trimmed,
      at: DateTime.now(),
    ));
    session.messages.add(ChatMessage(
      role: MessageRole.notice,
      text: 'Preview only — this message was not sent to an agent.',
      at: DateTime.now(),
    ));
    notifyListeners();
  }
  @override
  void dispose() {
    for (final subscription in _subscriptions.values) {
      unawaited(subscription.cancel());
    }
    _subscriptions.clear();
    _verifiedClients.clear();
    unawaited(_attention.close());
    super.dispose();
  }

}
