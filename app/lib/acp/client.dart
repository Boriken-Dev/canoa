import 'dart:convert';

import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_acp/page.dart';
import 'package:coqui_web/coqui_web.dart';

export 'package:coqui_acp/coqui_acp.dart';

/// A proven peer, and the channel that keeps everything after the proof with that peer.
final class VerifiedPeer {
  const VerifiedPeer(this.peerId, this.channel);
  final String peerId;
  final WebChannel channel;
}

/// Must cryptographically verify the remote peer, not trust a supplied ID, and answer with a channel whose messages
/// are tied to that proof: a plain channel after a proof belongs to whoever carries it.
typedef VerifyAcpPeer = Future<VerifiedPeer> Function(WebChannel channel, String expectedPeerId);

/// What an agent's `initialize` answer says about the agent itself, read from the answer as it passes, since the shared client
/// keeps only the version and capabilities. All of it is the agent's to say.
final class AgentIdentity {
  /// `agentInfo` in v1, `info` in v2; null when the agent sent none.
  String? name, title, version;

  /// Grok's ACP sends no `agentInfo`, only `_meta.grokShell` and `_meta.agentVersion`.
  bool grokShell = false;

  /// The method that opens an earlier session again, as the client would pick it, or null when the agent offers none.
  String? reopenBy;

  bool _seen = false;

  /// Whether the answer to `initialize` has passed.
  bool get seen => _seen;

  /// Bounded, since an agent can send any amount.
  static String? _text(Object? value) =>
      value is String && value.trim().isNotEmpty ? (value.length > 200 ? value.substring(0, 200) : value.trim()) : null;

  String _inspect(String message) {
    // Only the first answer naming a protocol version is read; every other message passes untouched.
    if (_seen || !message.contains('protocolVersion')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['result'] is! Map) return message;
    final result = decoded['result'] as Map;
    final version = result['protocolVersion'];
    if (version != 1 && version != 2) return message;
    _seen = true;
    final info = result[version == 2 ? 'info' : 'agentInfo'];
    if (info is Map) {
      name = _text(info['name']);
      title = _text(info['title']);
      this.version = _text(info['version']);
    }
    final meta = result['_meta'];
    if (meta is Map) {
      grokShell = meta['grokShell'] == true;
      this.version ??= _text(meta['agentVersion']);
    }
    final capabilities = result[version == 2 ? 'capabilities' : 'agentCapabilities'];
    bool says(List<String> path) {
      Object? at = capabilities;
      for (final step in path) {
        if (at is! Map) return false;
        at = at[step];
      }
      return at == true || at is Map;
    }
    reopenBy = version == 2
        ? 'session/resume'
        : says(const ['loadSession'])
        ? 'session/load'
        : says(const ['sessionCapabilities', 'resume'])
        ? 'session/resume'
        : null;
    return message;
  }
}

/// An ACP client that cannot exist until its peer is proven: Canoa's own rule, on the shared client.
final class VerifiedAgent {
  VerifiedAgent._(this._client, this.peerId, this.identity);

  final AcpClient _client;
  final String peerId;

  /// What the agent said about itself when it was initialized.
  final AgentIdentity identity;

  static Future<VerifiedAgent> open(
    WebChannel channel, {
    required String expectedPeerId,
    required VerifyAcpPeer verifyPeer,
    Duration verificationTimeout = const Duration(seconds: 10),
    AcpDecider? decider,
  }) async {
    if (expectedPeerId.isEmpty) throw ArgumentError.value(expectedPeerId, 'expectedPeerId');
    try {
      final actual = await verifyPeer(channel, expectedPeerId).timeout(verificationTimeout);
      if (actual.peerId != expectedPeerId) {
        await actual.channel.close();
        throw const AcpFailure('Verified peer does not match expected identity');
      }
      final proven = actual.channel;
      final identity = AgentIdentity();
      return VerifiedAgent._(
        AcpClient(
          incoming: proven.messages.map(identity._inspect),
          send: proven.send,
          closeSource: proven.close,
          decider: decider,
          clientName: 'canoa',
          clientVersion: '0.0.1',
        ),
        actual.peerId,
        identity,
      );
    } catch (_) {
      await channel.close();
      rethrow;
    }
  }

  /// A child of this app, speaking ACP on its standard streams. It is the person's own program, so nothing needs proving.
  factory VerifiedAgent.fromChild({
    required String label,
    required Stream<String> messages,
    required Future<void> Function(String) send,
    required Future<void> Function() close,
    AcpDecider? decider,
  }) {
    final identity = AgentIdentity();
    return VerifiedAgent._(
      AcpClient(
        incoming: messages.map(identity._inspect),
        send: send,
        closeSource: close,
        decider: decider,
        clientName: 'canoa',
        clientVersion: '0.0.1',
        // An adapter fetched with `npx` downloads itself the first time, before it answers anything.
        requestTimeout: const Duration(minutes: 3),
      ),
      'local:$label',
      identity,
    );
  }

  /// Over a connection a browser's provider holds. The proof is the provider's, and without one there is no client.
  static Future<VerifiedAgent> fromProvider(AcpPageConnection connection, {AcpDecider? decider}) async {
    final agent = connection.agent;
    final peer = agent.peer;
    if (!agent.verified || peer == null) {
      await connection.close();
      throw const AcpFailure('The browser has not proven who this agent is');
    }
    final identity = AgentIdentity();
    return VerifiedAgent._(
      AcpClient(
        incoming: connection.messages.map(identity._inspect),
        send: connection.send,
        closeSource: connection.close,
        decider: decider,
        clientName: 'canoa',
        clientVersion: '0.0.1',
      ),
      peer,
      identity,
    );
  }

  Stream<AcpSessionUpdate> get updates => _client.updates;
  Stream<AcpEvent> get events => _client.events;
  bool get isClosed => _client.isClosed;
  int? get protocolVersion => _client.protocolVersion;
  Future<void> initialize() => _client.initialize();
  Future<AcpDescription> describe() => _client.describe();
  Future<String> createSession({required String cwd}) => _client.createSession(cwd: cwd);
  Future<AcpOpenedSession> openSession({required String cwd}) => _client.openSession(cwd: cwd);
  Future<Map<String, dynamic>> setConfigOption(String sessionId, String configId, String value) =>
      _client.setConfigOption(sessionId, configId, value);
  Future<void> setMode(String sessionId, String modeId) => _client.setMode(sessionId, modeId);
  Future<void> resumeSession(String id, {required String cwd, bool replayHistory = false}) =>
      _client.resumeSession(id, cwd: cwd, replayHistory: replayHistory);
  bool get takesImages => _client.takesImages;
  bool get takesFiles => _client.takesFiles;
  bool get canReopenSessions => _client.canReopenSessions;
  Future<AcpSessionPage> listSessions({String? cwd, String? cursor}) => _client.listSessions(cwd: cwd, cursor: cursor);
  Future<AcpOpenedSession> reopenSession(String id, {required String cwd}) => _client.reopenSession(id, cwd: cwd);
  Future<String?> prompt(String sessionId, String text, {List<AcpAttachment> attachments = const []}) =>
      _client.prompt(sessionId, text, attachments: attachments);
  Future<void> cancel(String sessionId) => _client.cancel(sessionId);
  Future<void> closeSession(String sessionId) => _client.closeSession(sessionId);
  Future<void> close() => _client.close();
}
