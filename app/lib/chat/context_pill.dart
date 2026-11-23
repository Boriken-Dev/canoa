import 'package:coqui_acp_chat/coqui_acp_chat.dart' show SessionUsage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../session_store.dart' show ChatSession, SessionStatus;
import 'agent_events.dart' show AgentCommandInfo;

/// The command an agent reports for compacting its context, or null when it reports none: Compact is offered only for what the
/// agent itself lists, and the prompt sent is that command as the agent names it.
AgentCommandInfo? compactCommand(Iterable<AgentCommandInfo> commands) {
  for (final command in commands) {
    if (command.name.toLowerCase() == 'compact') return command;
  }
  return null;
}

/// The prompt that compacts [session]'s context, or null when its agent reports no such command or cannot take a prompt.
String? compactPrompt(ChatSession session) {
  if (session.status != SessionStatus.connected) return null;
  final command = compactCommand(session.commands);
  return command == null ? null : '/${command.name}';
}

/// Shows how full the context window is right beside the composer, and offers to run the agent's compact command or open the breakdown in Details.
class ContextComposerPill extends StatelessWidget {
  const ContextComposerPill({
    super.key,
    required this.usage,
    this.onCompact,
    this.onOpenDetails,
  });

  final SessionUsage usage;

  /// Runs the agent's own compact command; null when the agent offers none or cannot take a prompt now.
  final VoidCallback? onCompact;

  /// Opens the Details inspector to the context breakdown; null in chat-only mode.
  final VoidCallback? onOpenDetails;

  @override
  Widget build(BuildContext context) {
    final percent = usage.percent;
    if (percent == null && onCompact == null) return const SizedBox.shrink();

    final label = percent == null ? 'Compact' : '$percent% context';
    final warn = percent != null && percent >= 75;
    final danger = percent != null && percent >= 90;
    final tooltip = switch ((percent, usage.used, usage.size, onCompact != null)) {
      (final p?, final used?, final size?, true) => 'Context window: ${compactCount(used)} / ${compactCount(size)} ($p%) · Click to compact',
      (final p?, final used?, final size?, false) => 'Context window: ${compactCount(used)} / ${compactCount(size)} ($p%)',
      (final p?, _, _, _) => 'Context window: $p% used',
      _ => 'Ask the agent to compact its context',
    };

    return MenuAnchor(
      menuChildren: [
        if (onCompact != null)
          MenuItemButton(
            key: const Key('context-compact'),
            leadingIcon: const Icon(Icons.compress, size: 18),
            onPressed: onCompact,
            child: const Text('Compact context'),
          ),
        if (onOpenDetails != null)
          MenuItemButton(
            key: const Key('context-details'),
            leadingIcon: const Icon(Icons.analytics_outlined, size: 18),
            onPressed: onOpenDetails,
            child: const Text('Context breakdown'),
          ),
      ],
      builder: (context, controller, _) => InkWell(
        key: const Key('composer-context-pill'),
        borderRadius: BorderRadius.circular(8),
        onTap: onCompact == null && onOpenDetails == null
            ? null
            : () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: tooltip,
          child: ComposerPill(
            label: label,
            icon: Icons.data_usage,
            danger: danger,
            status: warn && !danger,
          ),
        ),
      ),
    );
  }
}
