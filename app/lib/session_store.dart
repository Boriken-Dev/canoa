import 'dart:convert';
import 'package:flutter/foundation.dart';

/// Canoa's local view of an agent-issued session. The agent session id is
/// separate from the local view id and is absent from offline previews.
enum SessionStatus { preview, disconnected, connecting, connected, working }

enum MessageRole { user, agent, notice }

@immutable
class ChatMessage {
  const ChatMessage({required this.role, required this.text, required this.at});

  final MessageRole role;
  final String text;
  final DateTime at;
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
    List<ChatMessage>? messages,
  }) : messages = messages ?? <ChatMessage>[];

  final String id;
  String title;
  final String agent;
  SessionStatus status;
  final String? peerId;
  final String? remoteSessionId;
  final Uri? endpoint;
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

  /// A verified transport may attach an existing remote session. This method
  /// records its identity but does not establish trust or connect to a peer.
  ChatSession addRemoteSession({
    required String title,
    required String agent,
    required String peerId,
    required String remoteSessionId,
    required Uri endpoint,
  }) {
    if (endpoint.scheme != 'wss') {
      throw ArgumentError.value(endpoint, 'endpoint', 'Remote sessions require WSS');
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
      status: SessionStatus.disconnected,
    );
    _sessions.insert(0, session);
    _selectedId = session.id;
    notifyListeners();
    return session;
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
}
