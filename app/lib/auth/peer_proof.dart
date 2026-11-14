import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import '../acp/noise_secret.dart';
import '../acp/secret_wire.dart' show SecretIdentity;
import '../acp/sealed_channel.dart';
import 'web/peer_session.dart' show PeerFailure;

/// The proof an agent's key makes, which the agent's record names: chosen per agent and never negotiated on a connection.
enum PeerMethod {
  noise('noise', 'Noise'),
  webcrypto('webcrypto', 'WebCrypto');

  const PeerMethod(this.id, this.label);

  /// The name a record and the bridge use.
  final String id;
  final String label;

  /// WebCrypto's handshake messages are text frames, Noise's binary ones.
  bool get textHandshake => this == webcrypto;

  static PeerMethod parse(String id) => values.firstWhere(
    (method) => method.id == id,
    orElse: () => throw ArgumentError.value(id, 'method', 'Not a proof method'),
  );
}

/// [method] cannot run here, for [reason]. Asked before any connection, and never a reason to try another method.
final class MethodUnavailable implements Exception {
  const MethodUnavailable(this.method, this.reason);
  final PeerMethod method;
  final String reason;
  @override
  String toString() => '${method.label} is unavailable here: $reason';
}

/// One step of a proof or a pairing: what to send now, and once it is over, the peer it proved and, for a proof, the
/// session that seals everything after it.
final class PeerStep {
  const PeerStep({this.send = const [], this.peerId, this.peerKey, this.session, this.failure});
  final List<Uint8List> send;
  final String? peerId;

  /// The static public key of the peer a Noise proof proved, as its SubjectPublicKeyInfo.
  final Uint8List? peerKey;
  final String? session;

  /// Why the exchange ended in a refusal. [send] is then the last thing to tell the peer, and nothing follows.
  final String? failure;
}

/// The calls of the web build's peer bridge (`WebPeerBridge`): each method's own key, the handshake run a step at a time,
/// then the session it ends in. A test can stand in another implementation.
abstract interface class PeerBridge {
  /// Why this runtime cannot prove by [method], or null. It asks the local runtime only, so nothing on a network can change it.
  Future<String?> unreachable(PeerMethod method);

  /// This runtime's Noise key as what a secret binding agrees with: its SubjectPublicKeyInfo and X25519 of its private key, which is
  /// never handed over, with a point. Throws where Noise is unavailable.
  Future<({Uint8List spki, Future<Uint8List> Function(Uint8List point) agree})> noiseAgreement();

  /// This runtime's own peer id under [method]: each method has a key of its own.
  Future<String> localId(PeerMethod method);

  /// Starts a proof of [expectedPeerId] by [method], and answers its token.
  Future<String> begin(PeerMethod method, String expectedPeerId);

  /// Starts a pairing of this runtime's Noise key by a typed [pin], and answers its token.
  Future<String> pair(String pin);

  /// The next step of [token], given the peer's last message, or null for the first step.
  Future<PeerStep> step(String token, Uint8List? received);
  void abort(String token);
  Future<Uint8List> seal(String session, Uint8List message);

  /// Null for the peer's close record.
  Future<Uint8List?> open(String session, Uint8List record);
  Future<Uint8List> closeRecord(String session);
  void closeSession(String session);
}

/// Runs [token]'s steps on [channel] until one names the proved peer, each wait for the peer bounded by [wait].
Future<PeerStep> _exchange(
  PeerBridge bridge,
  StagedChannel channel,
  String token, {
  required bool text,
  required Duration wait,
}) async {
  Uint8List? received;
  for (;;) {
    final step = await bridge.step(token, received);
    final paired = step.peerId != null && step.session == null;
    for (final message in step.send) {
      try {
        if (text) {
          await channel.send(utf8.decode(message));
        } else {
          await channel.sendBytes(message);
        }
      } catch (_) {
        // A finished pairing's last record is this side's close, which a peer that confirmed and hung up does not need; a
        // refusal is told as far as the peer still listens.
        if (!paired && step.failure == null) rethrow;
      }
    }
    if (step.failure != null) throw PeerFailure(step.failure!);
    if (step.peerId != null) return step;
    received = await channel.nextSetupBytes().timeout(wait);
  }
}

/// What a Noise proof lets this connection bind secret answers with: this key agreeing with the host's static key, which the proof gave, or
/// none when the proof gave no key whose digest is the pinned id. Never throws: no identity is no secret prompts.
Future<SecretIdentity?> _noiseSecrets(PeerBridge bridge, PeerStep proof) async {
  try {
    final own = await bridge.noiseAgreement();
    return noiseSecretIdentity(hostId: proof.peerId!, hostSpki: proof.peerKey, spki: own.spki, agree: own.agree);
  } catch (_) {
    return null;
  }
}

/// Proves [expectedPeerId] by [method] on [channel] through [bridge], and answers with the channel every later message must
/// use. A method this runtime cannot run fails before anything is sent, and no other method is tried.
Future<VerifiedPeer> provePeer(
  PeerBridge bridge,
  WebChannel channel,
  String expectedPeerId, {
  required PeerMethod method,
}) async {
  if (channel is! StagedChannel) {
    throw StateError('A staged transport connection is required');
  }
  final why = await bridge.unreachable(method);
  if (why != null) throw MethodUnavailable(method, why);
  String? token;
  try {
    token = await bridge.begin(method, expectedPeerId);
    final done = await _exchange(
      bridge,
      channel,
      token,
      text: method.textHandshake,
      wait: const Duration(seconds: 15),
    );
    final session = done.session;
    if (session == null) throw const FormatException('A proof ended with no session');
    final sealed = SealedChannel(
      channel,
      seal: (message) => bridge.seal(session, message),
      open: (record) => bridge.open(session, record),
      closeRecord: () => bridge.closeRecord(session),
      release: () => bridge.closeSession(session),
    );
    if (method == PeerMethod.noise) noiseSecretsOf[sealed] = (identity: await _noiseSecrets(bridge, done));
    return VerifiedPeer(done.peerId!, sealed);
  } finally {
    if (token != null) bridge.abort(token);
  }
}

/// Pairs this runtime's Noise key, by the [pin] a person typed, with the machine on [channel], which runs
/// `coqui-acp-bridge pair-accept --method noise`, and answers the id that machine proved. The channel is closed after. A
/// refused PIN is spent: ask for a new one, never try it again.
Future<String> pairByPin(
  PeerBridge bridge,
  WebChannel channel,
  String pin, {
  Duration timeout = const Duration(minutes: 2),
}) async {
  String? token;
  try {
    if (channel is! StagedChannel) {
      throw StateError('A staged transport connection is required');
    }
    final why = await bridge.unreachable(PeerMethod.noise);
    if (why != null) throw MethodUnavailable(PeerMethod.noise, why);
    token = await bridge.pair(pin);
    final done = await _exchange(bridge, channel, token, text: false, wait: timeout).timeout(timeout);
    return done.peerId!;
  } finally {
    if (token != null) bridge.abort(token);
    await channel.close();
  }
}
