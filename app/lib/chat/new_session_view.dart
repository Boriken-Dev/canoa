import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/canoa_mark.dart';
import '../theme/look.dart';
import 'composer.dart';

/// A session before its first message: the mark, the folder it starts in, and the composer in the middle, whose row holds
/// the source, Local or Worktree, and the agent's choices. Nothing has started; sending starts it.
class NewSessionView extends StatelessWidget {
  const NewSessionView({super.key, required this.store, required this.session, required this.kit, required this.composer});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  /// The composer, built by the conversation that owns its text.
  final Widget composer;

  /// The menu value for the scratch folder; no folder's path holds a NUL.
  static const _scratchChoice = '\u0000scratch';

  Future<void> _open(BuildContext context) async {
    final folder = await kit.pickFolder?.call();
    if (folder != null) store.updateDraft(session, cwd: folder);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final theme = Theme.of(context);
    final cwd = session.cwd;
    final scratch = kit.scratchFolder?.call(session);
    final folders = {?cwd, ...kit.recentFolders}.toList();
    final notices = session.messages.where((message) => message.role == MessageRole.notice).toList();
    final folder = PopupMenuButton<String>(
      key: const Key('new-session-folder'),
      tooltip: 'The folder it works in',
      enabled: session.status == SessionStatus.draft,
      onSelected: (chosen) => chosen.isEmpty
          ? unawaited(_open(context))
          : chosen == _scratchChoice ? store.updateDraft(session, clearFolder: true) : store.updateDraft(session, cwd: chosen),
      itemBuilder: (context) => [
        if (scratch != null)
          CheckedPopupMenuItem(
            key: const Key('new-session-scratch'),
            value: _scratchChoice,
            checked: cwd == null,
            child: Text('Scratch  $scratch', overflow: TextOverflow.ellipsis),
          ),
        for (final each in folders)
          CheckedPopupMenuItem(value: each, checked: each == cwd, child: Text(each, overflow: TextOverflow.ellipsis)),
        if (folders.isNotEmpty) const PopupMenuDivider(),
        PopupMenuItem(
          key: const Key('new-session-browse'),
          value: '',
          enabled: kit.pickFolder != null,
          child: const Text('Open folder…'),
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
          Flexible(
            child: cwd == null && scratch != null
                ? Text.rich(
                    TextSpan(children: [
                      const TextSpan(text: 'Scratch', style: TextStyle(fontWeight: FontWeight.w600)),
                      TextSpan(text: '  $scratch', style: TextStyle(color: look.muted)),
                    ]),
                    key: const Key('new-session-scratch-path'),
                    overflow: TextOverflow.ellipsis,
                    maxLines: 1,
                  )
                : Text(cwd ?? 'Choose a folder', overflow: TextOverflow.ellipsis, maxLines: 1),
          ),
          Icon(Icons.expand_more, size: 18, color: look.muted),
        ]),
      ),
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
              folder,
              const SizedBox(height: 20),
              composer,
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
