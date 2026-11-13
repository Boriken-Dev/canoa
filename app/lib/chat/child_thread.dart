import 'package:coqui_acp/coqui_acp.dart' show AcpSessionUpdate;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as pkg;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../session_store.dart';
import '../sources/source_icons.dart';
import '../theme/look.dart';
import 'delegation_row.dart';
import 'delegation_view.dart';
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
  Map<String, pkg.TerminalView> get terminals => _session.terminals;
  List<pkg.PlanEntry> get plan => _session.plan;
}

/// A child's conversation, to look at: its updates through the transcript's widgets, what it asks of the person while it waits,
/// Stop, and the way back to the conversation that handed it out. There is no composer: the person cannot write to it.
class ChildThreadView extends StatefulWidget {
  const ChildThreadView({
    super.key, required this.store, required this.session, required this.task, this.renderers = ChatRenderers.none, this.narrow = false,
  });

  final SessionStore store;
  final ChatSession session;
  final DelegateTask task;
  final ChatRenderers renderers;
  final bool narrow;

  @override
  State<ChildThreadView> createState() => _ChildThreadViewState();
}

class _ChildThreadViewState extends State<ChildThreadView> {
  late ChildThread _thread = ChildThread(widget.task);

  @override
  void didUpdateWidget(ChildThreadView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.task.id != widget.task.id) _thread = ChildThread(widget.task);
  }

  @override
  Widget build(BuildContext context) {
    final task = widget.task, session = widget.session, store = widget.store;
    _thread.sync(task);
    final look = context.chatLook;
    final child = ChildView.of(task);
    final asking = session.pendingPermissions.where((question) => question.task?.id == task.id).isNotEmpty;
    final first = session.pendingPermissions.isEmpty ? null : session.pendingPermissions.first;
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
                  builder: (context) {
                    final elapsed = child.elapsedAt(store.clock());
                    return Text(
                      '${child.source} · ${child.word}${elapsed == null ? '' : ' · ${spanLabel(elapsed)}'} · read only',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: stateColor(look, child), fontSize: 12),
                    );
                  },
                ),
              ]),
            ),
            if (child.live)
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
        child: ChatTranscript(
          key: ValueKey('thread-${task.id}'),
          entries: _thread.entries,
          terminals: _thread.terminals,
          plan: _thread.plan,
          working: child.live,
          renderers: widget.renderers,
          narrow: widget.narrow,
        ),
      ),
      if (task.message != null && child.failed)
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 8),
          child: Align(alignment: Alignment.centerLeft, child: Text(task.message!, style: TextStyle(color: look.danger, fontSize: 13))),
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
