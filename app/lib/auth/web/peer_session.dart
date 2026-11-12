import 'dart:typed_data';

/// The session a proof ends in: every later message is one sealed record, in call order.
abstract interface class PeerSession {
  /// One message as one record; records are sent whole, in the order of these calls.
  Future<Uint8List> seal(Uint8List message);

  /// The message of the next record, or null for the peer's close. A bad record ends the session.
  Future<Uint8List?> open(Uint8List record);

  /// The record that ends this direction, to send last.
  Future<Uint8List> close();

  /// Drops every key the session holds; each later call fails.
  void destroy();
}

/// A refusal or failure of a proof, a pairing or a session. The text is for the person; it never holds a key, a PIN or a
/// handshake byte.
final class PeerFailure implements Exception {
  const PeerFailure(this.message);
  final String message;
  @override
  String toString() => message;
}
