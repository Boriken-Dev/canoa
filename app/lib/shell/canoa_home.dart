import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart' show KeyUpEvent;

import '../chat/composer.dart' show ComposerKit;
import '../l10n/app_text.dart';
import '../chat/conversation_view.dart';
import '../client/related_rail.dart';
import '../client/relation.dart';
import '../client/relation_source.dart';
import '../extension/local_host.dart';
import '../extension/local_host_banner.dart';
import '../ledger/running_sessions.dart';
import '../ledger/session_ledger.dart';
import '../rail/session_groups.dart';
import '../rail/session_rail.dart';
import '../selectable.dart';
import '../session_store.dart';
import '../theme/look.dart';
import '../usage/spend_model.dart' show SpendRow;
import 'app_palette.dart';
import 'app_shortcuts.dart';
import 'hosts/hosts_controller.dart';
import 'layout_state.dart';
import 'sections.dart';
import 'shell.dart';
import 'surface.dart' show WorkSurfaces;

/// The top-level app shell hosting Conversations and the Settings, Store, Loadouts, Usage, Secrets, Terminal and Hosts sections.
class CanoaHome extends StatelessWidget {
  const CanoaHome({
    super.key,
    required this.store,
    required this.onNewSession,
    required this.onSettings,
    this.onHosts,
    this.onLoadout,
    this.onLoadouts,
    this.account,
    this.onSignOut,
    this.onReopen,
    this.running,
    this.onAttach,
    required this.conversation,
    this.kit = const ComposerKit(),
    this.renderers = chat.ChatRenderers.none,
    required this.welcome,
    this.noAgent = false,
    this.localHost,
    required this.grouping,
    required this.layout,
    required this.surfaces,
    required this.settings,
    required this.loadout,
    required this.loadouts,
    required this.hostsIndex,
    required this.hostsPage,
    required this.storeIndex,
    required this.loadoutsIndex,
    this.usageIndex,
    this.usagePage,
    this.secretsIndex,
    this.secretsPage,
    this.sessionSpend,
    required this.terminalIndex,
    required this.terminalMain,
    this.chatOnly = false,
    this.relations,
    required this.onNewRelated,
    required this.onResumeRelated,
    this.onOpenFullApp,
    this.startOptions,
    this.shortcutOverrides = const {},
  });

  final LocalHostBridge? localHost;
  final ValueNotifier<SessionGrouping> grouping;
  final ShellLayout layout;
  final WorkSurfaces surfaces;
  final Widget settings, loadout, loadouts;
  final Widget hostsIndex, hostsPage, storeIndex, loadoutsIndex;
  final Widget? usageIndex, usagePage;
  final Widget? secretsIndex, secretsPage;
  final Widget terminalIndex, terminalMain;
  final bool chatOnly;
  final RelationSource? relations;
  final Future<void> Function(Relation relation) onNewRelated;
  final Future<void> Function(LedgerEntry entry) onResumeRelated;
  final VoidCallback? onOpenFullApp;
  final Widget? startOptions;
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;
  final SessionStore store;
  final Widget welcome;
  final bool noAgent;
  final VoidCallback onNewSession;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;
  final VoidCallback onSettings;
  final VoidCallback? onHosts;
  final VoidCallback? onLoadout, onLoadouts;
  final String? account;
  final Future<void> Function()? onSignOut;
  final Future<void> Function(LedgerEntry)? onReopen;
  final RunningSessions? running;
  final Future<void> Function(RunningSession)? onAttach;
  final ConversationHandle conversation;
  final Map<String, String> shortcutOverrides;

  Widget _rail(BuildContext context) => SessionRail(
    store: store,
    onNewSession: onNewSession,
    onReopen: onReopen,
    running: running,
    onAttach: onAttach,
    grouping: grouping,
    initialScroll: layout.of(ShellLayout.conversations).scroll,
    onScroll: (offset, {required end}) {
      layout.setScroll(ShellLayout.conversations, offset);
      if (end) unawaited(layout.flush());
    },
    onSearch: () => unawaited(_palette(context, sessionsOnly: true)),
    onClose: conversation.close ?? store.close,
  );

  Future<void> _palette(BuildContext context, {bool sessionsOnly = false}) => showAppPalette(
    context,
    store: store,
    grouping: grouping,
    conversation: conversation,
    onNewSession: onNewSession,
    onSettings: onSettings,
    onStop: () => stopActiveSession(store),
    onHosts: onHosts,
    onLoadout: onLoadout,
    onLoadouts: onLoadouts,
    onReopen: onReopen,
    shortcutOverrides: shortcutOverrides,
    chatOnly: chatOnly,
    sessionsOnly: sessionsOnly,
  );

  @override
  Widget build(BuildContext context) {
    return CallbackShortcuts(
      bindings: buildAppShortcuts(
        context,
        store: store,
        grouping: grouping,
        layout: layout,
        surfaces: chatOnly ? WorkSurfaces(const []) : surfaces,
        conversation: conversation,
        kit: kit,
        onNewSession: onNewSession,
        onSettings: onSettings,
        onPalette: () => _palette(context),
        overrides: shortcutOverrides,
      ),
      child: Focus(
        autofocus: true,
        // Push to talk ends when any key of the shortcut comes up.
        onKeyEvent: (_, event) {
          if (event is KeyUpEvent && kit.dictation?.holding == true) unawaited(kit.dictation!.release());
          return KeyEventResult.ignored;
        },
        child: _layout(context),
      ),
    );
  }

  Widget _layout(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final narrow = AppShell.isNarrow(constraints.maxWidth) || Uri.base.queryParameters['view'] == 'panel';
        return Selectable(
          child: AppShell(
            layout: layout,
            narrow: narrow,
            account: account,
            onSignOut: onSignOut,
            indexWidth: context.look.railWidth,
            banner: localHost == null ? null : LocalHostBanner(bridge: localHost!),
            selector: !chatOnly,
            sections: [
              AppSection(
                id: ShellLayout.conversations,
                ownsLayoutControls: true,
                label: context.t('nav.conversations'),
                symbol: 'conversations',
                index: chatOnly ? _relatedRail() : _rail(context),
                main: ConversationView(
                  key: ValueKey(store.selectedId ?? 'empty'),
                  store: store,
                  narrow: narrow,
                  handle: conversation,
                  layout: layout,
                  surfaces: surfaces,
                  kit: kit,
                  renderers: renderers,
                  welcome: welcome,
                  noAgent: noAgent,
                  sessionSpend: sessionSpend,
                  chatOnly: chatOnly,
                  onOpenFullApp: onOpenFullApp,
                ),
              ),
              if (!chatOnly) ...[
                AppSection(id: HostsController.section, label: context.t('nav.hosts'), symbol: 'hosts', index: hostsIndex, main: hostsPage),
                AppSection(id: 'terminal', label: context.t('nav.terminal'), symbol: 'terminal', index: terminalIndex, main: terminalMain),
                if (usagePage != null) AppSection(id: 'usage', label: context.t('nav.usage'), symbol: 'usage', index: usageIndex, main: usagePage!),
                if (secretsPage != null) AppSection(id: 'secrets', label: context.t('nav.secrets'), symbol: 'secrets', index: secretsIndex, main: secretsPage!),
                AppSection(id: 'loadouts', label: context.t('nav.loadouts'), symbol: 'loadouts', index: loadoutsIndex, main: loadouts),
                AppSection(id: 'store', label: context.t('nav.store'), symbol: 'store', index: storeIndex, main: loadout, ownsLayoutControls: true),
                AppSection(id: 'settings', label: context.t('nav.settings'), tooltip: context.t('nav.settings_tooltip'), symbol: 'settings', main: settings, atFoot: true),
              ],
            ],
          ),
        );
      },
    );
  }

  Widget _relatedRail() => RelatedRail(
    source: relations!,
    store: store,
    ledger: store.ledger,
    onNew: onNewRelated,
    onNewSession: onNewSession,
    onResume: onResumeRelated,
    onOpenFullApp: onOpenFullApp,
    running: running,
    onAttach: onAttach,
    startOptions: startOptions,
  );
}
