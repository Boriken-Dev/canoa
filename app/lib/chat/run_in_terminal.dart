import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ToolCall;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

/// The command card of an agent's `execute` tool call with "Run in terminal" under it, which types the command into the person's
/// terminal without pressing Enter. [offered] says whether a terminal is there to type into; [run] types the command.
ChatRenderers terminalRenderers({required bool Function() offered, required void Function(String command) run}) => ChatRenderers(
  tool: {
    'execute': (context, tool) => RunInTerminalCard(tool: tool, offered: offered(), onRun: run),
  },
);

/// A tool card, and under a command's the way to try that command in the terminal.
class RunInTerminalCard extends StatelessWidget {
  const RunInTerminalCard({super.key, required this.tool, required this.offered, required this.onRun});

  final ToolCall tool;
  final bool offered;
  final void Function(String command) onRun;

  @override
  Widget build(BuildContext context) {
    final command = tool.title.trim();
    if (!offered || command.isEmpty) return ToolCard(tool: tool);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      ToolCard(tool: tool),
      Align(
        alignment: Alignment.centerRight,
        child: TextButton.icon(
          key: ValueKey('run in terminal ${tool.id}'),
          style: TextButton.styleFrom(visualDensity: VisualDensity.compact, textStyle: const TextStyle(fontSize: 12)),
          onPressed: () => onRun(command),
          icon: const Icon(Icons.terminal, size: 14),
          label: const Text('Run in terminal'),
        ),
      ),
    ]);
  }
}
