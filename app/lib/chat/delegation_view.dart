import 'package:coqui_acp_chat/coqui_acp_chat.dart' show Compaction, MessageRole, ToolCall, ToolText;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/widgets.dart';

import '../session_store.dart';
import 'delegation_row.dart';
import 'session_views.dart' show SessionEntries;

/// The name the host gives its delegation tools as an MCP server. A tool call named for another server is not the host's.
const String delegationServer = 'canoa-delegate';

const _delegationTools = {'delegate_task', 'task_status', 'cancel'};

final _server = RegExp('(^|[^\\w-])${RegExp.escape(delegationServer)}(?![\\w-])');
final _taskId = RegExp(r'"taskId"\s*:\s*"([^"\\]{1,200})"');

/// Whether [tool] is a call of the host's own delegation tools: its title names the host's server and one of the three tools, in
/// whatever shape the agent writes them (`canoa-delegate: delegate_task`, `mcp__canoa-delegate__cancel`).
bool isDelegationCall(ToolCall tool) =>
    _server.hasMatch(tool.title) && _delegationTools.any((name) => RegExp('(^|[^\\w])$name(?![\\w])').hasMatch(tool.title));

/// One task as the card, the waiting line and the lineage show it: plain data, with no widget and no session in it.
final class ChildView {
  const ChildView({required this.id, required this.title, required this.source, required this.phase, this.started, this.ended, this.message});

  final String id;
  final String title;

  /// The name of the source the task runs on, which is its entry in canoa.cfg.
  final String source;
  final TaskPhase phase;
  final DateTime? started, ended;

  /// What the host last said of its state: a refusal's reason, what a failure was.
  final String? message;

  /// The provider whose mark stands for the source.
  String get provider => source.toLowerCase();

  /// Whether the task has not ended.
  bool get live => !phase.ended;

  /// Whether it ended without its reply: it failed or was refused.
  bool get failed => phase == TaskPhase.failed || phase == TaskPhase.rejected;

  /// The state as one word.
  String get word => switch (phase) {
    TaskPhase.submitted => 'Starting',
    TaskPhase.working => 'Working',
    TaskPhase.inputRequired => 'Needs you',
    TaskPhase.authRequired => 'Needs a sign-in',
    TaskPhase.completed => 'Done',
    TaskPhase.failed => 'Failed',
    TaskPhase.rejected => 'Refused',
    TaskPhase.canceled => 'Stopped',
  };

  /// How long it ran, or has run at [now]; null when the host never said when it started.
  Duration? elapsedAt(DateTime now) {
    final from = started;
    if (from == null) return null;
    final span = (ended ?? now).difference(from);
    return span.isNegative ? Duration.zero : span;
  }

  static ChildView of(DelegateTask task) => ChildView(
    id: task.id, title: task.title, source: task.source, phase: task.phase, started: task.started, ended: task.ended, message: task.message,
  );
}

/// The children one card stands for.
final class DelegationView {
  const DelegationView(this.children);

  final List<ChildView> children;

  int get working => children.where((child) => child.live).length;
  int get failed => children.where((child) => child.failed).length;
  int get stopped => children.where((child) => child.phase == TaskPhase.canceled).length;

  /// How long the children have been at it: from the first start to now, or to the last end once none runs; null when none has a
  /// start.
  Duration? elapsedAt(DateTime now) {
    final starts = [for (final child in children) ?child.started];
    if (starts.isEmpty) return null;
    final first = starts.reduce((a, b) => a.isBefore(b) ? a : b);
    final ends = [for (final child in children) ?child.ended];
    final last = working == 0 && ends.isNotEmpty ? ends.reduce((a, b) => a.isAfter(b) ? a : b) : now;
    final span = last.difference(first);
    return span.isNegative ? Duration.zero : span;
  }
}

/// The rows the session inspector's Lineage list draws: one per task of [session], in the order they were first heard of. Each has
/// its source's mark ([ChildView.provider]), title, state and elapsed time ([ChildView.elapsedAt]); [SessionStore.openChild] opens
/// its thread.
List<ChildView> lineage(ChatSession session) => [for (final task in session.tasks.all) ChildView.of(task)];

/// The entry of a transcript that stands for a turn's delegation tool calls: one row for all its children.
final class DelegationEntry implements ChatEntryData {
  const DelegationEntry({required this.view, required this.at, required this.now, required this.onOpen});

  final DelegationView view;
  @override
  final DateTime at;

  /// What the time is, for the lengths the row shows.
  final DateTime Function() now;

  /// Opens a child's thread.
  final void Function(String taskId) onOpen;

  @override
  MessageRole get role => MessageRole.notice;

  @override
  String get text {
    final count = view.children.length;
    return '$count subagent${count == 1 ? '' : 's'}: ${[for (final child in view.children) '${child.title} (${child.word})'].join(', ')}';
  }

  @override
  ToolCall? get tool => null;

  @override
  Compaction? get compaction => null;
}

/// [session]'s entries with each turn's delegation tool calls, and the `task_status` and `cancel` calls that went with them, folded
/// into one [DelegationEntry] where the turn's first of them was. A call is a turn's when its result names the task; a task no call
/// names (an agent that shows no result) goes to the last turn that delegated. Calls with no task of this app's to show for them
/// stay as the agent sent them.
List<ChatEntryData> delegationEntries(ChatSession session, {required void Function(String taskId) onOpen, required DateTime Function() now}) {
  final entries = session.entries;
  if (session.tasks.isEmpty) return entries;
  final turns = <int, List<int>>{};
  var turn = 0;
  for (var i = 0; i < entries.length; i++) {
    if (entries[i].role == MessageRole.user) turn++;
    final tool = entries[i].tool;
    if (tool != null && isDelegationCall(tool)) (turns[turn] ??= []).add(i);
  }
  if (turns.isEmpty) return entries;
  final named = <int, List<String>>{};
  final claimed = <String>{};
  for (final MapEntry(:key, :value) in turns.entries) {
    for (final i in value) {
      for (final block in entries[i].tool!.blocks) {
        if (block is! ToolText) continue;
        for (final match in _taskId.allMatches(block.text)) {
          final id = match.group(1)!;
          if (session.tasks[id] != null && claimed.add(id)) (named[key] ??= []).add(id);
        }
      }
    }
  }
  final unnamed = [for (final task in session.tasks.all) if (!claimed.contains(task.id)) task.id];
  if (unnamed.isNotEmpty) (named[turns.keys.last] ??= []).addAll(unnamed);

  final folded = <int>{};
  final out = <ChatEntryData>[];
  for (var i = 0; i < entries.length; i++) {
    final tool = entries[i].tool;
    if (tool == null || !isDelegationCall(tool)) {
      out.add(entries[i]);
      continue;
    }
    // Which turn this call is of: the one whose list holds it.
    final mine = turns.entries.firstWhere((entry) => entry.value.contains(i)).key;
    final ids = named[mine];
    if (ids == null || ids.isEmpty) {
      out.add(entries[i]);
      continue;
    }
    if (!folded.add(mine)) continue;
    out.add(DelegationEntry(
      view: DelegationView([for (final id in ids) ChildView.of(session.tasks[id]!)]),
      at: entries[i].at,
      now: now,
      onOpen: onOpen,
    ));
  }
  return out;
}

/// Draws a [DelegationEntry] as its row, and passes on any other.
Widget? drawDelegation(BuildContext context, ChatEntryData entry) =>
    entry is DelegationEntry ? DelegationRow(view: entry.view, now: entry.now, onOpen: entry.onOpen) : null;

/// Canoa's own content kinds for the conversation's renderer registry.
const ChatRenderers delegationRenderers = ChatRenderers(entry: [drawDelegation]);
