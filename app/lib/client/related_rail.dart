import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../brand.dart';
import '../ledger/ledger_list.dart';
import '../ledger/running_sessions.dart';
import '../ledger/session_ledger.dart';
import '../rail/session_groups.dart' show rowStateOf;
import '../session_store.dart';
import '../theme/canoa_mark.dart';
import '../theme/look.dart';
import 'related_sessions.dart';
import 'relation.dart';
import 'relation_source.dart';

/// The chat-only app's index: the sessions of what is in front of the client first, then the other open sessions and the earlier
/// ones, and a way to the whole app. In a drawer on a narrow client, which is what a side panel or a task pane is.
class RelatedRail extends StatelessWidget {
  const RelatedRail({
    super.key,
    required this.source,
    required this.store,
    required this.ledger,
    required this.onNew,
    required this.onNewSession,
    required this.onResume,
    this.onOpenFullApp,
    this.running,
    this.onAttach,
    this.startOptions,
  });

  final RelationSource source;
  final SessionStore store;
  final SessionLedger? ledger;

  /// Starts a session for what is in front.
  final Future<void> Function(Relation relation) onNew;

  /// Starts a session that relates to nothing, which is all there is to start while nothing is in front.
  final VoidCallback onNewSession;
  final Future<void> Function(LedgerEntry entry) onResume;

  /// Opens the whole app; null where the client cannot.
  final VoidCallback? onOpenFullApp;

  /// The sessions the host keeps running for several clients, which the list offers to join below the page's own; and what joins one.
  final RunningSessions? running;
  final Future<void> Function(RunningSession)? onAttach;

  /// What a session started for what is in front starts with, chosen beside its button.
  final Widget? startOptions;

  void _closeDrawer(BuildContext context) {
    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Material(
      color: context.look.rail,
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 6),
          child: Align(
            alignment: Alignment.centerLeft,
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              CanoaMark(color: look.accent),
              const SizedBox(width: 10),
              const Text(Brand.name, softWrap: false, style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700)),
            ]),
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: ListenableBuilder(
            listenable: Listenable.merge([source, store]),
            builder: (context, _) {
              final relation = source.current;
              final others = [for (final session in store.listed) if (relation == null || !relation.relates(session.relation)) session];
              return ListView(padding: const EdgeInsets.only(bottom: 8), children: [
                if (relation == null || ledger == null)
                  ListTile(
                    key: const Key('rail-new'),
                    dense: true,
                    leading: const Icon(Icons.add_comment_outlined, size: 20),
                    title: const Text('New session'),
                    onTap: () {
                      onNewSession();
                      _closeDrawer(context);
                    },
                  )
                else
                  RelatedSessions(
                    source: source,
                    ledger: ledger!,
                    store: store,
                    showOpen: true,
                    startOptions: startOptions,
                    onChosen: () => _closeDrawer(context),
                    onNew: (relation) async {
                      await onNew(relation);
                      if (context.mounted) _closeDrawer(context);
                    },
                    onResume: (entry) async {
                      await onResume(entry);
                      if (context.mounted) _closeDrawer(context);
                    },
                  ),
                if (others.isNotEmpty) ...[
                  Padding(
                    padding: const EdgeInsets.fromLTRB(16, 12, 12, 2),
                    child: Text(relation == null ? 'Conversations' : 'Other conversations', style: TextStyle(color: look.muted, fontSize: 12, fontWeight: FontWeight.w600)),
                  ),
                  for (final session in others)
                    ListTile(
                      key: ValueKey('rail-open ${session.id}'),
                      dense: true,
                      selected: session.id == store.selectedId,
                      title: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                      subtitle: Text([rowStateOf(session).word, session.agent].join(' · '), maxLines: 1, overflow: TextOverflow.ellipsis),
                      onTap: () {
                        store.select(session.id);
                        _closeDrawer(context);
                      },
                    ),
                ],
                if (ledger != null)
                  LedgerList(
                    ledger: ledger!,
                    store: store,
                    onReopen: onResume,
                    running: running,
                    onAttach: onAttach,
                    where: (entry) => relation == null || !relation.relates(entry.relation),
                  ),
              ]);
            },
          ),
        ),
        if (onOpenFullApp != null) ...[
          const Divider(height: 1),
          ListTile(
            key: const Key('open-full-app'),
            dense: true,
            leading: const Icon(Icons.open_in_new, size: 20),
            title: const Text('Open the full app'),
            subtitle: Text('Workspaces, terminal, Store and Settings', style: TextStyle(color: look.muted, fontSize: 11)),
            onTap: () {
              onOpenFullApp!();
              _closeDrawer(context);
            },
          ),
        ],
      ]),
    );
  }
}
