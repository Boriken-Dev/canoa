import 'package:coqui_acp/coqui_acp.dart' show AcpSessionUpdate;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as pkg;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../session_store.dart';
import '../sources/source_icons.dart';
import '../theme/look.dart';
import '../voice/reply_reader.dart';
import 'delegation_row.dart';
import 'delegation_view.dart';
import 'reply_voice.dart' show followReplies;
import 'session_views.dart';

/// What a child said, built from the updates the host sent, through the same model the conversation's own agent's updates go
/// through, so every card the transcript draws for the agent it draws for the child.
final class ChildThread {
  ChildThread(DelegateTask task) : _session = pkg.ChatSession(id: task.id, agent: task.source);

  final pkg.ChatSession _session;
  int _applied = 0;

  /// Takes what [task] heard since the last call.
  void sync(DelegateTask task) {
    final fresh = task.updatesSeen - _applied;
    if (fresh <= 0) return;
    final updates = task.updates;
    for (final raw in updates.sublist(updates.length - (fresh > updates.length ? updates.length : fresh))) {
      _session.apply(AcpSessionUpdate(task.id, raw));
    }
    _applied = task.updatesSeen;
  }

  List<ChatEntryData> get entries => [for (final message in _session.messages) ModelEntry(message)];

  /// What the child has said, in order, for voice mode to follow.
  List<pkg.ChatMessage> get messages => _session.messages;
  Map<String, pkg.TerminalView> get terminals => _session.terminals;
  List<pkg.PlanEntry> get plan => _session.plan;
}

/// What a child's header says in one line: its source, state, how long it ran (or when it was last heard of) and that the person
/// cannot write to it.
String childLine(ChildView child, DateTime now) {
  final time = timeWord(child, now);
  return '${child.source} · ${child.word}${time == null ? '' : ' · $time'} · read only';
}

/// The tools a child's updates show it used, each once with how many times, in the order first used.
List<(String, int)> toolsOf(DelegateTask task) {
  final counts = <String, int>{};
  for (final update in task.updates) {
    if (update['sessionUpdate'] != 'tool_call') continue;
    final title = update['title'];
    if (title is String && title.trim().isNotEmpty) counts[title] = (counts[title] ?? 0) + 1;
  }
  return [for (final entry in counts.entries) (entry.key, entry.value)];
}

/// What the app knows of a child beyond its conversation, only as far as the host or the agent supplied it: the task it was given,
/// where it runs, the tools it used, the result it gave back and what went wrong. A part nobody supplied is not drawn. When not
/// [full] the conversation beside it already shows what the child did, so only what it cannot show is drawn: an error, and the task
/// and result of a child whose agent reports nothing else.
class ChildFacts extends StatelessWidget {
  const ChildFacts({super.key, required this.task, this.parent, this.full = false});

  final DelegateTask task;
  final bool full;

  /// The task that started it, when one did.
  final ChildView? parent;

  static const int toolsShown = 8;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final muted = TextStyle(color: look.muted, fontSize: 13);
    final head = muted.copyWith(fontWeight: FontWeight.w600);
    final tools = toolsOf(task);
    final where = [
      if (task.source.isNotEmpty) 'Runs on ${task.source}',
      if (task.folder != null) 'in ${task.folder}',
      if (task.remote) 'on another machine',
      if (task.handed != null) 'given ${task.handed}',
      if (parent != null) 'started by ${parent!.title}' else if (task.native) "started by the conversation's agent",
    ].join(' ');
    Widget part(String label, Widget body, Key key) => Padding(
      key: key,
      padding: const EdgeInsets.only(bottom: 8),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [Text(label, style: head), body]),
    );
    final failure = (task.phase == TaskPhase.failed || task.phase == TaskPhase.rejected) ? task.message : null;
    final bare = task.native && !task.reportsProgress;
    final parts = <Widget>[
      if (task.brief != null && (full || bare)) part('Task', Text(task.brief!, maxLines: 6, overflow: TextOverflow.ellipsis), const Key('child-task')),
      if (full && where.isNotEmpty) part('Context', Text(where, style: muted), const Key('child-context')),
      if (full && tools.isNotEmpty)
        part(
          'Tools',
          Text(
            [
              for (final (title, count) in tools.take(toolsShown)) count > 1 ? '$title ×$count' : title,
              if (tools.length > toolsShown) 'and ${tools.length - toolsShown} more',
            ].join(' · '),
            style: muted,
          ),
          const Key('child-tools'),
        ),
      if (task.result != null && (full || bare)) part('Result', Text(task.result!, maxLines: 12, overflow: TextOverflow.ellipsis), const Key('child-result')),
      if (full) part('Tokens', Text(tokensLine(task.usage), style: muted), const Key('child-tokens')),
      if (failure != null) part('Error', Text(failure, style: TextStyle(color: look.danger, fontSize: 13)), const Key('child-error')),
      if (bare) Text('Live progress not reported', key: const Key('child-not-reported'), style: muted),
    ];
    if (parts.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 10, 16, 2),
      child: SingleChildScrollView(child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: parts)),
    );
  }
}

/// A child's facts, its conversation through the transcript's widgets, and what it asks of the person while it waits. [thread] holds
/// what the child said; [scroll] keeps where the person read to.
class ChildThreadBody extends StatelessWidget {
  const ChildThreadBody({
    super.key,
    required this.store,
    required this.session,
    required this.task,
    required this.thread,
    this.parent,
    this.scroll,
    this.full = false,
    this.renderers = ChatRenderers.none,
    this.narrow = false,
  });

  final SessionStore store;
  final ChatSession session;
  final DelegateTask task;
  final ChildThread thread;
  final ChildView? parent;
  final ScrollController? scroll;

  /// Whether to draw all the child's facts above its conversation.
  final bool full;
  final ChatRenderers renderers;
  final bool narrow;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    thread.sync(task);
    final asking = session.pendingPermissions.where((question) => question.task?.id == task.id).isNotEmpty;
    final first = session.pendingPermissions.isEmpty ? null : session.pendingPermissions.first;
    return Column(children: [
      // The facts take what they need up to two fifths of the room, and scroll past it.
      Flexible(flex: 2, child: ChildFacts(task: task, parent: parent, full: full)),
      Expanded(
        flex: 3,
        child: ChatTranscript(
          key: ValueKey('thread-${task.id}'),
          entries: thread.entries,
          terminals: thread.terminals,
          plan: thread.plan,
          working: !task.phase.ended,
          controller: scroll,
          renderers: renderers,
          narrow: narrow,
        ),
      ),
      if (asking && first?.task?.id == task.id)
        SessionPermissionCard(store: store, session: session)
      else if (asking)
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
          child: Text('It asks for your answer once the request ahead of it is answered.', style: TextStyle(color: look.muted, fontSize: 13)),
        ),
    ]);
  }
}

/// A child's conversation, to look at: its updates through the transcript's widgets, what it asks of the person while it waits,
/// Stop, and the way back to the conversation that handed it out. There is no composer: the person cannot write to it.
class ChildThreadView extends StatefulWidget {
  const ChildThreadView({
    super.key, required this.store, required this.session, required this.task, this.renderers = ChatRenderers.none, this.reader, this.narrow = false,
  });

  final SessionStore store;
  final ChatSession session;
  final DelegateTask task;
  final ChatRenderers renderers;

  /// Reads the child's replies aloud as they arrive while voice mode is on; null where nothing can be read aloud.
  final ReplyReader? reader;
  final bool narrow;

  @override
  State<ChildThreadView> createState() => _ChildThreadViewState();
}

class _ChildThreadViewState extends State<ChildThreadView> {
  late ChildThread _thread = ChildThread(widget.task);

  @override
  void initState() {
    super.initState();
    widget.store.addListener(_follow);
    // What the child already said is not read when it is opened; what comes after is.
    _follow();
  }

  @override
  void dispose() {
    widget.store.removeListener(_follow);
    super.dispose();
  }

  /// Voice mode hears the child's conversation as it grows, as it does the one the person started, and reads what is new.
  void _follow() {
    final reader = widget.reader;
    if (reader == null || !reader.voiceMode) return;
    _thread.sync(widget.task);
    followReplies(reader, 'child:${widget.task.id}', _thread.messages, working: !widget.task.phase.ended);
  }

  @override
  void didUpdateWidget(ChildThreadView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.store != widget.store) {
      oldWidget.store.removeListener(_follow);
      widget.store.addListener(_follow);
    }
    if (oldWidget.task.id != widget.task.id) {
      _thread = ChildThread(widget.task);
      _follow();
    }
  }

  @override
  Widget build(BuildContext context) {
    final task = widget.task, session = widget.session, store = widget.store;
    final look = context.chatLook;
    final tree = Lineage.of(session);
    final child = tree[task.id] ?? ChildView.of(task);
    return Column(children: [
      Material(
        color: context.look.bar,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(children: [
            TextButton.icon(
              key: const Key('open-parent'),
              onPressed: () => store.closeChild(session),
              icon: const Icon(Icons.arrow_back, size: 18),
              label: const Text('Open parent'),
            ),
            const SizedBox(width: 8),
            SourceMark.provider(child.provider, size: 18),
            const SizedBox(width: 8),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600)),
                Ticking(
                  active: child.live,
                  builder: (context) => Text(
                    childLine(child, store.clock()),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: stateColor(look, child), fontSize: 12),
                  ),
                ),
              ]),
            ),
            if (child.live && !task.native)
              TextButton.icon(
                key: const Key('child-stop'),
                onPressed: () => store.stopTask(session, task.id),
                icon: const Icon(Icons.stop_circle_outlined, size: 18),
                label: const Text('Stop'),
              ),
          ]),
        ),
      ),
      Expanded(
        child: ChildThreadBody(
          store: store,
          session: session,
          task: task,
          thread: _thread,
          parent: child.parent == null ? null : tree[child.parent!],
          renderers: widget.renderers,
          narrow: widget.narrow,
        ),
      ),
    ]);
  }
}
