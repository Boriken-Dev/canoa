import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:markdown/markdown.dart' as md;

import '../highlight/highlighted.dart';
import '../highlight/syntax.dart' show languageForPath;
import '../voice/reply_speaker.dart';
import 'outputs.dart';
import 'reply_voice.dart';

final RegExp _trailingRange = RegExp(r'#L\d+(?:C\d+)?(?:-L?\d+(?:C\d+)?)?$', caseSensitive: false);
final RegExp _trailingLineCol = RegExp(r':\d+(?::\d+)?$');
final RegExp _windowsDrive = RegExp(r'^[A-Za-z]:[\\/]');
final RegExp _scheme = RegExp(r'^[A-Za-z][A-Za-z0-9+.-]*:');
final RegExp _fileWithExt = RegExp(r'^[A-Za-z0-9_.\-/\\]+?\.[A-Za-z0-9]{1,12}$');

/// Strips `:line(:col)` or `#Lline(-Lline)` from [raw], leaving a plain path candidate.
String _stripPosition(String raw) => raw.replaceFirst(_trailingRange, '').replaceFirst(_trailingLineCol, '');

/// Resolves [href] to a local workspace path (joined to [root] when relative), or returns null when [href] is an external URL, a
/// fragment, or does not name a file path.
String? resolveFileLink(String href, {String? root}) {
  final trimmed = href.trim();
  if (trimmed.isEmpty || trimmed.startsWith('#')) return null;

  if (trimmed.toLowerCase().startsWith('file://')) {
    final uri = Uri.tryParse(trimmed);
    if (uri == null) return null;
    try {
      final path = _stripPosition(uri.toFilePath());
      if (path.isEmpty) return null;
      return resolvedIn(root, path);
    } on UnsupportedError {
      return null;
    }
  }

  if (_scheme.hasMatch(trimmed) && !_windowsDrive.hasMatch(trimmed)) return null;

  final path = _stripPosition(trimmed);
  if (path.isEmpty || path.contains('\n') || path.contains('\r')) return null;
  if (path.startsWith('//') || path.contains('://')) return null;

  final looksLikePath = path.contains('/') || path.contains('\\') || _fileWithExt.hasMatch(path);
  if (!looksLikePath) return null;
  return resolvedIn(root, path);
}

/// Whether [text] inside inline code looks like a file path worth turning into a link into Files.
bool looksLikeFilePath(String text) {
  final trimmed = text.trim();
  if (trimmed.length < 2 || trimmed.length > 260) return false;
  if (trimmed.contains('\n') || trimmed.contains('\r') || RegExp(r'\s').hasMatch(trimmed)) return false;
  return resolveFileLink(trimmed) != null;
}

/// Distinct resolved file paths referenced by [tool] through `changedPaths` and `locations`.
List<String> toolFilePaths(ToolCall tool, {String? root}) {
  final seen = <String>{};
  final paths = <String>[];
  void add(String raw) {
    final resolved = resolveFileLink(raw, root: root);
    if (resolved != null && seen.add(resolved)) paths.add(resolved);
  }

  for (final path in tool.changedPaths) {
    add(path);
  }
  for (final place in tool.locations) {
    add(place);
  }
  return paths;
}

/// Renderers that turn file links and file-path code spans in agent replies, tool locations, and diff blocks into clicks that open the
/// file in Files (while preserving clipboard fallback for external links and Play/Copy buttons).
ChatRenderers filePathRenderers({
  required ValueChanged<String>? onOpenFile,
  String? root,
  ReplySpeaker? reader,
}) {
  final inlineSyntaxes = onOpenFile == null ? const <md.InlineSyntax>[] : <md.InlineSyntax>[_InlineCodePathSyntax()];

  void handleLink(BuildContext context, String href) {
    final open = onOpenFile;
    final resolved = open == null ? null : resolveFileLink(href, root: root);
    if (resolved != null && open != null) {
      open(resolved);
      return;
    }
    Clipboard.setData(ClipboardData(text: href));
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(
      SnackBar(content: Text('Address copied: $href', maxLines: 2, overflow: TextOverflow.ellipsis)),
    );
  }

  Widget drawAgent(BuildContext context, ChatEntryData entry) {
    final look = context.chatLook;
    final markdown = AgentMarkdown(
      text: entry.text,
      inlineSyntaxes: inlineSyntaxes,
      builders: highlightedMarkdown,
      onTapLink: (href, _) => handleLink(context, href),
    );
    if (reader != null && reader.available) {
      return Column(crossAxisAlignment: CrossAxisAlignment.end, mainAxisSize: MainAxisSize.min, children: [
        MessageBox(color: look.agent, inBubble: look.agentInBubble, child: markdown),
        Row(mainAxisSize: MainAxisSize.min, children: [
          PlayReplyButton(reader: reader, replyKey: replyKey(entry.at), text: entry.text),
          CopyButton(text: entry.text, tooltip: 'Copy message'),
        ]),
      ]);
    }
    return MessageBox(color: look.agent, inBubble: look.agentInBubble, copy: entry.text, child: markdown);
  }

  Widget drawTool(BuildContext context, ToolCall tool) => _ClickableToolCard(tool: tool, root: root, onOpenFile: onOpenFile);

  return ChatRenderers(
    entry: [
      (context, entry) => entry.role == MessageRole.agent ? drawAgent(context, entry) : null,
    ],
    tool: {
      if (onOpenFile != null) ...{
        'read': drawTool,
        'edit': drawTool,
        'delete': drawTool,
        'move': drawTool,
        'search': drawTool,
      },
    },
    block: [
      if (onOpenFile != null)
        (context, tool, block) {
          if (block is! ToolDiff) return null;
          final resolved = resolvedIn(root, block.path);
          return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            _FilePathLink(
              key: ValueKey('diff-file-link ${block.path}'),
              label: block.path,
              tooltip: 'Open ${block.path} in Files',
              onTap: () => onOpenFile(resolved),
            ),
            const SizedBox(height: 4),
            HighlightedDiffView(lines: withContext(diffLines(block.oldText, block.newText)), language: languageForPath(block.path)),
          ]);
        },
    ],
  );
}

class _InlineCodePathSyntax extends md.InlineSyntax {
  _InlineCodePathSyntax() : super(r'`([^`\s\r\n]+)`', startCharacter: 0x60);

  @override
  bool tryMatch(md.InlineParser parser, [int? startMatchPos]) {
    final at = startMatchPos ?? parser.pos;
    if (parser.source.codeUnitAt(at) != 0x60) return false;
    final m = pattern.matchAsPrefix(parser.source, at);
    if (m == null || !looksLikeFilePath(m.group(1)!)) return false;
    return super.tryMatch(parser, startMatchPos);
  }

  @override
  bool onMatch(md.InlineParser parser, Match match) {
    final text = match.group(1)!;
    final anchor = md.Element('a', [md.Element.text('code', text)])..attributes['href'] = text;
    parser.addNode(anchor);
    return true;
  }
}

class _FilePathLink extends StatelessWidget {
  const _FilePathLink({super.key, required this.label, required this.tooltip, required this.onTap});

  final String label;
  final String tooltip;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(4),
        child: Text(
          label,
          style: TextStyle(
            fontFamily: look.codeFont,
            fontSize: 12,
            color: look.accent,
            decoration: TextDecoration.underline,
            decorationColor: look.accent,
          ),
        ),
      ),
    );
  }
}

class _ClickableToolCard extends StatelessWidget {
  const _ClickableToolCard({required this.tool, required this.root, required this.onOpenFile});

  final ToolCall tool;
  final String? root;
  final ValueChanged<String>? onOpenFile;

  @override
  Widget build(BuildContext context) {
    final paths = toolFilePaths(tool, root: root);
    if (onOpenFile == null || paths.isEmpty) return ToolCard(tool: tool);

    final look = context.chatLook;
    final (statusIcon, statusColor, statusLabel) = switch (tool.status) {
      ToolStatus.pending => (Icons.schedule, look.muted, 'waiting'),
      ToolStatus.running => (Icons.autorenew, look.accent, 'running'),
      ToolStatus.done => (Icons.check_circle_outline, look.accent, 'done'),
      ToolStatus.failed => (Icons.error_outline, look.danger, 'failed'),
      ToolStatus.cancelled => (Icons.block, look.muted, 'cancelled'),
      ToolStatus.stopped => (Icons.stop_circle_outlined, look.muted, 'stopped'),
    };
    final hasDetail = tool.blocks.isNotEmpty || tool.locations.isNotEmpty;
    final firstPath = paths.first;
    final header = Row(children: [
      Icon(toolIcon(tool.kind), size: 18, color: look.muted),
      const SizedBox(width: 8),
      Expanded(child: Text(tool.title, maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: look.bodySize - 1))),
      const SizedBox(width: 4),
      IconButton(
        key: ValueKey('tool-open-file $firstPath'),
        tooltip: 'Open in Files',
        icon: const Icon(Icons.open_in_new, size: 14),
        visualDensity: VisualDensity.compact,
        padding: EdgeInsets.zero,
        constraints: const BoxConstraints.tightFor(width: 24, height: 24),
        color: look.muted,
        onPressed: () => onOpenFile!(firstPath),
      ),
      if (tool.status == ToolStatus.stopped) ...[
        Text('stopped', style: TextStyle(fontSize: look.bodySize - 2, color: look.muted)),
        const SizedBox(width: 4),
      ],
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
                      child: () {
                        final resolved = resolveFileLink(place, root: root);
                        if (resolved != null) {
                          return Align(
                            alignment: Alignment.centerLeft,
                            child: _FilePathLink(
                              key: ValueKey('tool-location-link $place'),
                              label: place,
                              tooltip: 'Open $place in Files',
                              onTap: () => onOpenFile!(resolved),
                            ),
                          );
                        }
                        return Text(place, style: TextStyle(fontFamily: look.codeFont, fontSize: 12, color: look.muted));
                      }(),
                    ),
                  for (final block in tool.blocks) ToolBlockView(tool: tool, block: block),
                ],
              ),
            ),
    );
  }
}
