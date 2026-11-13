// What the extension's service worker says about tabs and the sessions attached to them, as the panel sees it. The worker owns
// the grants: this is the panel's view of them and the person's requests to change them (bindings/browser/chromium/tab-bindings.js).
import 'dart:convert';

import 'tab_bridge_stub.dart' if (dart.library.js_interop) 'tab_bridge_web.dart' as platform;

/// A web tab that is open now.
final class OpenTab {
  const OpenTab({required this.id, required this.title, required this.url, this.active = false});

  final int id;
  final String title;
  final String url;

  /// Whether it is the tab in front in the window the panel is in.
  final bool active;

  /// What to call it: its title, else its address.
  String get label => title.trim().isEmpty ? url : title.trim();
}

/// The client that holds a session's tab binding when nothing else does.
const panelClient = 'panel';

/// What the person calls a client: the side panel, or the window a session was popped out to.
String clientName(String client) => client == panelClient ? 'the side panel' : 'its own window';

/// A session attached to a tab, and the tabs it shares with its agent's machine.
final class TabBinding {
  const TabBinding({required this.sessionKey, required this.tabId, required this.origin, this.shared = const {}, this.owner = panelClient});

  final String sessionKey;
  final int tabId;
  final String origin;
  final Set<int> shared;

  /// The client that holds the binding: `panel`, or `window:<link>` for a popped-out window.
  final String owner;
}

/// What the worker holds, at one moment.
final class TabSnapshot {
  const TabSnapshot({this.tabs = const [], this.activeId, this.bindings = const []});

  final List<OpenTab> tabs;
  final int? activeId;
  final List<TabBinding> bindings;

  OpenTab? tab(int id) => tabs.where((tab) => tab.id == id).firstOrNull;

  /// Reads what the bridge's `snapshot` call answers.
  factory TabSnapshot.fromJson(String text) {
    final map = jsonDecode(text) as Map<String, Object?>;
    return TabSnapshot(
      tabs: [
        for (final tab in (map['tabs'] as List? ?? const []).cast<Map<String, Object?>>())
          OpenTab(id: tab['id'] as int, title: '${tab['title'] ?? ''}', url: '${tab['url'] ?? ''}', active: tab['active'] == true),
      ],
      activeId: map['active'] as int?,
      bindings: [
        for (final item in (map['sessions'] as List? ?? const []).cast<Map<String, Object?>>())
          TabBinding(
            sessionKey: item['sessionId'] as String,
            tabId: item['tabId'] as int,
            origin: '${item['origin'] ?? ''}',
            shared: {for (final id in (item['shared'] as List? ?? const [])) id as int},
            owner: item['owner'] is String ? item['owner'] as String : panelClient,
          ),
      ],
    );
  }
}

/// The calls the panel makes of the worker. A refusal is an exception whose message says why.
abstract interface class TabBridge {
  /// Who this view is to the worker: `panel`, or `window:<link>` in a popped-out window.
  String get client;

  Future<TabSnapshot> snapshot();
  Future<void> attach(String sessionKey, int tabId);
  Future<void> detach(String sessionKey);

  /// Gives this client the session's tab binding, on [tabId]: the session is not resumed and nothing is granted. What the old tab
  /// shared ends when the tab changes.
  Future<void> move(String sessionKey, int tabId);

  /// Shares the tab with the session's agent machine, as a read that lasts until it is [unshare]d, the tab goes or it changes page.
  /// Must run from the person's click: the browser asks for the site there.
  Future<void> share(String sessionKey, int tabId, String peerId);
  Future<void> unshare(String sessionKey, int tabId);

  /// A moment after anything the worker or the browser changes that this view shows.
  Stream<void> get changes;
}

/// The extension's own bridge, or null where this app is not running as the extension.
TabBridge? platformTabBridge() => platform.platformTabBridge();
