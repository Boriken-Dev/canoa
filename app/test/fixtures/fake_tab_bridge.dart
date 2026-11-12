import 'dart:async';

import 'package:canoa/extension/tab_bridge.dart';

/// The worker's side of the tab bridge, in memory, with its rules: a share needs a paired agent's key and a tab, ends when the tab
/// changes page or closes, and a detach ends a session's shares.
final class FakeTabBridge implements TabBridge {
  FakeTabBridge({List<OpenTab> tabs = const [], this.front});

  final tabs = <int, OpenTab>{};
  int? front;
  final bound = <String, int>{};
  final shares = <String, Set<int>>{};
  final calls = <String>[];
  final _changes = StreamController<void>.broadcast();

  /// Makes the tabs the person has open.
  void open(OpenTab tab) {
    tabs[tab.id] = tab;
    _changes.add(null);
  }

  /// The tab goes to another page: what it shared ends.
  void navigate(int tabId, String title) {
    final old = tabs[tabId]!;
    tabs[tabId] = OpenTab(id: tabId, title: title, url: old.url, active: old.active);
    for (final shared in shares.values) {
      shared.remove(tabId);
    }
    _changes.add(null);
  }

  void close(int tabId) {
    tabs.remove(tabId);
    for (final shared in shares.values) {
      shared.remove(tabId);
    }
    bound.removeWhere((_, tab) => tab == tabId);
    _changes.add(null);
  }

  void bring(int tabId) {
    front = tabId;
    _changes.add(null);
  }

  /// Bound already, as if from another window's panel.
  void bind(String key, int tabId) => bound[key] = tabId;

  /// What a click on a site the browser has not been given would answer.
  String? refuse;

  @override
  Future<TabSnapshot> snapshot() async => TabSnapshot(
        tabs: [for (final tab in tabs.values) OpenTab(id: tab.id, title: tab.title, url: tab.url, active: tab.id == front)],
        activeId: front,
        bindings: [
          for (final entry in bound.entries)
            TabBinding(sessionKey: entry.key, tabId: entry.value, origin: tabs[entry.value]?.url ?? '', shared: {...?shares[entry.key]}),
        ],
      );

  @override
  Future<void> attach(String sessionKey, int tabId) async {
    calls.add('attach $sessionKey $tabId');
    bound[sessionKey] = tabId;
  }

  @override
  Future<void> detach(String sessionKey) async {
    calls.add('detach $sessionKey');
    bound.remove(sessionKey);
    shares.remove(sessionKey);
  }

  @override
  Future<void> share(String sessionKey, int tabId, String peerId) async {
    calls.add('share $sessionKey $tabId $peerId');
    if (refuse != null) throw StateError(refuse!);
    if (!peerId.startsWith('sha256:')) throw StateError('Invalid browser permission');
    if (!tabs.containsKey(tabId)) throw StateError('That tab cannot be shared');
    shares.putIfAbsent(sessionKey, () => {}).add(tabId);
  }

  @override
  Future<void> unshare(String sessionKey, int tabId) async {
    calls.add('unshare $sessionKey $tabId');
    shares[sessionKey]?.remove(tabId);
  }

  @override
  Stream<void> get changes => _changes.stream;
}
