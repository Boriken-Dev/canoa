import '../session_store.dart';

/// What a session in the list has for the person, as one word.
enum RowState {
  needsYou('Needs you'),
  working('Working'),
  waiting('Waiting'),
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
  bool get active => this == needsYou || this == working || this == waiting;

  /// Whether the session has nothing to say and may be folded away.
  bool get settled => this == idle;
}

/// How many of [session]'s subagents wait for the person: a question or approval of theirs, or a state that asks for input. The
/// conversation is then in Needs you, whatever the conversation's own agent is doing.
int subagentsNeedingYou(ChatSession session) {
  final asking = {for (final question in session.pendingPermissions) ?question.task?.id};
  return session.tasks.live.where((task) => task.phase.waitingOnPerson || asking.contains(task.id)).length;
}

/// The state [session] shows in the list.
RowState rowStateOf(ChatSession session) {
  if (session.needsYou || subagentsNeedingYou(session) > 0) return RowState.needsYou;
  // A turn that runs while a subagent works is waiting on it.
  if (session.status == SessionStatus.working && session.tasks.live.isNotEmpty) return RowState.waiting;
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
  needsYou('Needs you'),
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

/// The group [session] is in. A pinned session stays under Pinned whatever it is doing; an archived one comes back to Needs you
/// or Working while it needs the person or runs.
SessionGroup groupOf(ChatSession session) {
  final state = rowStateOf(session);
  if (session.pinned) return SessionGroup.pinned;
  if (state == RowState.needsYou) return SessionGroup.needsYou;
  if (state.active) return SessionGroup.working;
  if (session.archived) return SessionGroup.archived;
  return state.settled ? SessionGroup.settled : SessionGroup.sessions;
}

/// [sessions] by group, each in the order the store keeps (newest first). A group with none is left out.
Map<SessionGroup, List<ChatSession>> groupSessions(Iterable<ChatSession> sessions) {
  final groups = {for (final group in SessionGroup.values) group: <ChatSession>[]};
  for (final session in sessions) {
    groups[groupOf(session)]!.add(session);
  }
  return {for (final MapEntry(:key, :value) in groups.entries) if (value.isNotEmpty) key: value};
}

/// How the list is grouped: by what each session has for the person, or by the workspace it works in.
enum SessionGrouping {
  status('By status'),
  workspace('By workspace');

  const SessionGrouping(this.label);
  final String label;
}

/// One group of the list grouped by workspace. [key] is the workspace's path, or `none` for sessions without one.
class WorkspaceGroup {
  const WorkspaceGroup(this.key, this.label, this.sessions, {this.folds = false});

  final String key;
  final String label;
  final List<ChatSession> sessions;

  /// Whether the list folds it away until the person opens it: the archived sessions that need nothing.
  final bool folds;
}

/// The last part of a path, whichever separator it uses.
String _basename(String path) {
  final parts = path.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty);
  return parts.isEmpty ? path : parts.last;
}

/// [sessions] by workspace, in the order each workspace's first session is kept (newest first). Inside one, the sessions that
/// need the person come first. Sessions with no workspace follow the others; archived ones that are not active are folded
/// into a last group.
List<WorkspaceGroup> groupByWorkspace(Iterable<ChatSession> sessions) {
  final byPath = <String, List<ChatSession>>{};
  final archived = <ChatSession>[];
  for (final session in sessions) {
    if (session.archived && !rowStateOf(session).active) {
      archived.add(session);
    } else {
      byPath.putIfAbsent(session.cwd ?? '', () => []).add(session);
    }
  }
  List<ChatSession> ordered(List<ChatSession> list) => [
    ...list.where((session) => session.needsYou),
    ...list.where((session) => !session.needsYou),
  ];
  return [
    for (final MapEntry(:key, :value) in byPath.entries)
      if (key.isNotEmpty) WorkspaceGroup(key, _basename(key), ordered(value)),
    if (byPath['']?.isNotEmpty ?? false) WorkspaceGroup('none', 'No workspace', ordered(byPath['']!)),
    if (archived.isNotEmpty) WorkspaceGroup('archived', 'Archived', archived, folds: true),
  ];
}

/// Every session in the order the list shows them, folded groups included: what Ctrl+1 to Ctrl+9 and Ctrl+Tab go through.
List<ChatSession> listOrder(Iterable<ChatSession> sessions, [SessionGrouping grouping = SessionGrouping.status]) => switch (grouping) {
  SessionGrouping.status => [for (final group in groupSessions(sessions).values) ...group],
  SessionGrouping.workspace => [for (final group in groupByWorkspace(sessions)) ...group.sessions],
};

/// How long ago [at] was, in a word or two: "now", "5m", "3h", "2d".
String ageLabel(DateTime now, DateTime at) {
  final ago = now.difference(at);
  if (ago.inMinutes < 1) return 'now';
  if (ago.inHours < 1) return '${ago.inMinutes}m';
  if (ago.inDays < 1) return '${ago.inHours}h';
  return '${ago.inDays}d';
}

/// The branch of [session]'s worktree, else its workspace's name; null for a session with neither.
String? locationOf(ChatSession session) {
  if (session.worktree case final worktree?) return worktree.branch;
  final parts = (session.cwd ?? '').split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty);
  return parts.isEmpty ? null : parts.last;
}
