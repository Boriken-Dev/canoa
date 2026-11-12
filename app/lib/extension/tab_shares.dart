import 'dart:async';

import 'package:flutter/foundation.dart';

import '../session_store.dart';
import 'tab_bridge.dart';

/// A share that ended without the person ending it: the tab went to another page or closed.
final class ShareDrop {
  const ShareDrop({required this.sessionKey, required this.tabTitle});

  final String sessionKey;
  final String tabTitle;
}

/// The key a session is known by in the worker: stable across reopening it from Earlier conversations.
String sessionKeyOf(ChatSession session) => session.workspaceId ?? session.id;

/// Whether the tabs of this browser can be shared with [session]'s agent: it has a paired agent's key to name.
bool canShareWith(ChatSession session) => session.peerId?.startsWith('sha256:') ?? false;

/// What the panel shows of the tabs a session is attached to and shares. The worker holds the truth and the grants; this reads it
/// and passes on the person's clicks. A share is to the agent's machine, so the words say so.
final class TabShares extends ChangeNotifier {
  TabShares(this.bridge) {
    _listen = bridge.changes.listen((_) => unawaited(refresh()));
    unawaited(refresh());
  }

  final TabBridge bridge;
  late final StreamSubscription<void> _listen;
  TabSnapshot _snapshot = const TabSnapshot();
  final _shared = <String, Map<int, String>>{};
  final _expected = <String, Set<int>>{};
  final _drops = <ShareDrop>[];

  /// The sessions this panel attached: the only ones it detaches when they close.
  final _mine = <String>{};
  var _disposed = false;

  List<OpenTab> get tabs => _snapshot.tabs;
  OpenTab? get frontTab => _snapshot.activeId == null ? null : _snapshot.tab(_snapshot.activeId!);

  TabBinding? bindingOf(String key) => _snapshot.bindings.where((binding) => binding.sessionKey == key).firstOrNull;

  bool isShared(String key, int tabId) => bindingOf(key)?.shared.contains(tabId) ?? false;

  /// The tabs [key] shares now.
  List<OpenTab> sharedTabs(String key) {
    final ids = bindingOf(key)?.shared ?? const <int>{};
    return [for (final tab in tabs) if (ids.contains(tab.id)) tab];
  }

  List<ShareDrop> dropsFor(String key) => [for (final drop in _drops) if (drop.sessionKey == key) drop];

  void dismiss(ShareDrop drop) {
    _drops.remove(drop);
    notifyListeners();
  }

  /// The tab [key] was attached to, when it is not the one in front: where its "Move here" comes from.
  OpenTab? startedElsewhere(String key) {
    final binding = bindingOf(key);
    final front = _snapshot.activeId;
    if (binding == null || front == null || binding.tabId == front) return null;
    return _snapshot.tab(binding.tabId);
  }

  Future<void> refresh() async {
    final TabSnapshot next;
    try {
      next = await bridge.snapshot();
    } on Object {
      return;
    }
    if (_disposed) return;
    for (final entry in _shared.entries.toList()) {
      final now = next.bindings.where((binding) => binding.sessionKey == entry.key).firstOrNull?.shared ?? const <int>{};
      final asked = _expected[entry.key] ?? <int>{};
      for (final gone in entry.value.entries.where((tab) => !now.contains(tab.key) && !asked.remove(tab.key))) {
        _drops.add(ShareDrop(sessionKey: entry.key, tabTitle: gone.value));
      }
    }
    _shared.clear();
    for (final binding in next.bindings) {
      _shared[binding.sessionKey] = {for (final id in binding.shared) id: next.tab(id)?.label ?? 'a tab'};
    }
    _snapshot = next;
    notifyListeners();
  }

  /// Attaches the session to the tab in front, the first time it is seen with none.
  Future<void> ensureAttached(String key) async {
    final front = _snapshot.activeId;
    if (bindingOf(key) != null || front == null) return;
    try {
      await bridge.attach(key, front);
    } on Object {
      return;
    }
    _mine.add(key);
    await refresh();
  }

  /// Sessions that have closed here end their shares: the worker would otherwise keep them until the tab changes.
  void sessionsChanged(Set<String> live) {
    for (final key in _mine.where((key) => !live.contains(key)).toList()) {
      _mine.remove(key);
      unawaited(bridge.detach(key).then((_) => refresh(), onError: (Object _) {}));
    }
  }

  /// Shares [tab] with [session]'s agent machine, or stops. Throws what the browser or the worker refused.
  Future<void> toggle(ChatSession session, OpenTab tab) async {
    final key = sessionKeyOf(session);
    if (isShared(key, tab.id)) {
      _expected.putIfAbsent(key, () => {}).add(tab.id);
      await bridge.unshare(key, tab.id);
    } else {
      if (!canShareWith(session)) throw StateError('This session has no paired agent to share with');
      if (bindingOf(key) == null) {
        await bridge.attach(key, tab.id);
        _mine.add(key);
      }
      _drops.removeWhere((drop) => drop.sessionKey == key && drop.tabTitle == tab.label);
      await bridge.share(key, tab.id, session.peerId!);
    }
    await refresh();
  }

  /// Takes the session to the tab in front: what it shared in the old tab stops, and it is attached here.
  Future<void> moveHere(ChatSession session) async {
    final key = sessionKeyOf(session);
    final front = _snapshot.activeId;
    if (front == null) return;
    for (final id in bindingOf(key)?.shared ?? const <int>{}) {
      _expected.putIfAbsent(key, () => {}).add(id);
      await bridge.unshare(key, id);
    }
    await bridge.attach(key, front);
    _mine.add(key);
    await refresh();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_listen.cancel());
    super.dispose();
  }
}
