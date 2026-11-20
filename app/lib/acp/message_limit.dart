import 'dart:convert';

/// What a message takes on a pipe besides its own bytes: the record it is sealed in.
const int pipeRecordOverhead = 64 * 1024;

/// The longest message the host takes, `[host] max_message_bytes`, as it said in its answer to `initialize`; null until it has.
final class HostLimit {
  int? bytes;

  /// Why [message], which is about to be sealed and sent, cannot go; null when it can or the host has said no limit. Refused here,
  /// before it is sealed: a sealed record that is not sent leaves the host out of step with this side's session.
  String? refuses(String message) {
    final limit = bytes;
    // UTF-8 takes at most three bytes for a UTF-16 code unit.
    if (limit == null || message.length * 3 <= limit) return null;
    final size = utf8.encode(message).length;
    if (size <= limit) return null;
    return 'A message of $size bytes is over the limit the host takes, $limit: [host] max_message_bytes in its configuration';
  }
}

/// A channel that frames its own messages and can take the longest one the host says it applies.
abstract interface class HostMessageLimit {
  /// Takes [bytes], the longest message the host takes (`[host] max_message_bytes`), as the longest message either way,
  /// whatever it was before; the channel allows for the record each message travels in.
  void followHost(int bytes);
}
