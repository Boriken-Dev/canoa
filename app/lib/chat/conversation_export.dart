import 'dart:async';
import 'dart:convert';

import 'package:file_selector/file_selector.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';

import '../session_store.dart';

/// Takes a conversation's Markdown under a suggested file name; answers whether it was saved.
typedef MarkdownExporter = Future<bool> Function(String suggestedName, String markdown);

/// Replaces the platform's save dialog, so a widget test can read what would be saved.
@visibleForTesting
MarkdownExporter? markdownExporterForTest;

/// Formats [session] in reading order as a Markdown document, keeping code blocks, headings, and tool titles intact.
String conversationMarkdown(ChatSession session) {
  final out = <String>[
    '# ${session.title}',
    '_Agent: ${session.agent}${session.cwd == null || session.cwd!.isEmpty ? '' : ' · Workspace: ${session.cwd}'}_',
  ];
  for (final message in session.messages) {
    switch (message.role) {
      case MessageRole.user:
        if (message.text.trim().isNotEmpty) out.add('## You\n\n${message.text}');
      case MessageRole.agent:
        if (message.text.trim().isNotEmpty) out.add('## ${session.agent}\n\n${message.text}');
      case MessageRole.tool:
        if (message.tool case final tool?) {
          final paths = tool.changedPaths.isNotEmpty ? tool.changedPaths : tool.locations;
          final suffix = paths.isEmpty ? '' : ' (${paths.join(', ')})';
          out.add('- **Tool (${tool.status.name})**: ${tool.title}$suffix');
        }
      case MessageRole.thought:
        if (message.text.trim().isNotEmpty) out.add('<details>\n<summary>Thinking</summary>\n\n${message.text}\n\n</details>');
      case MessageRole.compaction:
        if (message.compaction case final c?) {
          out.add(c.summary.isEmpty ? '_Context compacted_' : '_Context compacted: ${c.summary}_');
        }
      case MessageRole.notice:
        if (message.text.trim().isNotEmpty) out.add('> ${message.text.replaceAll('\n', '\n> ')}');
    }
  }
  return '${out.join('\n\n')}\n';
}

/// Sanitizes [title] into a portable `.md` filename.
String safeMarkdownFilename(String title) {
  final cleaned = title.trim().replaceAll(RegExp(r'[^A-Za-z0-9._ -]+'), '-').replaceAll(RegExp(r'\s+'), '-').replaceAll(RegExp(r'-+'), '-');
  final base = cleaned.replaceAll(RegExp(r'^[-.]+|[-.]+$'), '');
  return '${base.isEmpty ? 'conversation' : base}.md';
}

/// Copies [session] as Markdown to the clipboard and shows a confirmation notice.
Future<void> copyConversationMarkdown(BuildContext context, ChatSession session) async {
  await Clipboard.setData(ClipboardData(text: conversationMarkdown(session)));
  if (!context.mounted) return;
  ScaffoldMessenger.maybeOf(context)?.showSnackBar(
    const SnackBar(content: Text('Conversation copied as Markdown'), duration: Duration(seconds: 2)),
  );
}

/// Saves [session] as a `.md` file using the platform save dialog (or [markdownExporterForTest] in a test).
Future<void> exportConversationMarkdown(BuildContext context, ChatSession session) async {
  final content = conversationMarkdown(session);
  final suggested = safeMarkdownFilename(session.title);
  final messenger = ScaffoldMessenger.maybeOf(context);
  if (markdownExporterForTest case final exporter?) {
    final saved = await exporter(suggested, content);
    if (saved) messenger?.showSnackBar(SnackBar(content: Text('Exported $suggested'), duration: const Duration(seconds: 2)));
    return;
  }
  final location = await getSaveLocation(
    suggestedName: suggested,
    acceptedTypeGroups: const [XTypeGroup(label: 'Markdown', extensions: ['md'])],
  );
  if (location == null) return;
  final file = XFile.fromData(Uint8List.fromList(utf8.encode(content)), mimeType: 'text/markdown', name: suggested);
  await file.saveTo(location.path);
  messenger?.showSnackBar(SnackBar(content: Text('Exported $suggested'), duration: const Duration(seconds: 2)));
}
