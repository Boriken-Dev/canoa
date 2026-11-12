import 'package:flutter/material.dart';

import '../session_store.dart';
import 'tab_bridge.dart';
import 'tab_header.dart';
import 'tab_shares.dart';

/// What the composer shows and offers beyond its own controls, from the extension: the tabs a session shares, and a way to
/// share more. The composer calls these and knows nothing of tabs.
abstract interface class ComposerExtension {
  /// What goes above the message: banners and chips; null for none.
  Widget? header(BuildContext context, ChatSession session, TextEditingController controller);

  /// What `+` lists besides its own items.
  List<Widget> menuItems(BuildContext context, ChatSession session);
}

/// The extension's tabs, for a composer in the panel.
final class TabsComposerExtension implements ComposerExtension {
  TabsComposerExtension(this.tabs, {this.store}) {
    store?.addListener(_sessions);
  }

  /// This build's extension bridge, or null outside the extension.
  static TabsComposerExtension? platform(SessionStore store) {
    final bridge = platformTabBridge();
    return bridge == null ? null : TabsComposerExtension(TabShares(bridge), store: store);
  }

  final TabShares tabs;
  final SessionStore? store;

  void _sessions() => tabs.sessionsChanged({for (final session in store!.sessions) sessionKeyOf(session)});

  void dispose() {
    store?.removeListener(_sessions);
    tabs.dispose();
  }

  @override
  Widget? header(BuildContext context, ChatSession session, TextEditingController controller) =>
      canShareWith(session) ? TabHeader(tabs: tabs, session: session, controller: controller) : null;

  @override
  List<Widget> menuItems(BuildContext context, ChatSession session) {
    if (!canShareWith(session)) return const [];
    final key = sessionKeyOf(session);
    final messenger = ScaffoldMessenger.maybeOf(context);
    // The menu is built with the composer, which does not change when a tab does: this does.
    return [
      ListenableBuilder(
        listenable: tabs,
        builder: (context, _) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          for (final tab in tabs.tabs)
            MenuItemButton(
              key: Key('share-tab-${tab.id}'),
              leadingIcon: Icon(tabs.isShared(key, tab.id) ? Icons.check_circle : Icons.public, size: 18),
              onPressed: () => shareToggle(messenger, tabs, session, tab),
              child: ConstrainedBox(constraints: const BoxConstraints(maxWidth: 260), child: Text(tab.label, overflow: TextOverflow.ellipsis)),
            ),
          if (tabs.tabs.isNotEmpty) const Divider(height: 1),
        ]),
      ),
    ];
  }
}
