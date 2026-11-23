import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../session_store.dart';
import '../sources/source_icons.dart' show SourceMark;
import 'delegation_row.dart' show Ticking, stateColor;
import 'delegation_view.dart' show ChildView;

/// One command the agent ran in a terminal of its own, as the Details surface lists it.
final class BackgroundCommand {
  const BackgroundCommand({required this.id, required this.command, required this.running, this.exitCode, this.started, this.ended});

  final String id;
  final String command;
  final bool running;

  /// How it ended, where the agent said.
  final int? exitCode;

  /// When this app first heard of it and of its end; null for one it only learned of afterwards.
  final DateTime? started, ended;

  /// How long it ran, or has run at [now]; null when this app did not see it start.
  Duration? elapsedAt(DateTime now) {
    final from = started;
    return from == null ? null : (ended ?? now).difference(from);
  }
}

/// The commands [session]'s agent reported in terminals, in the order it reported them.
List<BackgroundCommand> commandsOf(ChatSession session) => [
  for (final MapEntry(:key, :value) in session.terminals.entries)
    BackgroundCommand(
      id: key,
      command: value.command ?? key,
      running: value.isRunning,
      exitCode: value.exitCode,
      started: session.terminalSpans[key]?.started,
      ended: session.terminalSpans[key]?.ended,
    ),
];

/// The session's subagents and commands: those still running first, each with how long it has run and, for a subagent the host
/// started, a way to stop it; the ones that ended fold into "N finished". A command has no Stop, because the agent runs it in
/// its own terminal and the protocol gives the app no way to end one.
class BackgroundTasks extends StatefulWidget {
  const BackgroundTasks({super.key, required this.children, required this.commands, required this.now, this.onOpenChild, this.onStopChild});

  final List<ChildView> children;
  final List<BackgroundCommand> commands;
  final DateTime Function() now;

  /// Opens the thread of the subagent with this id.
  final ValueChanged<String>? onOpenChild;

  /// Stops the subagent with this id; null where there is no host to ask.
  final ValueChanged<String>? onStopChild;

  /// How many running rows show before "Show more".
  static const int shown = 4;

  /// How many are running.
  static int runningIn(List<ChildView> children, List<BackgroundCommand> commands) =>
      children.where((child) => child.live).length + commands.where((command) => command.running).length;

  @override
  State<BackgroundTasks> createState() => _BackgroundTasksState();
}

class _BackgroundTasksState extends State<BackgroundTasks> {
  bool _more = false, _finished = false;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final children = widget.children, commands = widget.commands;
    final running = <Widget Function(DateTime)>[
      for (final child in children.where((child) => child.live)) (now) => _child(look, child, now),
      for (final command in commands.where((command) => command.running)) (now) => _command(look, command, now),
    ];
    final ended = <Widget Function(DateTime)>[
      for (final child in children.where((child) => !child.live)) (now) => _child(look, child, now),
      for (final command in commands.where((command) => !command.running)) (now) => _command(look, command, now),
    ];
    final visible = _more ? running : running.take(BackgroundTasks.shown).toList();
    final link = TextStyle(color: look.muted, fontSize: 13);
    return Ticking(
      active: running.isNotEmpty,
      builder: (context) {
        final now = widget.now();
        return Column(key: const ValueKey('lineage panel'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          for (final row in visible) row(now),
          if (running.length > BackgroundTasks.shown)
            _Toggle(
              key: const ValueKey('tasks more'),
              label: _more ? 'Show less' : 'Show more',
              style: link,
              onTap: () => setState(() => _more = !_more),
            ),
          if (ended.isNotEmpty) ...[
            _Toggle(
              key: const ValueKey('tasks finished'),
              label: '${ended.length} finished',
              style: link,
              icon: _finished ? Icons.expand_more : Icons.chevron_right,
              onTap: () => setState(() => _finished = !_finished),
            ),
            if (_finished) for (final row in ended) row(now),
          ],
        ]);
      },
    );
  }

  Widget _child(ChatLook look, ChildView child, DateTime now) {
    final elapsed = child.unknown ? null : child.elapsedAt(now);
    final stop = widget.onStopChild;
    return InkWell(
      key: ValueKey('lineage ${child.id}'),
      borderRadius: BorderRadius.circular(look.radius / 2),
      onTap: widget.onOpenChild == null ? null : () => widget.onOpenChild!(child.id),
      child: Padding(
        padding: EdgeInsets.fromLTRB(16.0 * child.depth.clamp(0, 4), 4, 0, 4),
        child: Row(children: [
          SizedBox.square(dimension: 18, child: Center(child: SourceMark.provider(child.provider, size: 14))),
          const SizedBox(width: 8),
          Expanded(child: Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 13))),
          const SizedBox(width: 8),
          Text(child.word, style: TextStyle(color: stateColor(look, child), fontSize: 12)),
          if (elapsed != null) Padding(padding: const EdgeInsets.only(left: 8), child: Text(spanLabel(elapsed), style: TextStyle(color: look.muted, fontSize: 12))),
          if (child.live && !child.native && stop != null)
            IconButton(
              key: ValueKey('stop ${child.id}'),
              tooltip: 'Stop ${child.title}',
              visualDensity: VisualDensity.compact,
              iconSize: 16,
              constraints: const BoxConstraints.tightFor(width: 28, height: 28),
              padding: EdgeInsets.zero,
              onPressed: () => stop(child.id),
              icon: const Icon(Icons.stop_circle_outlined),
            ),
        ]),
      ),
    );
  }

  Widget _command(ChatLook look, BackgroundCommand command, DateTime now) {
    final elapsed = command.elapsedAt(now);
    final code = command.exitCode;
    return Padding(
      key: ValueKey('command ${command.id}'),
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(children: [
        SizedBox.square(dimension: 18, child: Icon(Icons.terminal, size: 16, color: look.muted)),
        const SizedBox(width: 8),
        Expanded(child: Text(command.command, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(fontFamily: look.codeFont, fontSize: 12))),
        if (!command.running && code != null) ...[
          const SizedBox(width: 8),
          Text(code == 0 ? 'Done' : 'Exit $code', style: TextStyle(color: code == 0 ? look.success : look.danger, fontSize: 12)),
        ],
        if (elapsed != null) Padding(padding: const EdgeInsets.only(left: 8), child: Text(spanLabel(elapsed), style: TextStyle(color: look.muted, fontSize: 12))),
      ]),
    );
  }
}

class _Toggle extends StatelessWidget {
  const _Toggle({super.key, required this.label, required this.style, required this.onTap, this.icon});

  final String label;
  final TextStyle style;
  final VoidCallback onTap;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: onTap,
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(children: [
        if (icon != null) ...[Icon(icon, size: 16, color: style.color), const SizedBox(width: 4)],
        Text(label, style: style),
      ]),
    ),
  );
}
