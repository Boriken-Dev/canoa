import 'dart:async';
import 'acp/client.dart';
import 'package:coqui_web/coqui_web.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'chat/agent_events.dart';

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
enum SessionStatus { preview, disconnected, connecting, connected, working }

enum MessageRole { user, agent, notice, thought, tool }

/// The most a conversation keeps. An agent's turn can run a very long time, and the oldest entries are what go.
const int maxEntries = 5000;

/// The most one message keeps, since an agent can send any amount.
const int maxMessageText = 2 * 1024 * 1024;

class ChatMessage {
  ChatMessage({required this.role, required this.text, required this.at, this.messageId, this.tool});

  final MessageRole role;
  String text;
  final DateTime at;
  final String? messageId;
  final ToolCall? tool;
}

/// A tool an agent wants to use, waiting on the person. Nothing answers it for them.
final class PermissionQuestion {
  PermissionQuestion(this.title, this.kind, {this.detail});
  final String title;
  final String? kind;

  /// What it would run or touch, when the agent said.
  final String? detail;
  final Completer<AcpDecision> _answer = Completer();
}

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
    List<ChatMessage>? messages,
  }) : messages = messages ?? <ChatMessage>[];

  /// What the agent is waiting for the person to answer, oldest first.
  final List<PermissionQuestion> pendingPermissions = [];

  /// What the agent lets a person choose, its plan, its commands and what it has used, each as it last said.
  List<ConfigOption> options = const [];
  List<PlanEntry> plan = const [];
  List<AgentCommandInfo> commands = const [];
  SessionUsage usage = const SessionUsage();

  /// Commands the agent ran, which a tool's card shows.
  final Map<String, TerminalView> terminals = {};
  final Map<String, ChatMessage> toolEntries = {};

  /// Whether the agent may name the conversation, which it may until the person has.
  bool agentNames = false;

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
  SessionStore({bool includePreview = true}) {
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

  /// Puts a permission request to the person, in the conversation it belongs to. A request about no session here is refused.
  Future<AcpDecision> askPermission(AcpPermissionRequest request) {
    ChatSession? target;
    for (final session in _sessions) {
      if (session.remoteSessionId == request.sessionId) target = session;
    }
    if (target == null) return Future.value(const AcpDeny('not a session here'));
    // ACP has the tool call applied before the prompt shows, so the conversation shows what is being asked about.
    if (request.toolCall != null) _applyTool(target, request.toolCall!);
    final question = PermissionQuestion(
      shortText(request.title) ?? shortText(request.kind) ?? 'A tool',
      request.kind,
      detail: [
        ?shortText(request.description),
        ?describeToolInput(request.rawInput),
      ].join('\n').trim(),
    );
    target.pendingPermissions.add(question);
    notifyListeners();
    return question._answer.future;
  }

  /// The person's answer to the oldest question in [session].
  void answerPermission(ChatSession session, {required bool allow}) {
    if (session.pendingPermissions.isEmpty) return;
    final question = session.pendingPermissions.removeAt(0);
    question._answer.complete(allow ? const AcpAllow() : const AcpDeny('the person refused'));
    notifyListeners();
  }

  void _endWaiting(ChatSession session, AcpDecision answer) {
    for (final question in session.pendingPermissions) {
      question._answer.complete(answer);
    }
    session.pendingPermissions.clear();
  }

  /// A session over a browser provider's connection, which has no endpoint of Canoa's own.
  ChatSession addProviderSession({
    required String title,
    required String agent,
    required String peerId,
    required String remoteSessionId,
  }) {
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    final session = ChatSession(
      id: 'local-${++_nextId}',
      title: title,
      agent: agent,
      status: SessionStatus.disconnected,
      peerId: peerId,
      remoteSessionId: remoteSessionId,
    );
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
  }) {
    // This records transport locations, not the authentication state. The
    // session stays disconnected until peer proof and ACP are implemented.
    if (endpoint.scheme == 'https') {
      WebEndpoints(webTransport: endpoint, webSocket: websocketFallback);
    } else if (endpoint.scheme == 'wss') {
      if (websocketFallback != null) {
        throw ArgumentError('A direct WSS endpoint cannot have a fallback');
      }
      WebEndpoints(webSocket: endpoint);
    } else {
      throw ArgumentError.value(endpoint, 'endpoint', 'Use HTTPS WebTransport or WSS');
    }
    if (peerId.isEmpty || remoteSessionId.isEmpty) {
      throw ArgumentError('Peer identity and agent session ID are required');
    }
    final session = ChatSession(
      id: 'local-${++_nextId}',
      title: title,
      agent: agent,
      peerId: peerId,
      remoteSessionId: remoteSessionId,
      endpoint: endpoint,
      websocketFallback: websocketFallback,
      status: SessionStatus.disconnected,
    );
    _sessions.insert(0, session);
    _selectedId = session.id;
    notifyListeners();
    return session;
  }

  /// A verified and initialized ACP client can be bound to one session.
  /// Opening and peer authentication remain the connector's responsibility.
  void attachVerifiedClient(String localId, VerifiedAgent client) {
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
      if (client.protocolVersion != 1) return;
      if (event is AcpCompleted || event is AcpBlocked) {
        session.status = SessionStatus.connected;
        notifyListeners();
      } else if (event is AcpFailed) {
        session.status = SessionStatus.connected;
        session.messages.add(ChatMessage(role: MessageRole.notice, text: event.message, at: DateTime.now()));
        notifyListeners();
      }
    });
    notifyListeners();
  }

  /// What the agent said when it opened [session].
  void applyOpened(ChatSession session, AcpOpenedSession opened) {
    _setOptions(session, opened.raw['configOptions'], opened.raw['modes']);
    session.commands = parseCommands(opened.raw['availableCommands']) ?? session.commands;
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

  void _setCurrent(ChatSession session, String id, String value) {
    session.options = [
      for (final option in session.options)
        option.id == id
            ? ConfigOption(id: id, label: option.label, current: value, choices: option.choices, isMode: option.isMode)
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

  void _applyUpdate(ChatSession session, AcpSessionUpdate update) {
    switch (update.kind) {
      case 'tool_call' || 'tool_call_update':
        _applyTool(session, update.raw);
      case 'terminal_update':
        _applyTerminal(session, update.raw);
      case 'plan':
        session.plan = parsePlan(update.raw['entries']) ?? session.plan;
      case 'available_commands_update':
        session.commands = parseCommands(update.raw['availableCommands']) ?? session.commands;
      case 'usage_update':
        session.usage = session.usage.applied(update.raw);
      case 'config_option_update':
        _setOptions(session, update.raw['configOptions'], null);
      case 'current_mode_update':
        if (update.raw['currentModeId'] case final String mode) {
          for (final option in session.options) {
            if (option.isMode) _setCurrent(session, option.id, mode);
          }
        }
      case 'session_info_update':
        final title = shortText(update.raw['title']);
        if (title != null && session.agentNames) session.title = title;
    }
    if (update.kind == 'state_update') {
      if (update.state == 'running' || update.state == 'requires_action') {
        session.status = SessionStatus.working;
      } else if (update.state == 'idle') {
        session.status = SessionStatus.connected;
        if (update.reason == 'cancelled') {
          _note(session, 'Stopped.');
        } else if (update.reason == 'error') {
          session.messages.add(ChatMessage(
            role: MessageRole.notice,
            text: 'Agent reported an operation failure.',
            at: DateTime.now(),
          ));
        }
      }
    } else if (const [
      'user_message', 'agent_message', 'user_message_chunk',
      'agent_message_chunk', 'agent_thought_chunk', 'agent_thought',
    ].contains(update.kind)) {
      final text = update.text;
      if (text != null) {
        final role = update.kind.startsWith('user')
            ? MessageRole.user
            : update.kind.startsWith('agent_thought') ? MessageRole.thought : MessageRole.agent;
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
        if (existing == null && update.messageId == null && update.kind.endsWith('_chunk') &&
            session.messages.isNotEmpty && session.messages.last.role == role &&
            session.messages.last.messageId == null) {
          existing = session.messages.last;
        }
        if (existing != null) {
          existing.text = boundedHead(update.kind.endsWith('_chunk')
              ? existing.text + text : text, maxMessageText);
        } else {
          session.messages.add(ChatMessage(
            role: role, text: boundedHead(text, maxMessageText),
            at: DateTime.now(), messageId: update.messageId,
          ));
        }
      }
    }
    _trim(session);
    notifyListeners();
  }

  /// A v2 prompt response confirms insertion, not agent turn completion.
  /// The prompt is never retried automatically after a disconnect.
  Future<void> sendText(String text) async {
    final session = selected;
    if (session == null || text.trim().isEmpty) return;
    if (session.status == SessionStatus.preview) {
      sendPreviewText(text);
      return;
    }
    final client = _verifiedClients[session.id];
    if (client == null || client.isClosed || session.remoteSessionId == null) {
      throw StateError('A verified ACP peer must be connected');
    }
    if (client.protocolVersion == 1) {
      // A v1 answer arrives when the whole turn ends, so the message shows now.
      session.messages.add(ChatMessage(role: MessageRole.user, text: text, at: DateTime.now()));
      session.status = SessionStatus.working;
      notifyListeners();
      await client.prompt(session.remoteSessionId!, text);
      return;
    }
    final messageId = await client.prompt(session.remoteSessionId!, text);
    if (!session.messages.any((message) => message.messageId == messageId)) {
      session.messages.add(ChatMessage(
        role: MessageRole.user,
        text: text, at: DateTime.now(), messageId: messageId,
      ));
      notifyListeners();
    }
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
    if (!_sessions.any((session) => session.id == id)) return;
    _selectedId = id;
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
    super.dispose();
  }

}
