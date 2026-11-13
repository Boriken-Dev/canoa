import 'dart:async';
import 'dart:convert';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

/// A [ChatViewLink] over a channel that carries strings, the way a platform's window messages and a page's `BroadcastChannel` do:
/// each message goes as a line of JSON, and what arrives is read back the same way. A subclass supplies how a string goes out and
/// calls [receive] for what arrives.
abstract class StringViewLink implements ChatViewLink {
  final _incoming = StreamController<Map<String, Object?>>();
  var _closed = false;

  @override
  Stream<Map<String, Object?>> get messages => _incoming.stream;

  @override
  bool get isClosed => _closed;

  /// Puts [text] on the channel.
  void write(String text);

  /// Takes a string the other end sent. Anything else on the channel is ignored.
  void receive(Object? text) {
    if (_closed || text is! String) return;
    try {
      final decoded = jsonDecode(text);
      if (decoded is Map) _incoming.add(decoded.cast<String, Object?>());
    } on FormatException {
      return;
    }
  }

  @override
  void send(Map<String, Object?> message) {
    if (_closed) return;
    write(jsonEncode(message));
  }

  /// The other end cannot be reached any more: the link ends as if it were closed.
  void fail() => unawaited(close());

  /// Stops listening to the channel; called by [close].
  Future<void> release();

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    await release();
    // Nobody may be listening, and the end of a stream nobody listens to is never delivered: closing is not waited for.
    unawaited(_incoming.close());
  }
}
