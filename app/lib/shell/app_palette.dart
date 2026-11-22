import 'dart:async';

import 'package:flutter/material.dart';

import '../acp/client.dart' show AcpSessionInfo;
import '../chat/agent_events.dart' show AgentCommandInfo;
import '../chat/context_pill.dart' show compactPrompt;
import '../chat/delegate_dialog.dart';
import '../chat/delegation_setup_dialog.dart';
import '../chat/inspector.dart' show changeTotals;
import '../chat/palette.dart';
import '../ledger/session_ledger.dart';
import '../popout/popout_hub.dart';
import '../rail/session_groups.dart';
import '../selectable.dart';
import '../session_store.dart';
import '../workspace/open_in.dart' show WorkspaceContext, workspaceOpener;
import '../workspace/open_in_ui.dart';

/// What the conversation shown offers the keyboard and the palette, set by it while it is shown.
final class ConversationHandle {
  Object? owner;
  VoidCallback? focusComposer, showChanges;
  ValueChanged<String>? insert;

  /// Ends a conversation as the app does, asking after its worktree; set by the app.
  Future<void> Function(ChatSession)? close;

  /// The sources a session can move to, by name and as shown, and the move itself; set by the app.
  Map<String, String> Function(ChatSession)? moveTargets;
  Future<void> Function(BuildContext, ChatSession)? move;

  /// The sessions the host brought in for a session's source, for Earlier conversations, whether it has any to list, and the
  /// showing of one; set by the app.
  bool Function(ChatSession)? canListImported;
  Future<List<AcpSessionInfo>> Function(ChatSession)? imported;
  Future<void> Function(ChatSession, AcpSessionInfo)? showImported;

  /// The conversations shown in a window of their own; set by the app.
  PopoutHub? popout;
}

List<PaletteItem> _closedLedgerItems(
  SessionStore store,
  SessionLedger? ledger,
  Future<void> Function(LedgerEntry)? onReopen,
) {
  if (ledger == null || onReopen == null) return const [];
  final open = {
    for (final session in store.sessions)
      if (session.peerId != null && session.remoteSessionId != null) (session.peerId!, session.remoteSessionId!),
  };
  return [
    for (final entry in ledger.entries)
      if (!open.contains((entry.peerId, entry.sessionId)))
        PaletteItem(
          entry.title ?? 'Earlier conversation',
          () => unawaited(onReopen(entry)),
          detail: '${entry.harness.isEmpty ? entry.entry : entry.harness} · Earlier conversation',
          icon: Icons.history,
        ),
  ];
}

/// Opens the command palette (or only open and earlier conversations when [sessionsOnly] is true).
Future<void> showAppPalette(
  BuildContext context, {
  required SessionStore store,
  required ValueNotifier<SessionGrouping> grouping,
  required ConversationHandle conversation,
  required VoidCallback onNewSession,
  required VoidCallback onSettings,
  required VoidCallback onStop,
  VoidCallback? onHosts,
  VoidCallback? onLoadout,
  VoidCallback? onLoadouts,
  Future<void> Function(LedgerEntry)? onReopen,
  Map<String, String> shortcutOverrides = const {},
  bool chatOnly = false,
  bool sessionsOnly = false,
}) {
  final session = store.selected;
  final earlier = _closedLedgerItems(store, store.ledger, onReopen);
  return showSelectableDialog<void>(
    context: context,
    builder: (_) => CommandPalette(
      items: sessionsOnly
          ? [
              for (final other in listOrder(store.listed, grouping.value))
                PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
              ...earlier,
            ]
          : [
              PaletteItem('New session', onNewSession, icon: Icons.add_comment_outlined),
              if (session != null && !session.isDraft && compactPrompt(session) != null)
                PaletteItem('Compact context', () => unawaited(store.sendText(compactPrompt(session)!)), icon: Icons.compress),
              if (!chatOnly && session != null && conversation.showChanges != null && (changeTotals(session) != null))
                PaletteItem('Show the changes', conversation.showChanges!, icon: Icons.difference_outlined),
              if (session?.status == SessionStatus.working) PaletteItem('Stop the turn', onStop, icon: Icons.stop_circle_outlined),
              if (session != null && store.canDelegate(session))
                PaletteItem(
                  'Delegate task',
                  () => unawaited(showDelegateDialog(context, store, session)),
                  detail: 'Hand a task to another agent, model or host',
                  icon: Icons.account_tree_outlined,
                ),
              if (session != null && store.canDelegate(session))
                PaletteItem(
                  'Delegation setup',
                  () => unawaited(openDelegationSetup(context, store, session)),
                  detail: 'Which agents this conversation may delegate to, and what it may attach',
                  icon: Icons.tune,
                ),
              if (!chatOnly && session != null)
                if (workspaceOpener.workspaceOf(place: session.place, cwd: session.cwd, endpoint: session.endpoint, peerId: session.peerId).target case final target?)
                  for (final action in workspaceOpener.actionsFor(WorkspaceContext.palette, target))
                    PaletteItem(
                      action.label,
                      () => unawaited(runWorkspaceAction(context, action, target)),
                      detail: target.path,
                      icon: action.icon,
                    ),
              if (chatOnly)
                PaletteItem('Open the full app', onSettings, detail: 'Workspaces, terminal, Store and Settings', icon: Icons.open_in_new)
              else ...[
                PaletteItem('Settings', onSettings, icon: Icons.settings_outlined),
                if (onLoadouts != null) PaletteItem('Loadouts', onLoadouts, icon: Icons.inventory_2_outlined),
                if (onLoadout != null) PaletteItem('Store', onLoadout, icon: Icons.storefront_outlined),
                if (onHosts != null) PaletteItem('Hosts', onHosts, detail: 'Local, saved connections and Hosting', icon: Icons.lan_outlined),
              ],
              PaletteItem('Keyboard shortcuts', () => unawaited(showShortcutHelp(context, overrides: shortcutOverrides)), icon: Icons.keyboard_outlined),
              for (final other in store.listed)
                if (other.id != store.selectedId)
                  PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
              ...earlier,
              for (final command in session?.commands ?? const <AgentCommandInfo>[])
                PaletteItem(
                  '/${command.name}',
                  () => conversation.insert?.call('/${command.name} '),
                  detail: command.description.isEmpty ? 'A command of the agent\'s' : command.description,
                  icon: Icons.terminal,
                ),
              PaletteItem(
                'Open in browser',
                () => unawaited(askToOpenInBrowser(context, workspaceOpener)),
                detail: 'A web address, in the system browser',
                icon: Icons.open_in_browser,
              ),
            ],
    ),
  );
}
