import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../ledger/session_ledger.dart';
import '../rail/session_groups.dart' show rowStateOf;
import '../session_store.dart';
import 'relation.dart';
import 'relation_source.dart';

/// The sessions of [relation] that are open in [store], newest first.
List<ChatSession> openRelated(SessionStore store, Relation relation, {bool narrow = false}) =>
    [for (final session in store.listed) if (relation.relates(session.relation, narrow: narrow)) session];

/// What is in front of the client, a button to start a session for it and the sessions related to it: the ones open here and the
/// ledger's earlier ones to resume. Nothing related to anything else is read into it.
class RelatedSessions extends StatefulWidget {
  const RelatedSessions({
    super.key,
    required this.source,
    required this.ledger,
    required this.store,
    required this.onNew,
    required this.onResume,
    this.showOpen = false,
    this.onChosen,
    this.startOptions,
  });

  final RelationSource source;
  final SessionLedger ledger;
  final SessionStore store;

  /// Starts a session for [relation]. Called by the person's click, with nothing awaited before it.
  final Future<void> Function(Relation relation) onNew;
  final Future<void> Function(LedgerEntry entry) onResume;

  /// Whether the sessions already open are listed too, to choose between; the welcome page shows only the earlier ones.
  final bool showOpen;

  /// Told when the person chose an open session, so a drawer holding the list can close.
  final VoidCallback? onChosen;

  /// What the next new session starts with, chosen beside its button: the loadout and the profile.
  final Widget? startOptions;

  @override
  State<RelatedSessions> createState() => _RelatedSessionsState();
}

class _RelatedSessionsState extends State<RelatedSessions> {
  String? _failed;
  bool _busy = false;

  /// Whether the list is of the one place inside the thing in front (a page, not its whole origin), which the person chooses.
  bool _narrow = false;

  static String _when(DateTime at) {
    final local = at.toLocal();
    String two(int value) => value.toString().padLeft(2, '0');
    return '${local.year}-${two(local.month)}-${two(local.day)} ${two(local.hour)}:${two(local.minute)}';
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _failed = null;
    });
    try {
      await action();
    } on Object catch (error) {
      if (mounted) setState(() => _failed = '$error'.replaceFirst(RegExp(r'^(Bad state|Exception): '), ''));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([widget.source, widget.ledger, widget.store]),
      builder: (context, _) {
        final relation = widget.source.current;
        if (relation == null) return const SizedBox.shrink();
        final look = context.chatLook;
        final theme = Theme.of(context);
        final narrow = _narrow && relation.scope != null;
        final open = {
          for (final session in widget.store.sessions)
            if (session.peerId != null && session.remoteSessionId != null)
              LedgerEntry.keyOf(session.peerId!, session.remoteSessionId!),
        };
        final earlier = [for (final entry in widget.ledger.forRelation(relation, narrow: narrow)) if (!open.contains(entry.key)) entry];
        final here = widget.showOpen ? openRelated(widget.store, relation, narrow: narrow) : const <ChatSession>[];
        return Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(children: [
                Icon(relation.kind == Relation.originKind ? Icons.public : Icons.link, size: 18, color: look.muted),
                const SizedBox(width: 8),
                Expanded(child: Text(relation.display, key: const Key('related-name'), style: theme.textTheme.titleSmall, overflow: TextOverflow.ellipsis)),
                FilledButton.tonal(
                  key: const Key('related-new'),
                  onPressed: _busy ? null : () => unawaited(_run(() => widget.onNew(relation))),
                  child: const Text('New session'),
                ),
              ]),
              ?widget.startOptions,
              if (relation.label != null)
                Padding(
                  padding: const EdgeInsets.only(top: 2, left: 26),
                  child: Text(relation.label!, key: const Key('related-label'), style: TextStyle(color: look.muted, fontSize: 12), maxLines: 1, overflow: TextOverflow.ellipsis),
                ),
              if (relation.scope != null)
                Align(
                  alignment: Alignment.centerLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: FilterChip(
                      key: const Key('related-narrow'),
                      label: const Text('This page only'),
                      selected: _narrow,
                      visualDensity: VisualDensity.compact,
                      onSelected: (value) => setState(() => _narrow = value),
                    ),
                  ),
                ),
              if (_failed != null)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Text(_failed!, key: const Key('related-failed'), style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
                ),
              if (here.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text('Open for ${relation.display}', style: theme.textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                for (final session in here)
                  ListTile(
                    key: ValueKey('related-open ${session.id}'),
                    dense: true,
                    contentPadding: EdgeInsets.zero,
                    selected: session.id == widget.store.selectedId,
                    title: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                    subtitle: Text([rowStateOf(session).word, session.agent].join(' · '), maxLines: 1, overflow: TextOverflow.ellipsis),
                    onTap: () {
                      widget.store.select(session.id);
                      widget.onChosen?.call();
                    },
                  ),
              ],
              if (earlier.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text('Earlier on ${relation.display}', style: theme.textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                for (final entry in earlier)
                  ListTile(
                    key: ValueKey('related-session ${entry.key}'),
                    dense: true,
                    contentPadding: EdgeInsets.zero,
                    title: Text(entry.title ?? 'Untitled session', maxLines: 1, overflow: TextOverflow.ellipsis),
                    subtitle: Text([_when(entry.lastUsed), entry.cwd].nonNulls.join(' · '), maxLines: 2, overflow: TextOverflow.ellipsis),
                    trailing: TextButton(
                      key: ValueKey('related-resume ${entry.key}'),
                      onPressed: _busy ? null : () => unawaited(_run(() => widget.onResume(entry))),
                      child: const Text('Resume'),
                    ),
                  ),
              ],
            ],
          ),
        );
      },
    );
  }
}
