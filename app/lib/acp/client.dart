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

/// An ACP client that cannot exist until its peer is proven: Canoa's own rule, on the shared client.
final class VerifiedAgent {
  VerifiedAgent._(this._client, this.peerId);

  final AcpClient _client;
  final String peerId;

  static Future<VerifiedAgent> open(
    WebChannel channel, {
    required String expectedPeerId,
    required VerifyAcpPeer verifyPeer,
    Duration verificationTimeout = const Duration(seconds: 10),
  }) async {
    if (expectedPeerId.isEmpty) throw ArgumentError.value(expectedPeerId, 'expectedPeerId');
    try {
      final actual = await verifyPeer(channel, expectedPeerId).timeout(verificationTimeout);
      if (actual.peerId != expectedPeerId) {
        await actual.channel.close();
        throw const AcpFailure('Verified peer does not match expected identity');
      }
      final proven = actual.channel;
      return VerifiedAgent._(
        AcpClient(
          incoming: proven.messages,
          send: proven.send,
          closeSource: proven.close,
          clientName: 'canoa',
          clientVersion: '0.0.1',
        ),
        actual.peerId,
      );
    } catch (_) {
      await channel.close();
      rethrow;
    }
  }

  /// Over a connection a browser's provider holds. The proof is the provider's, and without one there is no client.
  static Future<VerifiedAgent> fromProvider(AcpPageConnection connection) async {
    final agent = connection.agent;
    final peer = agent.peer;
    if (!agent.verified || peer == null) {
      await connection.close();
      throw const AcpFailure('The browser has not proven who this agent is');
    }
    return VerifiedAgent._(
      AcpClient(
        incoming: connection.messages,
        send: connection.send,
        closeSource: connection.close,
        clientName: 'canoa',
        clientVersion: '0.0.1',
      ),
      peer,
    );
  }

  Stream<AcpSessionUpdate> get updates => _client.updates;
  Stream<AcpEvent> get events => _client.events;
  bool get isClosed => _client.isClosed;
  int? get protocolVersion => _client.protocolVersion;
  Future<void> initialize() => _client.initialize();
  Future<String> createSession({required String cwd}) => _client.createSession(cwd: cwd);
  Future<void> resumeSession(String id, {required String cwd, bool replayHistory = false}) =>
      _client.resumeSession(id, cwd: cwd, replayHistory: replayHistory);
  Future<String?> prompt(String sessionId, String text) => _client.prompt(sessionId, text);
  Future<void> cancel(String sessionId) => _client.cancel(sessionId);
  Future<void> closeSession(String sessionId) => _client.closeSession(sessionId);
  Future<void> close() => _client.close();
}
