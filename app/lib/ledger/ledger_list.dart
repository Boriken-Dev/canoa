import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import 'running_sessions.dart';
import 'session_ledger.dart';
import '../selectable.dart';

/// The ledger's sessions that are not open now, listed under Earlier conversations, with a filter by agent. Each opens again where its agent
/// offers that, says why where it does not, and can be removed from the list, which never deletes the agent's own session.
class LedgerList extends StatefulWidget {
  const LedgerList({super.key, required this.ledger, required this.store, this.onReopen, this.running, this.onAttach, this.where});

  final SessionLedger ledger;
  final SessionStore store;

  /// Starts or reaches the session's agent again and opens the session; null where nothing can be.
  final Future<void> Function(LedgerEntry)? onReopen;

  /// The sessions the host keeps running for several clients, listed above the earlier ones; null where the host lists none.
  final RunningSessions? running;

  /// Takes part in a running session: connects to its entry on the host and attaches to it, replayed what was said.
  final Future<void> Function(RunningSession)? onAttach;

  /// Which entries are listed; all of them when null.
  final bool Function(LedgerEntry)? where;

  @override
  State<LedgerList> createState() => _LedgerListState();
}

class _LedgerListState extends State<LedgerList> {
  /// The harness shown, or null for all.
  String? _harness;
  final Set<String> _opening = {};
  final Set<String> _attaching = {};

  @override
  void initState() {
    super.initState();
    unawaited(widget.running?.refresh());
  }

  static String _when(DateTime at) {
    final local = at.toLocal();
    String two(int value) => value.toString().padLeft(2, '0');
    return '${local.year}-${two(local.month)}-${two(local.day)} ${two(local.hour)}:${two(local.minute)}';
  }

  Future<void> _reopen(LedgerEntry entry) async {
    final messenger = ScaffoldMessenger.of(context);
    setState(() => _opening.add(entry.key));
    try {
      await widget.onReopen!(entry);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That session could not be opened: $error')));
    } finally {
      if (mounted) setState(() => _opening.remove(entry.key));
    }
  }

  Future<void> _attach(RunningSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final key = _runningKey(session);
    setState(() => _attaching.add(key));
    try {
      await widget.onAttach!(session);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That session could not be joined: $error')));
    } finally {
      if (mounted) setState(() => _attaching.remove(key));
    }
  }

  static String _runningKey(RunningSession session) => '${session.entry}\n${session.sessionId}';

  /// How long ago [at] was, as a person says it.
  static String _ago(DateTime at) {
    final since = DateTime.now().toUtc().difference(at.toUtc());
    if (since.inSeconds < 60) return 'just now';
    if (since.inMinutes < 60) return '${since.inMinutes} min ago';
    if (since.inHours < 48) return '${since.inHours} h ago';
    return '${since.inDays} days ago';
  }

  Future<void> _remove(LedgerEntry entry) async {
    final title = entry.title ?? 'This session';
    final confirmed = await showSelectableDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Remove from Canoa\'s list?'),
        content: Text(
          '"$title" leaves this list only. This does not delete the session itself: ${entry.harness} still has it.'
          '${entry.messages.isEmpty ? '' : ' What Canoa kept of what was said goes with it.'}',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
        ],
      ),
    );
    if (confirmed ?? false) widget.ledger.remove(entry);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.ledger, ?widget.running]),
    builder: (context, _) {
      final look = context.chatLook;
      final open = {
        for (final session in widget.store.sessions)
          if (session.peerId != null && session.remoteSessionId != null)
            LedgerEntry.keyOf(session.peerId!, session.remoteSessionId!),
      };
      // What is attached to now is a conversation of the list, not a running one to join.
      final joined = {
        for (final session in widget.store.sessions)
          if (session.remoteSessionId != null && session.status != SessionStatus.disconnected)
            session.remoteSessionId!,
      };
      final running = [
        for (final session in widget.running?.sessions ?? const <RunningSession>[])
          if (!joined.contains(session.sessionId)) session,
      ];
      final isRunning = {for (final session in running) '${session.entry}\n${session.sessionId}'};
      final shown = [
        for (final entry in widget.ledger.entries)
          if (!open.contains(entry.key) && !isRunning.contains('${entry.entry}\n${entry.sessionId}') && (widget.where?.call(entry) ?? true)) entry,
      ];
      final harnesses = {for (final entry in shown) entry.harness}.toList();
      final filter = harnesses.contains(_harness) ? _harness : null;
      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 16, 12, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Earlier conversations', style: Theme.of(context).textTheme.titleSmall),
            Text(
              'Removing one here never deletes the agent\'s own session.',
              style: TextStyle(color: look.muted, fontSize: 11),
            ),
            if (widget.ledger.problem != null)
              Text(widget.ledger.problem!, style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12)),
            if (running.isNotEmpty) ...[
              Padding(
                key: const ValueKey('running sessions'),
                padding: const EdgeInsets.only(top: 12, bottom: 2),
                child: Text('Running now', style: Theme.of(context).textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
              ),
              Text(
                'The host keeps these going: join one from any device.',
                style: TextStyle(color: look.muted, fontSize: 11),
              ),
              for (final session in running) _runningTile(context, session),
            ],
            if (shown.isEmpty && running.isEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(
                  'Sessions you open are kept here, to open again.',
                  style: TextStyle(color: look.muted, fontSize: 12),
                ),
              )
            else if (shown.isNotEmpty) ...[
              const SizedBox(height: 8),
              Wrap(spacing: 6, runSpacing: 4, children: [
                ChoiceChip(
                  label: const Text('All'),
                  selected: filter == null,
                  onSelected: (_) => setState(() => _harness = null),
                ),
                for (final harness in harnesses)
                  ChoiceChip(
                    label: Text(harness),
                    selected: filter == harness,
                    onSelected: (_) => setState(() => _harness = harness),
                  ),
              ]),
              // Chats have no workspace of the person's, so they are listed apart and show no folder.
              if (shown.any((entry) => entry.kind == SessionKind.chat && (filter == null || entry.harness == filter))) ...[
                Padding(
                  key: const ValueKey('earlier chats'),
                  padding: const EdgeInsets.only(top: 12, bottom: 2),
                  child: Text('Chats', style: Theme.of(context).textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                ),
                for (final entry in shown)
                  if (entry.kind == SessionKind.chat && (filter == null || entry.harness == filter)) _tile(context, entry),
              ],
              for (final harness in [for (final each in filter == null ? harnesses : [filter]) if (shown.any((entry) => entry.harness == each && entry.kind == SessionKind.workspace)) each]) ...[
                Padding(
                  key: ValueKey('harness $harness'),
                  padding: const EdgeInsets.only(top: 12, bottom: 2),
                  child: Text(harness, style: Theme.of(context).textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                ),
                for (final entry in shown)
                  if (entry.harness == harness && entry.kind == SessionKind.workspace) _tile(context, entry),
              ],
            ],
          ],
        ),
      );
    },
  );

  Widget _runningTile(BuildContext context, RunningSession session) {
    final kept = widget.ledger.entries
        .where((entry) => entry.sessionId == session.sessionId && entry.entry == session.entry)
        .firstOrNull;
    final others = session.clients.length;
    final about = [
      session.entry,
      session.cwd,
      others == 0 ? 'nobody attached' : '$others attached',
      if (session.turnRunning) 'working',
      if (session.permissionsWaiting > 0) 'waiting for an answer',
      if (session.lastActivity != null) _ago(session.lastActivity!),
    ].nonNulls.join(' · ');
    final key = _runningKey(session);
    return ListTile(
      key: ValueKey('running $key'),
      dense: true,
      contentPadding: const EdgeInsets.only(left: 4),
      title: Text(kept?.title ?? session.title ?? 'Shared session', maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(about, maxLines: 3),
      trailing: widget.onAttach == null
          ? null
          : IconButton(
              tooltip: 'Join',
              icon: const Icon(Icons.login, size: 20),
              onPressed: _attaching.contains(key) ? null : () => unawaited(_attach(session)),
            ),
    );
  }

  Widget _tile(BuildContext context, LedgerEntry entry) {
    final why = entry.whyNotReopened;
    final branch = entry.worktree == null ? null : 'branch ${entry.worktree!.branch}';
    final chat = entry.kind == SessionKind.chat;
    final about = [
      if (chat) entry.harness,
      entry.versionLabel,
      entry.place.label,
      branch,
      if (!chat) entry.cwd,
      _when(entry.lastUsed),
    ].nonNulls.join(' · ');
    return ListTile(
      key: ValueKey('session ${entry.key}'),
      dense: true,
      contentPadding: const EdgeInsets.only(left: 4),
      title: Text(entry.title ?? 'Untitled session', maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(why == null ? about : '$about\n$why', maxLines: 5),
      isThreeLine: why != null,
      trailing: Row(mainAxisSize: MainAxisSize.min, children: [
        if (entry.canReopen && widget.onReopen != null)
          IconButton(
            tooltip: 'Reopen',
            icon: const Icon(Icons.replay, size: 20),
            onPressed: _opening.contains(entry.key) ? null : () => unawaited(_reopen(entry)),
          )
        else if (entry.messages.isNotEmpty)
          IconButton(
            tooltip: 'Show what was kept',
            icon: const Icon(Icons.notes, size: 20),
            onPressed: () => widget.store.showKept(entry),
          ),
        IconButton(
          tooltip: 'Remove from this list',
          icon: const Icon(Icons.delete_outline, size: 20),
          onPressed: () => unawaited(_remove(entry)),
        ),
      ]),
    );
  }
}
