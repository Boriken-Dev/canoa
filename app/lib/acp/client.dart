import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_web/coqui_web.dart';

export 'package:coqui_acp/coqui_acp.dart';

/// Must cryptographically verify the remote peer, not trust a supplied ID.
typedef VerifyAcpPeer = Future<String> Function(WebChannel channel, String expectedPeerId);

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
      if (actual != expectedPeerId) {
        throw const AcpFailure('Verified peer does not match expected identity');
      }
      return VerifiedAgent._(
        AcpClient(
          incoming: channel.messages,
          send: channel.send,
          closeSource: channel.close,
          clientName: 'canoa',
          clientVersion: '0.0.1',
        ),
        actual,
      );
    } catch (_) {
      await channel.close();
      rethrow;
    }
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
