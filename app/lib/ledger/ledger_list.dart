import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/look.dart';
import 'session_ledger.dart';

/// The ledger's sessions that are not open now, grouped by harness, with a filter by harness. Each opens again where its agent
/// offers that, says why where it does not, and can be removed from the list, which never deletes the agent's own session.
class LedgerList extends StatefulWidget {
  const LedgerList({super.key, required this.ledger, required this.store, this.onReopen});

  final SessionLedger ledger;
  final SessionStore store;

  /// Starts or reaches the session's agent again and opens the session; null where nothing can be.
  final Future<void> Function(LedgerEntry)? onReopen;

  @override
  State<LedgerList> createState() => _LedgerListState();
}

class _LedgerListState extends State<LedgerList> {
  /// The harness shown, or null for all.
  String? _harness;
  final Set<String> _opening = {};

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

  Future<void> _remove(LedgerEntry entry) async {
    final title = entry.title ?? 'This session';
    final confirmed = await showDialog<bool>(
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
    listenable: widget.ledger,
    builder: (context, _) {
      final look = context.look;
      final open = {
        for (final session in widget.store.sessions)
          if (session.peerId != null && session.remoteSessionId != null)
            LedgerEntry.keyOf(session.peerId!, session.remoteSessionId!),
      };
      final shown = [for (final entry in widget.ledger.entries) if (!open.contains(entry.key)) entry];
      final harnesses = {for (final entry in shown) entry.harness}.toList();
      final filter = harnesses.contains(_harness) ? _harness : null;
      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 16, 12, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Sessions by harness', style: Theme.of(context).textTheme.titleSmall),
            Text(
              'Removing one here never deletes the agent\'s own session.',
              style: TextStyle(color: look.muted, fontSize: 11),
            ),
            if (widget.ledger.problem != null)
              Text(widget.ledger.problem!, style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12)),
            if (shown.isEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(
                  'Sessions you open are kept here, by harness, to open again.',
                  style: TextStyle(color: look.muted, fontSize: 12),
                ),
              )
            else ...[
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
              for (final harness in filter == null ? harnesses : [filter]) ...[
                Padding(
                  key: ValueKey('harness $harness'),
                  padding: const EdgeInsets.only(top: 12, bottom: 2),
                  child: Text(harness, style: Theme.of(context).textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                ),
                for (final entry in shown)
                  if (entry.harness == harness) _tile(context, entry),
              ],
            ],
          ],
        ),
      );
    },
  );

  Widget _tile(BuildContext context, LedgerEntry entry) {
    final why = entry.whyNotReopened;
    final branch = entry.worktree == null ? null : 'branch ${entry.worktree!.branch}';
    final about = [entry.versionLabel, entry.place.label, branch, entry.cwd, _when(entry.lastUsed)].nonNulls.join(' · ');
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
