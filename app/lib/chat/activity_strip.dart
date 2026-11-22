import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:flutter/material.dart';

import '../rail/session_groups.dart' show subagentsNeedingYou;
import '../session_store.dart';

/// A compact status bar shown above the composer whenever the session is active,
/// waiting on input, or running background subagents, ACP commands, or interactive shells.
class SessionActivityStrip extends StatelessWidget {
  const SessionActivityStrip({
    super.key,
    required this.session,
    this.openShells = 0,
    this.runningShells = 0,
    this.onOpenSubagents,
    this.onOpenDetails,
    this.onOpenTerminal,
    this.onStopSubagents,
  });

  final ChatSession session;
  final int openShells;
  final int runningShells;
  final VoidCallback? onOpenSubagents;
  final VoidCallback? onOpenDetails;
  final VoidCallback? onOpenTerminal;
  final VoidCallback? onStopSubagents;

  /// Running ACP commands reported by the agent in [session].
  static int runningCommandsOf(ChatSession session) =>
      session.terminals.values.where((each) => each.isRunning).length;

  /// Whether there is any live activity or wait state worth showing above the composer.
  /// Shows background activity between parent turns (live subagents, running ACP commands,
  /// open interactive shells, subagents needing input) or hidden background commands/shells
  /// during a parent turn without shifting plain turns or pending main-turn prompt goldens.
  static bool visibleFor(ChatSession session, {int openShells = 0, int runningShells = 0}) {
    if (session.isDraft || session.status == SessionStatus.preview) return false;
    final needingSubagents = subagentsNeedingYou(session);
    final commands = runningCommandsOf(session);
    if (session.status == SessionStatus.working) {
      return commands > 0 || runningShells > 0 || openShells > 0 || needingSubagents > 0;
    }
    return needingSubagents > 0 ||
        session.tasks.live.isNotEmpty ||
        commands > 0 ||
        runningShells > 0 ||
        openShells > 0;
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final needing = subagentsNeedingYou(session);
    final liveTasks = session.tasks.live;
    final commands = runningCommandsOf(session);
    final stoppableTasks = liveTasks.any((task) => !task.native);
    final working = session.status == SessionStatus.working;

    final String stateLabel;
    final Color stateColor;
    if (needing > 0) {
      stateLabel = 'Subagent needs input · $needing';
      stateColor = look.warning;
    } else if (working) {
      stateLabel = 'Running beside turn';
      stateColor = look.text;
    } else if (liveTasks.isEmpty && commands == 0 && runningShells == 0) {
      stateLabel = 'Shells open';
      stateColor = look.text;
    } else {
      stateLabel = 'Running in background';
      stateColor = look.text;
    }

    return Container(
      key: const Key('session-activity-strip'),
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
      decoration: BoxDecoration(
        color: look.surface,
        border: Border(top: BorderSide(color: look.border)),
      ),
      child: Wrap(
        spacing: 8,
        runSpacing: 6,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                needing > 0 ? Icons.front_hand_outlined : Icons.bolt_outlined,
                size: 14,
                color: needing > 0 ? look.warning : look.accent,
              ),
              const SizedBox(width: 6),
              Text(
                stateLabel,
                style: TextStyle(fontSize: 12, fontWeight: FontWeight.w600, color: stateColor),
              ),
            ],
          ),
          if (liveTasks.isNotEmpty)
            _ActivityChip(
              key: const Key('activity-subagents'),
              icon: Icons.account_tree_outlined,
              label: '${liveTasks.length} live subagent${liveTasks.length == 1 ? '' : 's'}',
              color: needing > 0 ? look.warning : look.accent,
              onTap: onOpenSubagents,
            ),
          if (!working && stoppableTasks && onStopSubagents != null)
            TextButton.icon(
              key: const Key('activity-stop-subagents'),
              style: TextButton.styleFrom(
                visualDensity: VisualDensity.compact,
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                minimumSize: Size.zero,
              ),
              onPressed: onStopSubagents,
              icon: const Icon(Icons.stop_circle_outlined, size: 15),
              label: const Text('Stop all', style: TextStyle(fontSize: 12)),
            ),
          if (commands > 0)
            _ActivityChip(
              key: const Key('activity-commands'),
              icon: Icons.play_arrow_outlined,
              label: commands == 1 ? '1 command' : '$commands commands',
              color: look.accent,
              onTap: onOpenDetails,
            ),
          if (runningShells > 0 || openShells > 0)
            _ActivityChip(
              key: const Key('activity-shells'),
              icon: Icons.terminal,
              label: runningShells > 0
                  ? (runningShells == 1 ? '1 shell running' : '$runningShells shells running')
                  : (openShells == 1 ? '1 shell open' : '$openShells shells open'),
              color: runningShells > 0 ? look.accent : look.muted,
              onTap: onOpenTerminal,
            ),
        ],
      ),
    );
  }
}

class _ActivityChip extends StatelessWidget {
  const _ActivityChip({
    super.key,
    required this.icon,
    required this.label,
    required this.color,
    this.onTap,
  });

  final IconData icon;
  final String label;
  final Color color;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Material(
      color: look.raised,
      borderRadius: BorderRadius.circular(12),
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: look.border),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(icon, size: 13, color: color),
              const SizedBox(width: 5),
              Text(label, style: TextStyle(fontSize: 12, color: look.text)),
            ],
          ),
        ),
      ),
    );
  }
}
