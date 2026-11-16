import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/canoa_mark.dart';
import 'composer.dart';

/// A session before its first message: the mark, Chat or Workspace and, for a workspace, the folder it works in, and the composer
/// in the middle, whose row holds the source, Local or Worktree and the agent's choices. Nothing has started; sending starts it.
class NewSessionView extends StatelessWidget {
  const NewSessionView({super.key, required this.store, required this.session, required this.kit, required this.composer});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  /// The composer, built by the conversation that owns its text.
  final Widget composer;

  /// How many recent folders are offered as rows; the folder menu keeps the rest.
  static const recentShown = 5;

  Future<void> _open(BuildContext context) async {
    final folder = await kit.pickFolder?.call();
    if (folder != null) store.updateDraft(session, cwd: folder);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final theme = Theme.of(context);
    final cwd = session.cwd;
    // Where a chat works is Canoa's to make, so the view never shows it; a build that cannot make it has workspaces only.
    final canChat = kit.scratchFolder != null;
    final chat = canChat && session.kind == SessionKind.chat;
    final folders = {?cwd, ...kit.recentFolders}.toList();
    final draft = session.status == SessionStatus.draft;
    final recent = draft && !chat ? kit.recentFolders.take(recentShown).toList() : const <String>[];
    final notices = session.messages.where((message) => message.role == MessageRole.notice).toList();
    final folder = PopupMenuButton<String>(
      key: const Key('new-session-folder'),
      tooltip: 'The workspace it works in',
      enabled: session.status == SessionStatus.draft,
      onSelected: (chosen) => chosen.isEmpty ? unawaited(_open(context)) : store.updateDraft(session, cwd: chosen),
      itemBuilder: (context) => [
        for (final each in folders)
          CheckedPopupMenuItem(value: each, checked: each == cwd, child: Text(each, overflow: TextOverflow.ellipsis)),
        if (folders.isNotEmpty) const PopupMenuDivider(),
        PopupMenuItem(
          key: const Key('new-session-browse'),
          value: '',
          enabled: kit.pickFolder != null,
          child: const Text('Open workspace…'),
        ),
      ],
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        decoration: BoxDecoration(
          border: Border.all(color: look.muted.withValues(alpha: 0.35)),
          borderRadius: BorderRadius.circular(look.radius),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          Icon(Icons.folder_outlined, size: 18, color: look.muted),
          const SizedBox(width: 8),
          Flexible(child: Text(cwd ?? 'Choose a workspace', overflow: TextOverflow.ellipsis, maxLines: 1)),
          Icon(Icons.expand_more, size: 18, color: look.muted),
        ]),
      ),
    );
    final kind = !canChat
        ? null
        : SegmentedButton<SessionKind>(
            key: const Key('new-session-kind'),
            showSelectedIcon: false,
            segments: const [
              ButtonSegment(value: SessionKind.chat, icon: Icon(Icons.chat_bubble_outline, size: 18), label: Text('Chat')),
              ButtonSegment(value: SessionKind.workspace, icon: Icon(Icons.folder_outlined, size: 18), label: Text('Workspace')),
            ],
            selected: {session.kind},
            onSelectionChanged: draft ? (chosen) => store.updateDraft(session, kind: chosen.first) : null,
          );
    final source = session.source;
    final says = source == null ? null : store.choicesOf(source)?.cacheLifetime;
    final cacheNote = says == null ? null : 'Prompt cache: ${store.cacheLifetimeOf(session).lifetime.label.toLowerCase()}. Provider: $says';
    return LayoutBuilder(builder: (context, constraints) => SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 24),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: constraints.maxHeight - 48),
        child: Center(
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
            child: Column(mainAxisSize: MainAxisSize.min, children: [
              CanoaMark(size: 56, color: look.accent),
              const SizedBox(height: 16),
              Text('NEW SESSION', style: theme.textTheme.labelSmall?.copyWith(color: look.muted, letterSpacing: 1.4)),
              const SizedBox(height: 6),
              Text('What should we work on?', textAlign: TextAlign.center, style: theme.textTheme.headlineSmall),
              const SizedBox(height: 14),
              ?kind,
              if (!chat) ...[
                if (kind != null) const SizedBox(height: 10),
                folder,
              ],
              const SizedBox(height: 20),
              composer,
              if (draft) _RecentFolders(
                folders: recent,
                chosen: cwd,
                onChoose: (path) => store.updateDraft(session, cwd: path),
                onBrowse: kit.pickFolder == null ? null : () => unawaited(_open(context)),
                onSources: kit.onSourceSettings,
              ),
              if (cacheNote != null)
                Padding(
                  padding: const EdgeInsets.only(top: 10),
                  child: Text(cacheNote, key: const Key('new-session-cache-note'), textAlign: TextAlign.center,
                      style: theme.textTheme.bodySmall?.copyWith(color: look.muted)),
                ),
              for (final notice in notices)
                Padding(
                  padding: const EdgeInsets.only(top: 10),
                  child: Text(notice.text, textAlign: TextAlign.center,
                      style: TextStyle(color: notice.text.startsWith('Not started') ? theme.colorScheme.error : look.muted)),
                ),
            ]),
          ),
        ),
      ),
    ));
  }
}

/// The folders sessions worked in, newest first, one click each, and the quick actions beside them.
class _RecentFolders extends StatelessWidget {
  const _RecentFolders({required this.folders, required this.chosen, required this.onChoose, this.onBrowse, this.onSources});

  final List<String> folders;
  final String? chosen;
  final void Function(String folder) onChoose;
  final VoidCallback? onBrowse;
  final VoidCallback? onSources;

  /// The last part of [path], and the rest of it.
  static (String, String) _split(String path) {
    final trimmed = path.length > 1 ? path.replaceAll(RegExp(r'[\/]+$'), '') : path;
    final cut = trimmed.lastIndexOf(RegExp(r'[\/]'));
    if (cut < 0 || cut == trimmed.length - 1) return (trimmed, '');
    return (trimmed.substring(cut + 1), trimmed.substring(0, cut));
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 18),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (folders.isNotEmpty) ...[
          Padding(
            padding: const EdgeInsets.only(left: 12, bottom: 4),
            child: Text('RECENT WORKSPACES', style: theme.textTheme.labelSmall?.copyWith(color: look.muted, letterSpacing: 1.2)),
          ),
          for (final folder in folders)
            Builder(builder: (context) {
              final (name, parent) = _split(folder);
              final selected = folder == chosen;
              return InkWell(
                key: ValueKey('recent-folder $folder'),
                borderRadius: BorderRadius.circular(look.radius),
                onTap: () => onChoose(folder),
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                  child: Row(children: [
                    Icon(selected ? Icons.folder : Icons.folder_outlined, size: 18, color: selected ? look.accent : look.muted),
                    const SizedBox(width: 10),
                    Flexible(child: Text(name, overflow: TextOverflow.ellipsis, maxLines: 1, style: const TextStyle(fontWeight: FontWeight.w600))),
                    const SizedBox(width: 10),
                    Expanded(child: Text(parent, overflow: TextOverflow.ellipsis, maxLines: 1, style: TextStyle(color: look.muted))),
                    if (selected) Icon(Icons.check, size: 16, color: look.accent),
                  ]),
                ),
              );
            }),
          const SizedBox(height: 6),
        ],
        Wrap(alignment: WrapAlignment.center, spacing: 8, children: [
          if (onBrowse != null)
            TextButton.icon(
              key: const Key('quick-choose-folder'),
              onPressed: onBrowse,
              icon: const Icon(Icons.folder_open_outlined, size: 18),
              label: const Text('Open a workspace'),
            ),
          if (onSources != null)
            TextButton.icon(
              key: const Key('quick-sources'),
              onPressed: onSources,
              icon: const Icon(Icons.tune, size: 18),
              label: const Text('Sources'),
            ),
        ]),
      ]),
    );
  }
}
