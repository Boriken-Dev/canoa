import '../session_store.dart';

/// What a session in the list has for the person, as one word.
enum RowState {
  needsYou('Needs you'),
  working('Working'),
  failed('Failed'),
  done('Done'),
  starting('Starting'),
  offline('Offline'),
  preview('Preview'),
  fresh('New'),
  idle('Idle');

  const RowState(this.word);
  final String word;

  /// Whether the session is doing or waiting for something now, which keeps it in view whatever else is chosen for it.
  bool get active => this == needsYou || this == working;

  /// Whether the session has nothing to say and may be folded away.
  bool get settled => this == idle;
}

/// The state [session] shows in the list.
RowState rowStateOf(ChatSession session) {
  if (session.needsYou) return RowState.needsYou;
  if (session.status == SessionStatus.working) return RowState.working;
  if (session.failed) return RowState.failed;
  if (session.unread) return RowState.done;
  return switch (session.status) {
    SessionStatus.preview => RowState.preview,
    SessionStatus.draft => RowState.fresh,
    SessionStatus.connecting => RowState.starting,
    SessionStatus.disconnected => RowState.offline,
    _ => RowState.idle,
  };
}

/// The parts of the list, top to bottom.
enum SessionGroup {
  pinned('Pinned'),
  working('Working'),
  sessions('Sessions'),
  settled('Settled'),
  archived('Archived');

  const SessionGroup(this.label);
  final String label;

  /// Whether the list folds the group away until the person opens it.
  bool get folds => this == settled || this == archived;
}

/// The group [session] is in. A pinned session stays under Pinned whatever it is doing; an archived one comes back to Working
/// while it needs the person or runs.
SessionGroup groupOf(ChatSession session) {
  final state = rowStateOf(session);
  if (session.pinned) return SessionGroup.pinned;
  if (state.active) return SessionGroup.working;
  if (session.archived) return SessionGroup.archived;
  return state.settled ? SessionGroup.settled : SessionGroup.sessions;
}

/// [sessions] by group, each in the order the store keeps (newest first), the ones that need the person before the ones that
/// run. A group with none is left out.
Map<SessionGroup, List<ChatSession>> groupSessions(Iterable<ChatSession> sessions) {
  final groups = {for (final group in SessionGroup.values) group: <ChatSession>[]};
  for (final session in sessions) {
    groups[groupOf(session)]!.add(session);
  }
  final working = groups[SessionGroup.working]!;
  groups[SessionGroup.working] = [
    ...working.where((session) => session.needsYou),
    ...working.where((session) => !session.needsYou),
  ];
  return {for (final MapEntry(:key, :value) in groups.entries) if (value.isNotEmpty) key: value};
}

/// Every session in the order the list shows them, folded groups included: what Ctrl+1 to Ctrl+9 and Ctrl+Tab go through.
List<ChatSession> listOrder(Iterable<ChatSession> sessions) => [
  for (final group in groupSessions(sessions).values) ...group,
];

/// How long ago [at] was, in a word or two: "now", "5m", "3h", "2d".
String ageLabel(DateTime now, DateTime at) {
  final ago = now.difference(at);
  if (ago.inMinutes < 1) return 'now';
  if (ago.inHours < 1) return '${ago.inMinutes}m';
  if (ago.inDays < 1) return '${ago.inHours}h';
  return '${ago.inDays}d';
}

/// The branch of [session]'s worktree, else its folder's name; null for a session with neither.
String? locationOf(ChatSession session) {
  if (session.worktree case final worktree?) return worktree.branch;
  final parts = (session.cwd ?? '').split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty);
  return parts.isEmpty ? null : parts.last;
}
