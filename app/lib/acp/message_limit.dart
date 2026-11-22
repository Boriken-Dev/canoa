import 'dart:convert';

import 'package:coqui_transport_web/coqui_transport_web.dart';

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

/// What a message of [bytes] takes on a channel once sealed, whichever method seals it: the record around it, with Noise's
/// per-part overhead (19 bytes each 65518) on top of the pipe's flat allowance.
int sealedLimit(int bytes) => bytes + pipeRecordOverhead + 19 * (bytes ~/ 65518 + 1);

/// Opens [channel]'s limit to the host's [bytes], when the channel takes it: a channel that frames its own messages
/// (`HostMessageLimit`) or one whose limit is a setting (`LimitedWebChannel`, the WebSocket and WebTransport channels).
void followHostLimit(WebChannel channel, int bytes) {
  switch (channel) {
    case final HostMessageLimit framed:
      framed.followHost(bytes);
    case final LimitedWebChannel limited:
      limited.maxMessage = sealedLimit(bytes);
    default:
  }
}

/// A channel that frames its own messages and can take the longest one the host says it applies.
abstract interface class HostMessageLimit {
  /// Takes [bytes], the longest message the host takes (`[host] max_message_bytes`), as the longest message either way,
  /// whatever it was before; the channel allows for the record each message travels in.
  void followHost(int bytes);
}
