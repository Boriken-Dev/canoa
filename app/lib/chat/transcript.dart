import 'dart:async';

import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter/foundation.dart' show defaultTargetPlatform, TargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show SelectedContent;
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:markdown/markdown.dart' as md;

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
    // The conversation's own selection takes this text in, so it is not a selection of its own.
    return MarkdownBody(
      data: text,
      builders: {'pre': _CodeBlock(look)},
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
      MessageRole.compaction =>
        message.compaction == null ? const SizedBox.shrink() : _CompactionView(compaction: message.compaction!),
      MessageRole.agent => _Message(
        color: look.agent,
        inBubble: look.agentInBubble,
        copy: message.text,
        child: AgentMarkdown(text: message.text),
      ),
      MessageRole.user => _Message(
        color: look.user,
        inBubble: look.userPrefix == null,
        copy: message.text,
        child: Text(
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
  const _Message({required this.color, required this.child, this.inBubble = true, this.copy});

  final Color color;
  final bool inBubble;
  final Widget child;

  /// What its copy button copies; none without.
  final String? copy;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final body = !inBubble
        ? child
        : Container(
            padding: EdgeInsets.symmetric(horizontal: look.padding + 2, vertical: look.padding - 2),
            decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(look.radius)),
            child: child,
          );
    if (copy == null) return body;
    return Column(crossAxisAlignment: CrossAxisAlignment.end, mainAxisSize: MainAxisSize.min, children: [
      body,
      CopyButton(text: copy!, tooltip: 'Copy message'),
    ]);
  }
}

/// Copies [text] as it is, and says so.
class CopyButton extends StatelessWidget {
  const CopyButton({super.key, required this.text, this.tooltip = 'Copy'});

  final String text;
  final String tooltip;

  @override
  Widget build(BuildContext context) => SelectionContainer.disabled(
    child: IconButton(
      tooltip: tooltip,
      iconSize: 15,
      visualDensity: VisualDensity.compact,
      color: context.look.muted,
      onPressed: () async {
        await Clipboard.setData(ClipboardData(text: text));
        if (context.mounted) {
          ScaffoldMessenger.maybeOf(context)?.showSnackBar(const SnackBar(content: Text('Copied'), duration: Duration(seconds: 1)));
        }
      },
      icon: const Icon(Icons.copy_rounded),
    ),
  );
}

/// A code block in an agent's Markdown, with a button that copies its code alone.
class _CodeBlock extends MarkdownElementBuilder {
  _CodeBlock(this.look);
  final CanoaLook look;

  @override
  bool isBlockElement() => true;

  @override
  Widget? visitElementAfterWithContext(BuildContext context, md.Element element, TextStyle? preferredStyle, TextStyle? parentStyle) {
    var code = element.textContent;
    if (code.endsWith('\n')) code = code.substring(0, code.length - 1);
    return Container(
      width: double.infinity,
      margin: const EdgeInsets.symmetric(vertical: 4),
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      child: Stack(children: [
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          padding: const EdgeInsets.fromLTRB(10, 10, 40, 10),
          child: Text(code, style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: look.bodySize - 1, height: 1.35)),
        ),
        Positioned(top: 0, right: 0, child: CopyButton(text: code, tooltip: 'Copy code')),
      ]),
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
            child: Text(text, style: TextStyle(color: look.muted, fontSize: look.bodySize - 1, height: 1.4)),
          ),
        ],
      ),
    );
  }
}

/// The agent making room in its context. What it kept, its summary, is folded away like a thought.
class _CompactionView extends StatelessWidget {
  const _CompactionView({required this.compaction});

  final Compaction compaction;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final said = switch (compaction.status) {
      CompactionStatus.running => 'Compacting the context…',
      CompactionStatus.done => 'Context compacted',
      CompactionStatus.failed => 'Compacting the context failed${compaction.error == null ? '' : ': ${compaction.error}'}',
      CompactionStatus.cancelled => 'Compacting the context was stopped',
    };
    final icon = Icon(Icons.compress, size: 18, color: look.muted);
    final title = Text(said, style: TextStyle(color: look.muted, fontSize: look.bodySize - 2));
    if (compaction.summary.isEmpty) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Row(children: [icon, const SizedBox(width: 16), Expanded(child: title)]),
      );
    }
    return Theme(
      data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
      child: ExpansionTile(
        dense: true,
        tilePadding: EdgeInsets.zero,
        leading: icon,
        title: title,
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: Text(compaction.summary, style: TextStyle(color: look.muted, fontSize: look.bodySize - 1, height: 1.4)),
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
      ToolStatus.cancelled => (Icons.block, look.muted, 'cancelled'),
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
                      child: Text(place, style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: look.muted)),
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
        child: Text(text, style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, height: 1.35)),
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
          Text('\$ ${view.command}', style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: look.accent)),
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
                        PlanStatus.cancelled => Icons.block,
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

/// What the session has used, as the agent says. Its choices are in the row under the message.
class SessionControls extends StatelessWidget {
  const SessionControls({super.key, required this.session});

  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final usage = session.usage.describe();
    if (usage == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Text(usage, style: TextStyle(fontSize: look.bodySize - 3, color: look.muted)),
    );
  }
}

/// Markdown as the text it shows, a block to a line: a code block is its code, and no mark of the Markdown is left.
String plainMarkdown(String text) {
  final nodes = md.Document(extensionSet: md.ExtensionSet.gitHubFlavored).parseLines(text.split('\n'));
  return [
    for (final node in nodes)
      if (node.textContent.trimRight() case final shown when shown.isNotEmpty) shown,
  ].join('\n');
}

/// A conversation's text in reading order, as copying all of it gives it: each message, notice and tool by what it shows,
/// and a folded thought or summary by its heading.
String conversationText(ChatSession session) => [
  for (final message in session.messages)
    switch (message.role) {
      MessageRole.agent => plainMarkdown(message.text),
      MessageRole.user || MessageRole.notice => message.text,
      MessageRole.thought => 'Thinking',
      MessageRole.tool => message.tool?.title ?? '',
      MessageRole.compaction => 'Context compacted',
    },
].where((text) => text.isNotEmpty).join('\n\n');

/// The conversation as one selection: dragging selects across messages, tool cards, code and diffs, Ctrl+A (⌘A on a Mac)
/// selects all of it and Ctrl+C copies, and the context menu offers Copy and Select all. Entries the list has not built are
/// in it too: after Select all, Copy gives the whole conversation's text.
class SelectableTranscript extends StatefulWidget {
  const SelectableTranscript({super.key, required this.session, required this.child});

  final ChatSession session;
  final Widget child;

  @override
  State<SelectableTranscript> createState() => SelectableTranscriptState();
}

class SelectableTranscriptState extends State<SelectableTranscript> {
  late final FocusNode _focus = FocusNode(debugLabel: 'transcript', onKeyEvent: _onKey);

  /// Whether the selection is all of it: set by Select all, and ended by any other change.
  bool _all = false;
  bool _selectingAll = false;
  String? _allShown;

  bool get _mac => defaultTargetPlatform == TargetPlatform.macOS || defaultTargetPlatform == TargetPlatform.iOS;

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    if (!(_mac ? keys.isMetaPressed : keys.isControlPressed) || keys.isShiftPressed || keys.isAltPressed) {
      return KeyEventResult.ignored;
    }
    if (event.logicalKey == LogicalKeyboardKey.keyA) {
      _startSelectingAll();
    } else if (event.logicalKey == LogicalKeyboardKey.keyC && _all) {
      unawaited(copyAll());
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// Puts the whole conversation's text on the clipboard.
  Future<void> copyAll() => Clipboard.setData(ClipboardData(text: conversationText(widget.session)));

  /// Select all is under way until the frame after it: what the selection reports meanwhile is all of it.
  void _startSelectingAll() {
    _selectingAll = true;
    _all = true;
    WidgetsBinding.instance.addPostFrameCallback((_) => _selectingAll = false);
  }

  void _changed(SelectedContent? content) {
    if (_selectingAll) {
      _all = content != null;
      _allShown = content?.plainText;
    } else if (_all && content?.plainText != _allShown) {
      _all = false;
    }
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SelectionArea(
    focusNode: _focus,
    onSelectionChanged: _changed,
    contextMenuBuilder: (context, region) => AdaptiveTextSelectionToolbar.buttonItems(
      anchors: region.contextMenuAnchors,
      buttonItems: [
        for (final item in region.contextMenuButtonItems)
          switch (item.type) {
            ContextMenuButtonType.copy => item.copyWith(onPressed: () {
              if (_all) {
                unawaited(copyAll());
                region.hideToolbar();
              } else {
                item.onPressed?.call();
              }
            }),
            ContextMenuButtonType.selectAll => item.copyWith(onPressed: () {
              _startSelectingAll();
              item.onPressed?.call();
            }),
            _ => item,
          },
      ],
    ),
    child: widget.child,
  );
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

/// What an agent says it has from before, for a person to pick one. Titles are the agent's, and the list asks for more only when told.
class EarlierConversations extends StatefulWidget {
  const EarlierConversations({super.key, required this.list});

  final Future<AcpSessionPage> Function(String? cursor) list;

  @override
  State<EarlierConversations> createState() => _EarlierConversationsState();
}

class _EarlierConversationsState extends State<EarlierConversations> {
  final List<AcpSessionInfo> _found = [];
  String? _cursor;
  bool _loading = true;
  String? _problem;

  @override
  void initState() {
    super.initState();
    unawaited(_more());
  }

  Future<void> _more() async {
    setState(() {
      _loading = true;
      _problem = null;
    });
    try {
      final page = await widget.list(_cursor);
      if (!mounted) return;
      setState(() {
        _found.addAll(page.sessions);
        _cursor = page.nextCursor;
        _loading = false;
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _problem = 'This connection does not offer that.';
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Earlier conversations'),
      content: SizedBox(
        width: 520,
        height: 420,
        child: Column(children: [
          if (_problem != null)
            Padding(padding: const EdgeInsets.all(8), child: Text(_problem!))
          else if (!_loading && _found.isEmpty)
            const Padding(padding: EdgeInsets.all(8), child: Text('The agent has none here.')),
          Expanded(
            child: ListView.builder(
              itemCount: _found.length,
              itemBuilder: (context, index) {
                final earlier = _found[index];
                final about = [shortText(earlier.updatedAt), shortText(earlier.cwd)].whereType<String>().join(' · ');
                return ListTile(
                  title: Text(shortText(earlier.title) ?? 'Untitled', maxLines: 2, overflow: TextOverflow.ellipsis),
                  subtitle: about.isEmpty ? null : Text(about, maxLines: 1, overflow: TextOverflow.ellipsis),
                  onTap: () => Navigator.of(context).pop(earlier),
                );
              },
            ),
          ),
          if (_loading) const Padding(padding: EdgeInsets.all(8), child: CircularProgressIndicator()),
          if (!_loading && _cursor != null)
            TextButton(onPressed: () => unawaited(_more()), child: const Text('More')),
        ]),
      ),
      actions: [TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Close'))],
    );
  }
}
