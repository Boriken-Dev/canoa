import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import '../acp/sealed_channel.dart';

/// The WebCrypto method's calls, as `web/canoa_peer_bridge.mjs` offers them: the handshake, then the session it ends in. The
/// browser build reaches them through JavaScript interop; anything else that runs the same module can stand in.
abstract interface class PeerBridge {
  /// `{"token", "hello", "localPeerId"}` as JSON, for a proof of [expectedPeerId].
  Future<String> begin(String expectedPeerId);
  Future<String> answer(String token, String challenge);

  /// `{"peerId", "session"}` as JSON once the peer's acceptance checks out.
  Future<String> finish(String token, String accepted);
  void abort(String token);
  Future<Uint8List> seal(String session, Uint8List message);

  /// Null for the peer's close record.
  Future<Uint8List?> open(String session, Uint8List record);
  Future<Uint8List> closeRecord(String session);
  void closeSession(String session);
}

/// Proves [expectedPeerId] on [channel] through [bridge], and answers with the channel every later message must use.
Future<VerifiedPeer> provePeer(PeerBridge bridge, WebChannel channel, String expectedPeerId) async {
  if (channel is! StagedChannel) {
    throw StateError('A staged transport connection is required');
  }
  String? token;
  try {
    final start = jsonDecode(await bridge.begin(expectedPeerId));
    if (start is! Map || start['token'] is! String || start['hello'] is! String) {
      throw const FormatException('Invalid browser identity handshake');
    }
    token = start['token'] as String;
    final first = channel.nextSetupMessage();
    await channel.send(start['hello'] as String);
    final challenge = await first.timeout(const Duration(seconds: 15));
    final proof = await bridge.answer(token, challenge);

    final second = channel.nextSetupMessage();
    await channel.send(proof);
    final accepted = await second.timeout(const Duration(seconds: 15));
    final finished = jsonDecode(await bridge.finish(token, accepted));
    if (finished is! Map || finished['peerId'] is! String || finished['session'] is! String) {
      throw const FormatException('Invalid browser identity result');
    }
    final session = finished['session'] as String;
    return VerifiedPeer(
      finished['peerId'] as String,
      SealedChannel(
        channel,
        seal: (message) => bridge.seal(session, message),
        open: (record) => bridge.open(session, record),
        closeRecord: () => bridge.closeRecord(session),
        release: () => bridge.closeSession(session),
      ),
    );
  } finally {
    if (token != null) bridge.abort(token);
  }
}
