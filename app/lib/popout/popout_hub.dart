import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatSessionShare;
import 'package:flutter/foundation.dart';

import '../extension/tab_shares.dart' show sessionKeyOf;
import '../session_store.dart';
import 'popout_window.dart';
import 'shared_chat.dart';

export 'popout_window.dart';

final class _Popped {
  _Popped(this.share, this.window);
  final ChatSessionShare share;
  final PopoutWindow window;
}

/// The conversations of a store that are open in a window of their own, which is a view of the main window's conversation: the
/// conversation, its turn and what the agent waits on stay where they are. A window closed or docked back leaves the
/// conversation as it was.
class PopoutHub extends ChangeNotifier {
  PopoutHub(this.store, this.windows, {this.look = 'canoa', this.density = 'normal', this.shareInterval = const Duration(milliseconds: 80)}) {
    store.addListener(_storeChanged);
  }

  final SessionStore store;
  final PopoutWindows windows;

  /// The least time between two states sent to a window.
  final Duration shareInterval;

  /// The names of the app's look and density, for a window to draw with.
  String look, density;

  final _popped = <String, _Popped>{};

  /// Whether this platform can show a conversation in a window of its own.
  bool get available => windows.available;

  /// Whether [session] is shown in a window of its own now.
  bool isPoppedOut(ChatSession session) => _popped.containsKey(session.id);

  /// The names the views of [session] are attached under: empty while its window has not attached yet, or it has none.
  List<String> viewsOf(ChatSession session) => _popped[session.id]?.share.views ?? const [];

  /// Opens [session] in a window of its own, or brings its window forward when it has one.
  Future<void> popOut(ChatSession session) async {
    final held = _popped[session.id];
    if (held != null) return held.window.focus();
    if (!available || session.isDraft) throw StateError('This conversation cannot be shown in a window of its own yet');
    final window = await windows.open(title: session.title, look: look, density: density, session: sessionKeyOf(session));
    // Two requests for the same window can pass the check above; the second window goes.
    if (_popped.containsKey(session.id)) {
      unawaited(window.close());
      return;
    }
    final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: shareInterval);
    final popped = _popped[session.id] = _Popped(share, window);
    share.serve(window.link);
    // The window's own dock back button detaches its view, which leaves nothing to show there.
    unawaited(share.events.firstWhere((event) => !event.attached, orElse: () => throw StateError('closed')).then((_) => _end(session.id), onError: (_) {}));
    unawaited(window.closed.then((_) => _end(session.id, popped: popped)));
    notifyListeners();
  }

  /// Closes [session]'s window; the conversation goes on where it is.
  Future<void> dockBack(ChatSession session) async {
    final held = _popped[session.id];
    if (held == null) return;
    await _end(session.id);
  }

  Future<void> _end(String id, {_Popped? popped}) async {
    final held = _popped[id];
    if (held == null || (popped != null && !identical(held, popped))) return;
    _popped.remove(id);
    notifyListeners();
    await held.share.close();
    await held.window.close();
  }

  void _storeChanged() {
    // A conversation that was closed has nothing left to show.
    final here = {for (final session in store.sessions) session.id};
    for (final id in [..._popped.keys]) {
      if (!here.contains(id)) unawaited(_end(id));
    }
  }

  @override
  void dispose() {
    store.removeListener(_storeChanged);
    for (final id in [..._popped.keys]) {
      unawaited(_end(id));
    }
    super.dispose();
  }
}
