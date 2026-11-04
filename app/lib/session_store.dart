import 'dart:async';
import 'acp/client.dart';
import 'package:coqui_web/coqui_web.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'chat/agent_events.dart';
import 'chat/update_record.dart';
import 'ledger/session_ledger.dart';
import 'profile/apply.dart';
import 'profile/profile_layers.dart' show LayerNote;

export 'ledger/session_ledger.dart' show SessionPlace;

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
enum SessionStatus { preview, disconnected, connecting, connected, working }

enum MessageRole { user, agent, notice, thought, tool, compaction }

/// The most a conversation keeps. An agent's turn can run a very long time, and the oldest entries are what go.
const int maxEntries = 5000;

/// The most one message keeps, since an agent can send any amount.
const int maxMessageText = 2 * 1024 * 1024;

/// The most questions one conversation holds waiting for the person; an agent asking more has the rest answered cancelled.
const int maxPendingPermissions = 64;

/// The most that goes with one message. An agent's context is what these fill, so they stay what a person would pick by hand.
const int maxAttachments = 8;
const int maxAttachedText = 1024 * 1024;
const int maxAttachedImage = 5 * 1024 * 1024;

/// An image's type, from what it was chosen as or its name, or null for anything else.
String? imageTypeOf(String name, String? mimeType) {
  if (const {'image/png', 'image/jpeg', 'image/gif', 'image/webp'}.contains(mimeType)) return mimeType;
  final dot = name.lastIndexOf('.');
  return switch (dot < 0 ? '' : name.substring(dot + 1).toLowerCase()) {
    'png' => 'image/png',
    'jpg' || 'jpeg' => 'image/jpeg',
    'gif' => 'image/gif',
    'webp' => 'image/webp',
    _ => null,
  };
}

class ChatMessage {
  ChatMessage({required this.role, required this.text, required this.at, this.messageId, this.tool, this.compaction});

  final MessageRole role;
  String text;
  final DateTime at;
  final String? messageId;
  final ToolCall? tool;
  final Compaction? compaction;
}

/// A tool an agent wants to use, waiting on the person. Nothing answers it for them: full access answers a request before it
/// would wait here, never one already waiting.
final class PermissionQuestion {
  PermissionQuestion(this.title, this.kind, {this.detail, this.options = const [], this.fullAccessPassed = false});
  final String title;
  final String? kind;

  /// What it would run or touch, when the agent said.
  final String? detail;

  /// The answers the agent offers, in its order.
  final List<AcpPermissionOption> options;

  /// Whether full access was on and left this to the person, since the agent offered no option to allow once.
  final bool fullAccessPassed;
  final Completer<AcpDecision> _answer = Completer();
}

/// The most questions one conversation holds waiting for the person; an agent asking more has the rest answered cancelled.
const int maxPendingQuestions = 16;

/// An agent's question, waiting on the person. Nothing answers it for them; full access is for permission requests only.
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
  final List<AcpAttachment> attachments;
}

/// What a session needs the person for: a permission request, a question, or a turn it finished.
enum AttentionKind { permission, question, finished }

/// Where a session's full access comes from: turned on for it, or for its agent in Canoa.cfg.
enum FullAccess { session, agent }

/// The option full access answers with: the first that allows once. An `allow_always` is never chosen for the person, since the
/// agent would remember it after full access ends.
AcpPermissionOption? fullAccessOption(List<AcpPermissionOption> options) =>
    options.where((option) => option.kind == 'allow_once').firstOrNull;

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
    List<ChatMessage>? messages,
  }) : messages = messages ?? <ChatMessage>[];

  /// Where the agent works, on its machine, when this side knows.
  final String? cwd;

  /// Where the agent runs, as this app reached it.
  final SessionPlace place;

  /// The proof a peer's key makes, for a session reached by its endpoint.
  final String? method;

  /// What goes with the next message.
  final List<AcpAttachment> attachments = [];

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

  /// Whether this session's permission requests are answered without asking, and why; null when every one is asked. Off unless
  /// the person turned it on, for this session or for its agent.
  FullAccess? fullAccess;

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
  final String agent;
  SessionStatus status;
  final String? peerId;
  final String? remoteSessionId;
  /// Preferred HTTPS WebTransport or direct WSS endpoint.
  final Uri? endpoint;
  final Uri? websocketFallback;
  final List<ChatMessage> messages;
}

/// Shared UI state for desktop, PWA and packaged Chromium extension.
/// Transport, verified peers and persistence are intentionally outside it.
class SessionStore extends ChangeNotifier {
  SessionStore({bool includePreview = true, this.recorder, this.ledger, Set<String> fullAccessAgents = const {}, this.onWeakened})
      : _fullAccessAgents = {...fullAccessAgents} {
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

  /// Told each weakening as it happens, beside [weakenings]; by default the debug log.
  final void Function(String)? onWeakened;

  /// The most [weakenings] keeps, the oldest going first.
  static const maxWeakenings = 200;

  /// Each time something here was made to ask less than it does by default, in the words of a `weakened:` log line.
  final List<String> weakenings = [];

  final Set<String> _fullAccessAgents;

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

  /// The agents, by label, whose sessions start with full access.
  Set<String> get fullAccessAgents => Set.unmodifiable(_fullAccessAgents);

  void _weakened(String what) {
    final line = 'weakened: $what';
    weakenings.add(line);
    if (weakenings.length > maxWeakenings) weakenings.removeAt(0);
    (onWeakened ?? debugPrint)(line);
  }

  /// Turns full access on or off for [session]. On, each permission request it makes that offers to allow once is answered with
  /// that option without asking, which is logged as a weakening and said in the conversation; a request already waiting stays
  /// for the person. Off, every request is asked again.
  void setFullAccess(ChatSession session, bool on) {
    if (on == (session.fullAccess != null)) return;
    if (on) {
      session.fullAccess = FullAccess.session;
      _weakened('full access is on for the conversation "${_named(session.title)}" with ${_named(session.agent)}: it allows what '
          'it can without asking, looser than asking each time');
      _note(session, 'Full access is on for this conversation: each request that offers to allow once is allowed without '
          'asking you.');
    } else {
      session.fullAccess = null;
      _note(session, 'Full access is off: every request is asked again.');
    }
    notifyListeners();
  }

  /// Turns full access on or off for [agent], by the label of a program this app runs: its sessions open now and those it opens
  /// later, until it is turned off. A session reached any other way, whatever its agent calls itself, is not the program's.
  void setAgentFullAccess(String agent, bool on) {
    if (on ? !_fullAccessAgents.add(agent) : !_fullAccessAgents.remove(agent)) return;
    if (on) {
      _weakened('full access is on for the agent ${_named(agent)}: its sessions allow what they can without asking, looser than '
          'asking each time');
    }
    for (final session in _sessions) {
      if (session.agent != agent || !_runsAs(session, agent)) continue;
      if (on && session.fullAccess == null) {
        session.fullAccess = FullAccess.agent;
        _note(session, 'Full access is on for ${_named(agent)}: each request that offers to allow once is allowed without '
            'asking you.');
      } else if (!on && session.fullAccess == FullAccess.agent) {
        session.fullAccess = null;
        _note(session, 'Full access is off for ${_named(agent)}: every request is asked again.');
      }
    }
    notifyListeners();
  }

  /// Whether [session] is served by the program this app runs as [agent].
  bool _runsAs(ChatSession session, String agent) {
    final client = _verifiedClients[session.id];
    return client != null && client.isChild && client.peerId == 'local:$agent';
  }

  /// A name an agent or a person gave, as it is shown and logged: one line, with nothing that could reorder what surrounds it.
  static String _named(String name) => shortText(name) ?? 'an unnamed one';

  /// A session just given its agent, which starts with full access when that agent's setting says so.
  void _startFullAccess(ChatSession session) {
    if (session.fullAccess != null || !_fullAccessAgents.contains(session.agent) || !_runsAs(session, session.agent)) return;
    session.fullAccess = FullAccess.agent;
    _weakened('full access is on for the conversation "${_named(session.title)}" with ${_named(session.agent)}, as Canoa.cfg says '
        'for that agent: it allows what it can without asking, looser than asking each time');
    _note(session, 'Full access is on for ${_named(session.agent)}: each request that offers to allow once is allowed without '
        'asking you.');
  }

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
    if (target.fullAccess != null) {
      final allow = fullAccessOption(request.options);
      if (allow != null) {
        _note(target, 'Full access allowed: $title (${shortText(allow.name) ?? 'allow once'}).');
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
      fullAccessPassed: target.fullAccess != null,
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
  }) {
    // This records transport locations, not the authentication state. The
    // session stays disconnected until peer proof and ACP are implemented.
    if (endpoint.scheme == 'https') {
      WebEndpoints(webTransport: endpoint, webSocket: websocketFallback, allowLocalPlainWebSocket: true);
    } else if (endpoint.scheme == 'wss' || endpoint.scheme == 'ws') {
      if (websocketFallback != null) {
        throw ArgumentError('A direct WSS endpoint cannot have a fallback');
      }
      // Plain WebSocket only on loopback, where canoa-host listens.
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
    _startFullAccess(session);
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
      title: session.title,
      endpoint: session.endpoint?.toString(),
      fallback: session.websocketFallback?.toString(),
      method: session.method,
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

  /// The `_meta` key under which `canoa-host` lists, in its answer to `session/new`, what a person should know about an agent
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

  /// Sets one of the agent's choices. It stays as it was unless the agent agrees, which it says in its answer or an update.
  Future<void> choose(ChatSession session, ConfigOption option, String value) async {
    final client = _verifiedClients[session.id];
    final remote = session.remoteSessionId;
    if (client == null || remote == null) return;
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
      _note(session, 'The agent did not change ${option.label}: ${error.message}');
    }
    notifyListeners();
  }

  /// Finishes applying a profile to [session], just opened with [launch]'s setup: the model, effort and mode are set where
  /// the agent offers an option with that value, and the report goes on the session and, in one line, into it. [notes]
  /// are what the person was not shown of the layers, such as one left out, and go into it too.
  Future<void> applyProfile(ChatSession session, ProfileLaunch launch, PreparedProfile prepared,
      {required String harness, List<LayerNote> notes = const []}) async {
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
      final option = session.options.firstWhere((option) => option.id == id);
      await choose(session, option, value);
      final now = session.options.where((option) => option.id == id).firstOrNull;
      if (now?.current == value) set[key] = option.isMode ? 'session/set_mode' : 'session/set_config_option $id';
    }
    ProfileItem settled(ProfileItem item) {
      if (!const {'model', 'effort', 'mode'}.contains(item.key)) return item;
      final how = set[item.key];
      return how != null
          ? ProfileItem(item.key, item.layer, ItemState.applied, how)
          : ProfileItem(item.key, item.layer, ItemState.unsupported, 'the agent offers no ${item.key} option with this value');
    }
    session.profile = ProfileReport(harness, [for (final item in launch.items) settled(item)]);
    _note(session, session.profile!.summary);
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
        _setOptions(session, update.raw['configOptions'], null);
      case Rendering.choices:
        if (update.raw['currentModeId'] case final String mode) {
          for (final option in session.options) {
            if (option.isMode) _setCurrent(session, option.id, mode);
          }
        }
      case Rendering.title:
        final title = shortText(update.raw['title']);
        if (title != null && session.agentNames) {
          session.title = title;
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
    await _deliver(session, next.text, next.attachments);
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

  Future<void> _deliver(ChatSession session, String text, List<AcpAttachment> attachments) async {
    final client = _verifiedClients[session.id];
    if (client == null || client.isClosed || session.remoteSessionId == null) {
      throw StateError('A verified ACP peer must be connected');
    }
    if (session.peerId != null) ledger?.touch(session.peerId!, session.remoteSessionId!);
    final sent = attachments.isEmpty ? null : 'Sent with this: ${attachments.map((item) => item.name).join(', ')}';
    if (client.protocolVersion == 1) {
      // A v1 answer arrives when the whole turn ends, so the message shows now.
      if (text.trim().isNotEmpty) session.messages.add(ChatMessage(role: MessageRole.user, text: text, at: DateTime.now()));
      if (sent != null) _note(session, sent);
      session.attachments.removeWhere(attachments.contains);
      session.status = SessionStatus.working;
      notifyListeners();
      await client.prompt(session.remoteSessionId!, text, attachments: attachments);
      return;
    }
    final messageId = await client.prompt(session.remoteSessionId!, text, attachments: attachments);
    session.attachments.removeWhere(attachments.contains);
    if (text.trim().isNotEmpty && !session.messages.any((message) => message.messageId == messageId)) {
      session.messages.add(ChatMessage(
        role: MessageRole.user,
        text: text, at: DateTime.now(), messageId: messageId,
      ));
    }
    _nameAttachments(session, messageId, [for (final item in attachments) item.name]);
    notifyListeners();
  }

  /// Whether this session's agent takes anything with a message.
  bool canAttach(ChatSession session) {
    final client = _verifiedClients[session.id];
    return client != null && !client.isClosed && (client.takesImages || client.takesFiles);
  }

  /// Takes a file a person chose, to go with their next message. What is wrong with it, for them to read, or null.
  String? addAttachment(ChatSession session, String name, List<int> bytes, {String? mimeType}) {
    final client = _verifiedClients[session.id];
    if (client == null || client.isClosed) return 'This conversation has no agent.';
    if (session.attachments.length >= maxAttachments) return 'That is as many as one message can take.';
    if (session.attachments.any((item) => item.name == name)) return '$name is already there.';
    if (bytes.isEmpty) return '$name is empty.';
    final image = imageTypeOf(name, mimeType);
    if (image != null) {
      if (!client.takesImages) return '${session.agent} does not take images.';
      if (bytes.length > maxAttachedImage) return '$name is larger than an image can be here.';
      session.attachments.add(AcpAttachment.image(name, image, bytes));
    } else {
      if (!client.takesFiles) return '${session.agent} does not take files.';
      if (bytes.length > maxAttachedText) return '$name is larger than a file can be here.';
      final String text;
      try {
        text = utf8.decode(bytes);
      } on FormatException {
        return '$name is not text, and only text files and images can be sent.';
      }
      session.attachments.add(AcpAttachment.text(name, text, mimeType: mimeType));
    }
    notifyListeners();
    return null;
  }

  void removeAttachment(ChatSession session, AcpAttachment attachment) {
    session.attachments.remove(attachment);
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
    ));
    attachVerifiedClient(session.id, client, record: false);
    try {
      final opened = await client.reopenSession(entry.sessionId, cwd: cwd, setup: launch?.setup ?? AcpSessionSetup.none);
      applyOpened(session, opened);
      if (launch != null && prepared != null) {
        await applyProfile(session, launch, prepared, harness: harness ?? entry.harness, notes: notes);
      }
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
