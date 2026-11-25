import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatSessionShare;
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show SizedBox;

import '../chat/reply_voice.dart' show followReplies;
import '../extension/tab_shares.dart' show sessionKeyOf;
import '../session_store.dart';
import '../tabs/window_tabs.dart';
import '../voice/reply_reader.dart';
import 'item_windows.dart' show WindowFrame;
import 'hub_tabs.dart';
import 'popout_conversation.dart';
import 'popout_window.dart';
import 'read_aloud.dart';
import 'shared_chat.dart';

export 'popout_window.dart';

/// A conversation's window that is another page, and what serves it over its link.
final class _Popped {
  _Popped(this.window, {required this.share, this.reading});
  final PopoutWindow window;

  /// The share that serves the window's link.
  final ChatSessionShare share;

  /// Answers the window's read-aloud over its link, when the app can read aloud.
  final ReadAloudBridge? reading;

  Future<void> end() async {
    reading?.close();
    await share.close();
    await window.close();
  }
}

/// A window of this engine and the conversations it holds as tabs. Each tab draws the store's own conversation through a
/// [LiveConversation] of its own, attached under the window's name.
final class _Group {
  _Group(this.window);

  final PopoutWindow window;
  final tabs = WindowTabs();
  final lives = <String, LiveConversation>{};
  final speakers = <String, SessionReplySpeaker>{};
}

/// A window the person can move a tab to.
typedef TabWindow = ({String id, String title, int tabs});

/// Where the conversations of a store are shown: as tabs of the main window, or of a window of their own. A window of this engine
/// draws the main window's conversations themselves: a conversation, its turn and what the agent waits on are one set of objects,
/// which what the person does in any window changes. A window that is another page is a view of one over a link. A session is a tab
/// in at most one window, and moving it closes it where it was; a window closed or a tab moved back leaves the conversation as it was.
class PopoutHub extends ChangeNotifier {
  PopoutHub(
    this.store,
    this.windows, {
    this.look = 'canoa',
    this.density = 'normal',
    this.shareInterval = const Duration(milliseconds: 80),
    this.reader,
    this.onVoiceMode,
    this.frame,
  }) {
    store.addListener(_storeChanged);
    mainTabs.addListener(notifyListeners);
    _syncTabs();
  }

  /// The key of the main window among the windows tabs are in.
  static const mainKey = 'main';

  /// Reads replies aloud for the windows that play no sound of their own; null where the app cannot.
  final ReplyReader? reader;

  /// Told when a window turns voice mode on or off, to keep the choice.
  final ValueChanged<bool>? onVoiceMode;

  /// Gives a window of this engine the app's theme and words; null where the window draws its own.
  final WindowFrame? frame;

  final SessionStore store;
  final PopoutWindows windows;

  /// The least time between two states sent to a window that is another page.
  final Duration shareInterval;

  /// The names of the app's look and density, for a window to draw with.
  String look, density;

  /// Whether a plain click on a conversation opens it in a window of its own (Settings, Appearance, Open in).
  bool openInNewWindows = false;

  /// The tabs of the main window. The tab shown is the store's selected session, which the rail changes too. A new session that has
  /// not started is no tab: the window shows its start screen until the first message, and the strip stays as it was.
  final mainTabs = WindowTabs();

  /// Shows the start screen of a new session: told when the main window is left with no tab to show.
  VoidCallback? onNewSession;

  final _popped = <String, _Popped>{};
  final _groups = <String, _Group>{};
  String? _lastMain;
  bool _syncing = false;

  /// Whether this platform can show a conversation in a window of its own.
  bool get available => windows.available;

  /// Whether a session moves between windows as a tab: the windows draw what this engine gives them.
  bool get movesTabs => windows.available && windows.drawsWidgets;

  /// Whether [session] is shown in a window of its own now.
  bool isPoppedOut(ChatSession session) => _popped.containsKey(session.id) || _groupOf(session.id) != null;

  /// The names the views of [session] are attached under: empty while its window has not attached yet, or it has none.
  List<String> viewsOf(ChatSession session) {
    final held = _popped[session.id];
    if (held != null) return held.share.views;
    final live = _groupOf(session.id)?.lives[session.id];
    return live != null && live.isOpen ? [live.clientId] : const [];
  }

  _Group? _groupOf(String id) => _groups.values.where((group) => group.tabs.contains(id)).firstOrNull;

  /// The window [session] is a tab of: [mainKey], a window's id, or null when it is in none.
  String? placeOf(ChatSession session) => _groupOf(session.id)?.window.id ?? (mainTabs.contains(session.id) ? mainKey : null);

  /// The tabs of the window [key], in order.
  List<ChatSession> sessionsIn(String key) {
    final ids = key == mainKey ? mainTabs.sessions : (_groups[key]?.tabs.sessions ?? const <String>[]);
    final here = {for (final session in store.sessions) session.id: session};
    return [for (final id in ids) ?here[id]];
  }

  /// The id of the tab the window [key] shows.
  String? currentIn(String key) {
    if (key != mainKey) return _groups[key]?.tabs.current;
    final chosen = store.selectedId;
    return chosen != null && mainTabs.contains(chosen) ? chosen : null;
  }

  /// Whether [session] is a tab where it is shown: one that has started, or is starting; not the start screen of a new session.
  bool _tabbable(ChatSession session) => !session.isDraft || store.isStarting(session);

  /// The tabs of a window of its own, to listen to; null for a window that is not one of this engine's.
  WindowTabs? tabsOf(String key) => key == mainKey ? mainTabs : _groups[key]?.tabs;

  /// The conversation a tab of the window [key] draws, and what reads it aloud.
  PopoutConversation? conversationIn(String key, String session) => _groups[key]?.lives[session];
  SessionReplySpeaker? speakerIn(String key, String session) => _groups[key]?.speakers[session];

  /// The windows of its own that hold tabs, other than [except]; each is titled by the tab it shows.
  List<TabWindow> otherWindows({String? except}) => [
    for (final MapEntry(:key, :value) in _groups.entries)
      if (key != except) (id: key, title: _titleOf(value), tabs: value.tabs.length),
  ];

  /// Whether the window [key] draws a strip: a window with several tabs does, and so does one with a window to move a tab to. The
  /// main window also draws it for a lone tab while the start screen of a new session is shown, to go back to it.
  bool showsStrip(String key) {
    final tabs = tabsOf(key);
    if (tabs == null || tabs.isEmpty) return false;
    if (key == mainKey && currentIn(key) == null) return true;
    return tabs.length > 1 || _groups.keys.any((other) => other != key);
  }

  /// Whether [session] can move to another window.
  bool canMove(ChatSession session) => movesTabs && !session.isDraft;

  String _titleOf(_Group group) {
    final shown = store.sessions.where((session) => session.id == group.tabs.current).firstOrNull;
    return shown?.title ?? group.window.title;
  }

  /// Opens [session] in a window of its own, or brings the one it is in forward. For a window of this engine that is a tab moved
  /// there; for a window that is another page, a view of the conversation.
  Future<void> popOut(ChatSession session) async {
    if (movesTabs) return moveToNewWindow(session);
    final held = _popped[session.id];
    if (held != null) return held.window.focus();
    if (!available || session.isDraft) throw StateError('This conversation cannot be shown in a window of its own yet');
    final window = await windows.open(title: session.title, look: look, density: density, session: sessionKeyOf(session));
    // Two requests for the same window can pass the check above; the second window goes.
    if (_popped.containsKey(session.id)) {
      unawaited(window.close());
      return;
    }
    final link = window.link;
    if (link == null) {
      unawaited(window.close());
      throw StateError('A window of this engine draws what it is given');
    }
    final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: shareInterval);
    final reading = reader == null ? null : ReadAloudLink(link);
    final bridge = reader == null || reading == null
        ? null
        : ReadAloudBridge(reader: reader!, link: reading, session: session.id, onVoiceMode: onVoiceMode ?? (_) {});
    final popped = _popped[session.id] = _Popped(window, share: share, reading: bridge);
    share.serve(reading ?? link);
    bridge?.start();
    // The window's own dock back button detaches its view, which leaves nothing to show there.
    unawaited(share.events.firstWhere((event) => !event.attached, orElse: () => throw StateError('closed')).then((_) => _end(session.id), onError: (_) {}));
    unawaited(window.closed.then((_) => _end(session.id, popped: popped)));
    notifyListeners();
  }

  /// Moves [session] into a window of its own, closing its tab where it was. A session alone in a window is only brought forward.
  Future<void> moveToNewWindow(ChatSession session) async {
    _requireMovable(session);
    final here = _groupOf(session.id);
    if (here != null && here.tabs.length == 1) return here.window.focus();
    _Group? group;
    final window = await windows.open(
      title: session.title,
      look: look,
      density: density,
      builder: (_) => group == null ? const SizedBox.shrink() : TabbedWindow(hub: this, windowId: group.window.id),
    );
    // Another request can have placed the session while the window opened.
    if (here == null && _groupOf(session.id) != null || !store.sessions.contains(session)) {
      unawaited(window.close());
      return;
    }
    final made = group = _Group(window);
    _groups[window.id] = made;
    made.tabs.addListener(notifyListeners);
    unawaited(window.closed.then((_) => _windowClosed(made)));
    _leave(session.id);
    _join(session, made);
  }

  /// Moves [session] into the window [windowId] as a tab shown there, closing its tab where it was.
  Future<void> moveToWindow(ChatSession session, String windowId) async {
    _requireMovable(session);
    final group = _groups[windowId];
    if (group == null) throw StateError('That window is gone');
    if (!group.tabs.contains(session.id)) {
      _leave(session.id);
      _join(session, group);
    }
    group.tabs.focus(session.id);
    await group.window.focus();
  }

  /// Moves [session] back to the main window, where it is the tab shown.
  Future<void> moveBack(ChatSession session) async {
    if (_groupOf(session.id) != null) _leave(session.id);
    mainTabs.open(session.id);
    _lastMain = session.id;
    store.select(session.id);
    notifyListeners();
  }

  /// Closes the tab [session] has in the window [key]; the conversation goes on. A window of its own closes with its last tab, and
  /// the main window then shows the start screen of a new session.
  void closeTab(String key, ChatSession session) {
    if (key == mainKey) {
      if (!mainTabs.contains(session.id)) return;
      _leave(session.id);
    } else if (_groups[key]?.tabs.contains(session.id) ?? false) {
      _leave(session.id);
    }
  }

  /// Shows [session]'s tab in the window [key]; in the main window that is selecting it, as the rail does.
  void selectTab(String key, ChatSession session) {
    if (key == mainKey) {
      if (!mainTabs.contains(session.id)) return;
      _lastMain = session.id;
      mainTabs.focus(session.id);
      store.select(session.id);
    } else {
      final group = _groups[key];
      if (group == null) return;
      group.tabs.focus(session.id);
      _retitle(group);
    }
  }

  /// Moves the tab at [from] to [to] (an index once it has left [from]) in the window [key].
  void reorder(String key, int from, int to) => tabsOf(key)?.reorder(from, to);

  void _requireMovable(ChatSession session) {
    if (!movesTabs || session.isDraft) throw StateError('This conversation cannot be shown in a window of its own yet');
  }

  /// Takes [session]'s tab out of the window it is in. The main window then shows its next tab; a window of its own with no tab
  /// left closes.
  void _leave(String id) {
    final group = _groupOf(id);
    if (group != null) {
      group.lives.remove(id)?.end();
      group.speakers.remove(id)?.dispose();
      group.tabs.remove(id);
      if (group.tabs.isEmpty) {
        _groups.remove(group.window.id);
        unawaited(group.window.close());
      } else {
        _retitle(group);
      }
    } else if (mainTabs.contains(id)) {
      final next = mainTabs.remove(id);
      if (store.selectedId == id) {
        if (next != null) {
          _lastMain = next;
          store.select(next);
        } else {
          _lastMain = null;
          onNewSession?.call();
        }
      }
    }
    notifyListeners();
  }

  void _join(ChatSession session, _Group group) {
    final speaker = reader == null ? null : SessionReplySpeaker(reader: reader!, session: session.id, onVoiceMode: onVoiceMode ?? (_) {});
    // The tab comes first: attaching the window to the conversation tells the store, which must find the session placed.
    group.tabs.open(session.id);
    final live = LiveConversation(store, session, clientId: group.window.id);
    group.lives[session.id] = live;
    if (speaker != null) group.speakers[session.id] = speaker;
    _retitle(group);
    // A window that detaches itself (its dock back button) is not showing the conversation any more: the tab goes home.
    unawaited(live.done.then((_) {
      if (identical(group.lives[session.id], live)) unawaited(moveBack(session));
    }));
    notifyListeners();
  }

  void _retitle(_Group group) {
    final title = _titleOf(group);
    if (title != group.window.title) group.window.setTitle(title);
  }

  /// The window was closed by the person or the system: its conversations go on as tabs of the main window.
  void _windowClosed(_Group group) {
    if (_groups[group.window.id] != group) return;
    _groups.remove(group.window.id);
    final shown = group.tabs.current;
    for (final id in group.tabs.sessions) {
      group.lives.remove(id)?.end();
      group.speakers.remove(id)?.dispose();
      mainTabs.open(id);
    }
    group.tabs.removeListener(notifyListeners);
    if (shown != null && store.sessions.any((session) => session.id == shown)) {
      _lastMain = shown;
      store.select(shown);
    }
    notifyListeners();
  }

  /// Closes [session]'s window; the conversation goes on where it is.
  Future<void> dockBack(ChatSession session) async {
    if (_groupOf(session.id) != null) return moveBack(session);
    final held = _popped[session.id];
    if (held == null) return;
    await _end(session.id);
  }

  Future<void> _end(String id, {_Popped? popped}) async {
    final held = _popped[id];
    if (held == null || (popped != null && !identical(held, popped))) return;
    _popped.remove(id);
    notifyListeners();
    await held.end();
  }

  /// Keeps the tabs in step with the store: the sessions that went are no tab, the one selected is shown, and a selection of a
  /// session that is in a window of its own brings that window forward instead of drawing it a second time.
  void _syncTabs() {
    if (_syncing) return;
    _syncing = true;
    try {
      final here = {for (final session in store.sessions) session.id: session};
      mainTabs.retain((id) => switch (here[id]) { final session? => _tabbable(session), null => false });
      for (final group in [..._groups.values]) {
        for (final id in [...group.tabs.sessions]) {
          if (!here.containsKey(id)) _leave(id);
        }
      }
      final chosen = store.selectedId;
      final selected = here[chosen];
      if (selected == null || !_tabbable(selected)) return;
      final group = _groupOf(chosen!);
      if (group == null) {
        mainTabs.open(chosen);
        _lastMain = chosen;
      } else if (mainTabs.isEmpty) {
        // The main window has no tab: it shows the start screen of a new session, and the window is brought forward.
        if (onNewSession case final show?) {
          group.tabs.focus(chosen);
          _retitle(group);
          unawaited(group.window.focus());
          scheduleMicrotask(show);
        }
      } else {
        group.tabs.focus(chosen);
        _retitle(group);
        unawaited(group.window.focus());
        final back = _lastMain != null && mainTabs.contains(_lastMain!) ? _lastMain! : mainTabs.sessions.first;
        scheduleMicrotask(() => store.select(back));
      }
    } finally {
      _syncing = false;
    }
  }

  void _storeChanged() {
    _syncTabs();
    // A conversation that was closed has nothing left to show.
    final here = {for (final session in store.sessions) session.id: session};
    for (final id in [..._popped.keys]) {
      final session = here[id];
      if (session == null) {
        unawaited(_end(id));
      } else if (_popped[id]!.window.title != session.title) {
        _popped[id]!.window.setTitle(session.title);
      }
    }
    for (final group in _groups.values) {
      _retitle(group);
    }
    // Voice mode turned on in a window reads that window's conversation as it grows, whichever one the main window shows.
    final pinned = reader?.pinned;
    if (reader != null && pinned != null && (_popped.containsKey(pinned) || _groupOf(pinned) != null)) {
      final session = here[pinned];
      if (session != null) followReplies(reader!, session.id, session.messages, working: session.status == SessionStatus.working);
    }
  }

  @override
  void dispose() {
    store.removeListener(_storeChanged);
    for (final id in [..._popped.keys]) {
      unawaited(_end(id));
    }
    for (final group in [..._groups.values]) {
      final lives = group.lives.values.toList();
      final speakers = group.speakers.values.toList();
      group.lives.clear();
      group.speakers.clear();
      for (final live in lives) {
        live.end();
      }
      for (final speaker in speakers) {
        speaker.dispose();
      }
      unawaited(group.window.close());
    }
    _groups.clear();
    mainTabs.dispose();
    super.dispose();
  }
}
