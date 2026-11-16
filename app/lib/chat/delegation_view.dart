import 'package:coqui_acp_chat/coqui_acp_chat.dart' show Compaction, MessageRole, ToolCall, ToolText;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/widgets.dart';

import '../acp/delegate.dart' show TaskUsage;
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
  const ChildView({
    required this.id,
    required this.title,
    required this.source,
    required this.phase,
    this.started,
    this.ended,
    this.message,
    this.native = false,
    this.parent,
    this.depth = 0,
    this.waitingForTask = false,
    this.lost = false,
    this.asks,
    this.updated,
    this.usage,
  });

  final String id;
  final String title;

  /// The name of the source the task runs on, which is its entry in canoa.cfg.
  final String source;
  final TaskPhase phase;
  final DateTime? started, ended;

  /// What the host last said of its state: a refusal's reason, what a failure was.
  final String? message;

  /// Whether the conversation's own agent started it, not the host's delegation tools: it cannot be stopped from here.
  final bool native;

  /// The task that started it, when one did.
  final String? parent;

  /// How many tasks stand above it in the lineage.
  final int depth;

  /// Whether it is waiting on a task of its own to end.
  final bool waitingForTask;

  /// Whether its connection ended while it ran, so how it ended is not known.
  final bool lost;

  /// What its agent asks of the person, when it waits for an answer; null otherwise.
  final String? asks;

  /// What its own session spent, where the host said; null when nobody reported it.
  final TaskUsage? usage;

  /// When the app last heard of it.
  final DateTime? updated;

  /// The provider whose mark stands for the source.
  String get provider => source.toLowerCase();

  /// Whether the task has not ended.
  bool get live => !phase.ended;

  /// Whether it ended without its reply: it failed or was refused.
  bool get failed => phase == TaskPhase.failed || phase == TaskPhase.rejected;

  /// Whether nothing is known of how it ended: its connection went with it running.
  bool get unknown => lost && phase == TaskPhase.canceled;

  /// The state, in the words the lineage uses.
  ChildState get state {
    if (unknown) return ChildState.unknown;
    if (asking) return ChildState.waitingForInput;
    return switch (phase) {
      TaskPhase.submitted => ChildState.starting,
      TaskPhase.working => waitingForTask ? ChildState.waitingForTask : ChildState.working,
      TaskPhase.inputRequired || TaskPhase.authRequired => ChildState.waitingForInput,
      TaskPhase.completed => ChildState.completed,
      TaskPhase.failed || TaskPhase.rejected => ChildState.failed,
      TaskPhase.canceled => ChildState.cancelled,
    };
  }

  /// The state as one word.
  String get word => switch (phase) {
    _ when unknown => 'Unknown',
    TaskPhase.authRequired => 'Needs a sign-in',
    TaskPhase.rejected => 'Refused',
    _ => state.word,
  };

  /// Whether its question or approval waits for the person: the thing a collapsed row has to show.
  bool get asking => live && asks != null;

  /// How long it ran, or has run at [now]; null when the host never said when it started.
  Duration? elapsedAt(DateTime now) {
    final from = started;
    if (from == null) return null;
    final span = (ended ?? now).difference(from);
    return span.isNegative ? Duration.zero : span;
  }

  static ChildView of(DelegateTask task, {String? parent, int depth = 0, bool waitingForTask = false, String? asks}) => ChildView(
    id: task.id, title: task.title, source: task.source, phase: task.phase, started: task.started, ended: task.ended, message: task.message, native: task.native,
    parent: parent, depth: depth, waitingForTask: waitingForTask, lost: task.lost, asks: asks, updated: task.updated ?? task.ended ?? task.started,
    usage: task.usage,
  );
}

/// Where a subagent is, as the lineage names it.
enum ChildState {
  starting('Starting'),
  working('Working'),
  waitingForInput('Waiting for input'),
  waitingForTask('Waiting for a task'),
  completed('Completed'),
  failed('Failed'),
  cancelled('Cancelled'),
  unknown('Unknown');

  const ChildState(this.word);
  final String word;
}

/// The children one card stands for.
final class DelegationView {
  const DelegationView(this.children);

  final List<ChildView> children;

  /// How many children are in [state].
  int count(ChildState state) => children.where((child) => child.state == state).length;

  /// The children that have not ended.
  int get live => children.where((child) => child.live).length;
  int get failed => count(ChildState.failed);
  int get stopped => count(ChildState.cancelled);

  /// The children that wait for the person.
  int get needInput => count(ChildState.waitingForInput);

  /// The counts as parts of a line, each child in exactly one: "2 working", "1 needs input", "1 done". Starting and waiting for a
  /// task count as working. A state no child is in has no part.
  List<(String, ChildState)> get parts {
    int n(Iterable<ChildState> states) => children.where((child) => states.contains(child.state)).length;
    final counts = [
      (n(const [ChildState.starting, ChildState.working, ChildState.waitingForTask]), 'working', ChildState.working),
      (needInput, 'needs input', ChildState.waitingForInput),
      (count(ChildState.completed), 'done', ChildState.completed),
      (failed, 'failed', ChildState.failed),
      (stopped, 'cancelled', ChildState.cancelled),
      (count(ChildState.unknown), 'unknown', ChildState.unknown),
    ];
    return [for (final (number, word, state) in counts) if (number > 0) ('$number $word', state)];
  }

  /// "4 subagents · 2 working · 1 needs input · 1 done".
  String get summary => [subagents(children.length), for (final (text, _) in parts) text].join(' · ');

  /// How long the children have been at it: from the first start to now, or to the last end once none runs; null when none has a
  /// start.
  Duration? elapsedAt(DateTime now) {
    final starts = [for (final child in children) ?child.started];
    if (starts.isEmpty) return null;
    final first = starts.reduce((a, b) => a.isBefore(b) ? a : b);
    final ends = [for (final child in children) ?child.ended];
    final last = live == 0 && ends.isNotEmpty ? ends.reduce((a, b) => a.isAfter(b) ? a : b) : now;
    final span = last.difference(first);
    return span.isNegative ? Duration.zero : span;
  }
}

/// Every task of [session] as the views show it, each with its parent, its depth and what it waits on. A task whose own task is
/// still running waits for it; the questions the conversation holds for a task are on that task's view.
final class Lineage {
  Lineage._(this.views, this._byId);

  factory Lineage.of(ChatSession session) {
    final tasks = session.tasks;
    final asking = <String, String>{};
    for (final question in session.pendingPermissions) {
      if (question.task case final task?) asking.putIfAbsent(task.id, () => question.title);
    }
    final all = tasks.all;
    final byParent = <String?, List<DelegateTask>>{};
    for (final task in all) {
      byParent.putIfAbsent(tasks.parentOf(task)?.id, () => []).add(task);
    }
    final views = <ChildView>[];
    final placed = <String>{};
    void place(DelegateTask task, String? parent, int depth) {
      if (!placed.add(task.id)) return;
      final below = byParent[task.id] ?? const [];
      views.add(ChildView.of(
        task,
        parent: parent,
        depth: depth,
        waitingForTask: below.any((child) => !child.phase.ended),
        asks: asking[task.id] ?? (task.phase.waitingOnPerson ? task.message : null),
      ));
      for (final child in below) {
        place(child, task.id, depth + 1);
      }
    }
    for (final task in byParent[null] ?? const <DelegateTask>[]) {
      place(task, null, 0);
    }
    // A cycle among parents would leave tasks unplaced; they still show, as roots.
    for (final task in all) {
      place(task, null, 0);
    }
    return Lineage._(views, {for (final view in views) view.id: view});
  }

  /// The tasks, each followed by the tasks it started.
  final List<ChildView> views;
  final Map<String, ChildView> _byId;

  ChildView? operator [](String id) => _byId[id];
}

/// The rows the session inspector's Lineage list draws: one per task of [session], each task followed by those it started. Each has
/// its source's mark ([ChildView.provider]), title, state and elapsed time ([ChildView.elapsedAt]); [SessionStore.openChild] opens
/// its thread.
List<ChildView> lineage(ChatSession session) => Lineage.of(session).views;

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
    return '${subagents(view.children.length)}: ${[for (final child in view.children) '${child.title} (${child.word})'].join(', ')}';
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
    if (tool != null && (isDelegationCall(tool) || session.tasks.nativeOf(tool.id) != null)) (turns[turn] ??= []).add(i);
  }
  if (turns.isEmpty) return entries;
  final named = <int, List<String>>{};
  final claimed = <String>{};
  for (final MapEntry(:key, :value) in turns.entries) {
    for (final i in value) {
      final own = session.tasks.nativeOf(entries[i].tool!.id);
      if (own != null && claimed.add(own.id)) (named[key] ??= []).add(own.id);
      for (final block in entries[i].tool!.blocks) {
        if (block is! ToolText) continue;
        for (final match in _taskId.allMatches(block.text)) {
          final id = match.group(1)!;
          if (session.tasks[id] != null && claimed.add(id)) (named[key] ??= []).add(id);
        }
      }
    }
  }
  final unnamed = [for (final task in session.tasks.all) if (!task.native && !claimed.contains(task.id)) task.id];
  if (unnamed.isNotEmpty) (named[turns.keys.last] ??= []).addAll(unnamed);

  final tree = Lineage.of(session);
  final folded = <int>{};
  final out = <ChatEntryData>[];
  for (var i = 0; i < entries.length; i++) {
    final tool = entries[i].tool;
    if (tool == null || !(isDelegationCall(tool) || session.tasks.nativeOf(tool.id) != null)) {
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
      view: DelegationView([for (final id in ids) ?tree[id]]),
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

/// A child's tokens in one line, each counter it reported and "Not reported" for the rest, so a missing counter is never read as 0:
/// "Input 120 · Output 30 · Cache read Not reported · Cache write Not reported". A child that reported none says only that.
String tokensLine(TaskUsage? usage) {
  if (usage == null) return 'Not reported';
  String of(int? count) => count == null ? 'Not reported' : compactCount(count);
  return 'Input ${of(usage.input)} · Output ${of(usage.output)} · Cache read ${of(usage.cacheRead)} · Cache write ${of(usage.cacheWrite)}';
}
