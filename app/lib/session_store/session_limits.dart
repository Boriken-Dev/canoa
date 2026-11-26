part of '../session_store.dart';

/// The words the host says in a turn it refuses for a limit: `Limit NAME stopped this turn: ...`.
final _stoppedBy = RegExp(r'^Limit (\S+) stopped this turn');

extension SessionLimits on SessionStore {
  /// The limit that refused the last turn of [session], while the person has not sent another.
  LimitStop? limitStop(ChatSession session) => _limitStops[session.id];

  /// What the protocol library says of a turn that ended with the stop reason `refusal` (ACP v1).
  static const _refusedWords = 'the agent refused to continue';

  /// Listens for [session]'s turns ending with the stop reason `refusal`, on the connection it has now: in v1 the turn's answer, in v2 the
  /// idle state that gives `refusal` as its reason.
  void _watchRefusals(ChatSession session) {
    final connection = session.chat.connection;
    final held = _refusals[session.id];
    if (held != null && identical(held.connection, connection)) return;
    for (final each in held?.listening ?? const <StreamSubscription<Object?>>[]) {
      unawaited(each.cancel());
    }
    if (connection == null) {
      _refusals.remove(session.id);
      return;
    }
    _refusals[session.id] = (
      connection: connection,
      listening: [
        connection.events.listen((event) {
          if (event is AcpFailed && event.message == _refusedWords && (event.sessionId == null || event.sessionId == session.chat.sessionId)) {
            unawaited(refused(session));
          }
        }),
        connection.updates.listen((update) {
          if (update.sessionId == session.chat.sessionId && update.kind == 'state_update' && update.state == 'idle' && update.reason == 'refusal') {
            unawaited(refused(session));
          }
        }),
      ],
    );
  }

  /// A turn of [session] ended with `refusal`. When the host's own words in it name a limit, the host is asked how that limit
  /// stands, and the conversation shows the card of the limit that is stopping it; an agent's own refusal shows none.
  Future<void> refused(ChatSession session) async {
    final link = limits;
    if (link == null) return;
    // The host's words come as the turn's last message, which the conversation may not have taken yet when the turn's end is told.
    String? name;
    for (var wait = 0; wait < 5 && name == null; wait++) {
      if (wait > 0) await Future<void>.delayed(const Duration(milliseconds: 100));
      // Only what was said since the person's last message: the chat's own notice that the turn failed comes after the host's words.
      for (final message in session.chat.messages.reversed) {
        if (message.role == MessageRole.user) break;
        if (message.role == MessageRole.agent) name ??= _stoppedBy.firstMatch(message.text)?.group(1);
      }
    }
    if (name == null) return;
    try {
      final rows = await link.state();
      final stop = rows.where((row) => row.name == name).map(LimitStop.ofRow).nonNulls.firstOrNull;
      if (stop == null) return;
      _limitStops[session.id] = stop;
      _notify();
    } on Object {
      // The host cannot say now: the notice in the conversation still gives the reason.
    }
  }

  void _clearLimitStop(ChatSession session) {
    if (_limitStops.remove(session.id) != null) _notify();
  }

  /// Lets one turn through the limit that refused [session], and sends the message it refused again. Returns the host's words when it
  /// would not.
  Future<String?> continueThroughLimit(ChatSession session) async {
    final stop = _limitStops[session.id], link = limits;
    if (stop == null || link == null) return null;
    final refusedBy = await link.allow(stop.limit, session: session.chat.sessionId);
    if (refusedBy != null) return refusedBy;
    _clearLimitStop(session);
    for (final message in session.chat.messages.reversed) {
      if (message.role != MessageRole.user) continue;
      await sendTextTo(session, message.text);
      break;
    }
    return null;
  }
}
