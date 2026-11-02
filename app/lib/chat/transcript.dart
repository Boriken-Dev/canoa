import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

import '../session_store.dart';
import '../theme/look.dart';
import 'agent_events.dart';
import 'diff.dart';

/// Markdown from an agent, which is untrusted. An image would be fetched as soon as it is drawn, which lets whatever wrote it
/// send data to any address, so only its description is shown. A link is copied rather than opened, so the address is seen first.
class AgentMarkdown extends StatelessWidget {
  const AgentMarkdown({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final base = Theme.of(context).textTheme.bodyMedium?.copyWith(fontSize: look.bodySize, height: 1.45);
    final code = base?.copyWith(fontFamily: CanoaLook.monospaceFont, fontSize: look.bodySize - 1);
    return MarkdownBody(
      data: text,
      selectable: true,
      styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context)).copyWith(
        p: base,
        code: code?.copyWith(backgroundColor: look.code),
        codeblockDecoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
        blockquoteDecoration: BoxDecoration(
          color: look.notice,
          border: Border(left: BorderSide(color: look.muted, width: 3)),
        ),
      ),
      imageBuilder: (uri, title, alt) => Text(
        alt?.isNotEmpty ?? false ? '[image: $alt]' : '[image]',
        style: TextStyle(color: look.muted),
      ),
      onTapLink: (label, href, title) {
        if (href == null) return;
        Clipboard.setData(ClipboardData(text: href));
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          SnackBar(content: Text('Address copied: $href', maxLines: 2, overflow: TextOverflow.ellipsis)),
        );
      },
    );
  }
}

/// One entry of a conversation, drawn by its kind and the look in use.
class TranscriptEntry extends StatelessWidget {
  const TranscriptEntry({super.key, required this.message, required this.session});

  final ChatMessage message;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final body = switch (message.role) {
      MessageRole.tool => message.tool == null ? const SizedBox.shrink() : ToolCard(tool: message.tool!, session: session),
      MessageRole.thought => _Thought(text: message.text),
      MessageRole.agent => _Message(color: look.agent, inBubble: look.agentInBubble, child: AgentMarkdown(text: message.text)),
      MessageRole.user => _Message(
        color: look.user,
        inBubble: look.userPrefix == null,
        child: SelectableText(
          '${look.userPrefix ?? ''}${message.text}',
          style: TextStyle(fontSize: look.bodySize, height: 1.45, color: look.userPrefix == null ? null : look.accent),
        ),
      ),
      MessageRole.notice => _Message(
        color: look.notice,
        child: Text(message.text, style: TextStyle(fontSize: look.bodySize - 2, color: look.muted, height: 1.45)),
      ),
    };
    final toTheRight = message.role == MessageRole.user && look.userToTheRight;
    return Align(
      alignment: toTheRight ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxWidth: look.messageWidth),
        child: Padding(padding: EdgeInsets.only(bottom: look.gap), child: body),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.color, required this.child, this.inBubble = true});

  final Color color;
  final bool inBubble;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    if (!inBubble) return child;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: look.padding + 2, vertical: look.padding - 2),
      decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(look.radius)),
      child: child,
    );
  }
}

/// An agent's thinking, kept out of the way until asked for.
class _Thought extends StatelessWidget {
  const _Thought({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Theme(
      data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
      child: ExpansionTile(
        dense: true,
        tilePadding: EdgeInsets.zero,
        leading: Icon(Icons.psychology_outlined, size: 18, color: look.muted),
        title: Text('Thinking', style: TextStyle(color: look.muted, fontSize: look.bodySize - 2)),
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: SelectableText(text, style: TextStyle(color: look.muted, fontSize: look.bodySize - 1, height: 1.4)),
          ),
        ],
      ),
    );
  }
}

IconData _toolIcon(String? kind) => switch (kind) {
  'read' => Icons.description_outlined,
  'edit' => Icons.edit_outlined,
  'delete' => Icons.delete_outline,
  'move' => Icons.drive_file_move_outline,
  'search' => Icons.search,
  'execute' => Icons.terminal,
  'fetch' => Icons.public,
  'think' => Icons.psychology_outlined,
  _ => Icons.build_outlined,
};

/// A tool the agent is using: what, how it went, and what it changed. A failure opens by itself, since that is what a person came
/// to see, and the rest stay closed to keep a long turn readable.
class ToolCard extends StatelessWidget {
  const ToolCard({super.key, required this.tool, required this.session});

  final ToolCall tool;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final (statusIcon, statusColor, statusLabel) = switch (tool.status) {
      ToolStatus.pending => (Icons.schedule, look.muted, 'waiting'),
      ToolStatus.running => (Icons.autorenew, look.accent, 'running'),
      ToolStatus.done => (Icons.check_circle_outline, look.accent, 'done'),
      ToolStatus.failed => (Icons.error_outline, Theme.of(context).colorScheme.error, 'failed'),
    };
    final hasDetail = tool.blocks.isNotEmpty || tool.locations.isNotEmpty;
    final header = Row(children: [
      Icon(_toolIcon(tool.kind), size: 18, color: look.muted),
      const SizedBox(width: 8),
      Expanded(child: Text(tool.title, maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: look.bodySize - 1))),
      const SizedBox(width: 8),
      Tooltip(message: statusLabel, child: Icon(statusIcon, size: 16, color: statusColor, semanticLabel: statusLabel)),
    ]);
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: look.muted.withValues(alpha: 0.3)),
        borderRadius: BorderRadius.circular(look.radius / 1.5),
      ),
      clipBehavior: Clip.antiAlias,
      child: !hasDetail
          ? Padding(padding: EdgeInsets.all(look.padding), child: header)
          : Theme(
              data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
              child: ExpansionTile(
                // Rebuilt as the status changes, so a tool that fails while being watched opens.
                key: ValueKey('${tool.id}-${tool.status == ToolStatus.failed}'),
                initiallyExpanded: tool.status == ToolStatus.failed,
                dense: true,
                tilePadding: EdgeInsets.symmetric(horizontal: look.padding),
                childrenPadding: EdgeInsets.fromLTRB(look.padding, 0, look.padding, look.padding),
                expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
                title: header,
                children: [
                  for (final place in tool.locations)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 4),
                      child: SelectableText(place, style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: look.muted)),
                    ),
                  for (final block in tool.blocks) _ToolBlockView(block: block, session: session),
                ],
              ),
            ),
    );
  }
}

class _ToolBlockView extends StatelessWidget {
  const _ToolBlockView({required this.block, required this.session});

  final ToolBlock block;
  final ChatSession session;

  @override
  Widget build(BuildContext context) => switch (block) {
    ToolText(:final text) => _Code(text: text),
    ToolDiff(:final path, :final oldText, :final newText) => Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(path, style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: context.look.muted)),
        const SizedBox(height: 4),
        DiffView(lines: withContext(diffLines(oldText, newText))),
      ],
    ),
    ToolPatch(:final text) => DiffView(lines: patchLines(text)),
    ToolTerminal(:final terminalId) => _TerminalBlock(view: session.terminals[terminalId]),
  };
}

/// A unified patch as lines. The headers are context, and null marks where a hunk starts.
List<DiffLine?> patchLines(String patch) {
  final out = <DiffLine?>[];
  for (final line in patch.split('\n')) {
    if (line.startsWith('@@')) {
      out.add(null);
    } else if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ') || line.startsWith('index ')) {
      out.add(DiffLine(DiffKind.same, line));
    } else if (line.startsWith('+')) {
      out.add(DiffLine(DiffKind.added, line.substring(1)));
    } else if (line.startsWith('-')) {
      out.add(DiffLine(DiffKind.removed, line.substring(1)));
    } else if (line.isNotEmpty) {
      out.add(DiffLine(DiffKind.same, line.startsWith(' ') ? line.substring(1) : line));
    }
  }
  return out;
}

class DiffView extends StatelessWidget {
  const DiffView({super.key, required this.lines});

  final List<DiffLine?> lines;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    const style = TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, height: 1.35);
    return Container(
      constraints: const BoxConstraints(maxHeight: 420),
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      child: SingleChildScrollView(
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: SelectionArea(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                for (final line in lines)
                  if (line == null)
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      child: Text('⋯', style: style.copyWith(color: look.muted)),
                    )
                  else
                    Container(
                      color: switch (line.kind) {
                        DiffKind.added => look.added,
                        DiffKind.removed => look.removed,
                        DiffKind.same => null,
                      },
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      child: Text(
                        '${switch (line.kind) { DiffKind.added => '+', DiffKind.removed => '-', DiffKind.same => ' ' }} ${line.text}',
                        style: style,
                        softWrap: false,
                      ),
                    ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _Code extends StatelessWidget {
  const _Code({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Container(
      constraints: const BoxConstraints(maxHeight: 320),
      width: double.infinity,
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      child: SingleChildScrollView(
        child: SelectableText(text, style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, height: 1.35)),
      ),
    );
  }
}

class _TerminalBlock extends StatelessWidget {
  const _TerminalBlock({required this.view});

  final TerminalView? view;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final view = this.view;
    if (view == null) return const SizedBox.shrink();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (view.command != null)
          SelectableText('\$ ${view.command}', style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: look.accent)),
        if (view.output.isNotEmpty) Padding(padding: const EdgeInsets.only(top: 4), child: _Code(text: view.output)),
        if (view.exitCode != null && view.exitCode != 0)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text('exit ${view.exitCode}', style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12)),
          ),
      ],
    );
  }
}

/// What the agent says it is going to do, above the composer while it does it.
class PlanPanel extends StatelessWidget {
  const PlanPanel({super.key, required this.entries});

  final List<PlanEntry> entries;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final done = entries.where((entry) => entry.status == PlanStatus.done).length;
    return Theme(
      data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
      child: ExpansionTile(
        dense: true,
        leading: Icon(Icons.checklist, size: 18, color: look.muted),
        title: Text('Plan: $done of ${entries.length}', style: TextStyle(fontSize: look.bodySize - 2, color: look.muted)),
        children: [
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 220),
            child: ListView(
              shrinkWrap: true,
              children: [
                for (final entry in entries)
                  ListTile(
                    dense: true,
                    visualDensity: VisualDensity.compact,
                    leading: Icon(
                      switch (entry.status) {
                        PlanStatus.done => Icons.check_circle_outline,
                        PlanStatus.running => Icons.play_circle_outline,
                        PlanStatus.pending => Icons.radio_button_unchecked,
                      },
                      size: 16,
                      color: entry.status == PlanStatus.pending ? look.muted : look.accent,
                    ),
                    title: Text(entry.content, style: TextStyle(fontSize: look.bodySize - 2)),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The agent's own choices, such as its model and mode, and what the session has used.
class SessionControls extends StatelessWidget {
  const SessionControls({super.key, required this.session, required this.onChoose});

  final ChatSession session;
  final void Function(ConfigOption option, String value) onChoose;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final usage = session.usage.describe();
    if (session.options.isEmpty && usage == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Wrap(
        spacing: 12,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          for (final option in session.options)
            Tooltip(
              message: option.label,
              child: DropdownButton<String>(
                value: option.current,
                isDense: true,
                underline: const SizedBox.shrink(),
                style: TextStyle(fontSize: look.bodySize - 3, color: look.muted),
                items: [
                  for (final choice in option.choices)
                    DropdownMenuItem(value: choice.value, child: Text(choice.label, overflow: TextOverflow.ellipsis)),
                ],
                onChanged: (value) {
                  if (value != null && value != option.current) onChoose(option, value);
                },
              ),
            ),
          if (usage != null) Text(usage, style: TextStyle(fontSize: look.bodySize - 3, color: look.muted)),
        ],
      ),
    );
  }
}

/// The agent's commands matching what is being typed. Nothing runs until the person sends it.
class CommandSuggestions extends StatelessWidget {
  const CommandSuggestions({super.key, required this.commands, required this.typed, required this.onPick});

  static const int shown = 6;

  final List<AgentCommandInfo> commands;
  final String typed;
  final void Function(AgentCommandInfo command) onPick;

  @override
  Widget build(BuildContext context) {
    if (!typed.startsWith('/') || typed.contains(' ')) return const SizedBox.shrink();
    final wanted = typed.substring(1).toLowerCase();
    final found = commands.where((command) => command.name.toLowerCase().startsWith(wanted)).take(shown).toList();
    if (found.isEmpty) return const SizedBox.shrink();
    final look = context.look;
    return Material(
      color: look.composer,
      borderRadius: BorderRadius.circular(look.radius / 2),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final command in found)
            ListTile(
              dense: true,
              visualDensity: VisualDensity.compact,
              title: Text('/${command.name}', style: const TextStyle(fontFamily: CanoaLook.monospaceFont)),
              subtitle: command.description.isEmpty ? null : Text(command.description, maxLines: 1, overflow: TextOverflow.ellipsis),
              onTap: () => onPick(command),
            ),
        ],
      ),
    );
  }
}
