import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatSessionShare;
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show SizedBox, Widget;

import '../chat/reply_voice.dart' show followReplies;
import '../extension/tab_shares.dart' show sessionKeyOf;
import '../session_store.dart';
import '../voice/reply_reader.dart';
import 'item_windows.dart' show WindowFrame;
import 'popout_app.dart';
import 'popout_conversation.dart';
import 'popout_launch.dart';
import 'popout_window.dart';
import 'read_aloud.dart';
import 'shared_chat.dart';

export 'popout_window.dart';

/// A conversation's window and what serves it: a window that is another page is served over its link, and a window of this engine
/// draws the conversation itself.
final class _Popped {
  _Popped(this.window, {this.share, this.reading, this.live, this.speaker});
  final PopoutWindow window;

  /// The share that serves the window's link; null for a window of this engine.
  final ChatSessionShare? share;

  /// Answers the window's read-aloud over its link, when the app can read aloud.
  final ReadAloudBridge? reading;

  /// The conversation as a window of this engine holds it; null for a window that is another page.
  final LiveConversation? live;

  /// Reads aloud for a window of this engine.
  final SessionReplySpeaker? speaker;

  Future<void> end() async {
    live?.end();
    speaker?.dispose();
    reading?.close();
    await share?.close();
    await window.close();
  }
}

/// The conversations of a store that are open in a window of their own. A window of this engine draws the main window's
/// conversation itself: the conversation, its turn and what the agent waits on are one set of objects, which what the person does
/// in either window changes. A window that is another page is a view of it over a link. A window closed or docked back leaves the
/// conversation as it was.
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
  }

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

  final _popped = <String, _Popped>{};

  /// Whether this platform can show a conversation in a window of its own.
  bool get available => windows.available;

  /// Whether [session] is shown in a window of its own now.
  bool isPoppedOut(ChatSession session) => _popped.containsKey(session.id);

  /// The names the views of [session] are attached under: empty while its window has not attached yet, or it has none.
  List<String> viewsOf(ChatSession session) {
    final held = _popped[session.id];
    if (held == null) return const [];
    final live = held.live;
    return live != null ? [if (live.isOpen) live.clientId] : held.share?.views ?? const [];
  }

  /// Opens [session] in a window of its own, or brings its window forward when it has one.
  Future<void> popOut(ChatSession session) async {
    final held = _popped[session.id];
    if (held != null) return held.window.focus();
    if (!available || session.isDraft) throw StateError('This conversation cannot be shown in a window of its own yet');
    _Popped? popped;
    final window = await windows.open(
      title: session.title,
      look: look,
      density: density,
      session: sessionKeyOf(session),
      builder: (_) => popped == null ? const SizedBox.shrink() : _content(popped),
    );
    // Two requests for the same window can pass the check above; the second window goes.
    if (_popped.containsKey(session.id)) {
      unawaited(window.close());
      return;
    }
    final link = window.link;
    if (link == null) {
      final speaker = reader == null ? null : SessionReplySpeaker(reader: reader!, session: session.id, onVoiceMode: onVoiceMode ?? (_) {});
      popped = _popped[session.id] = _Popped(window, live: LiveConversation(store, session, clientId: window.id), speaker: speaker);
    } else {
      final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: shareInterval);
      final reading = reader == null ? null : ReadAloudLink(link);
      final bridge = reader == null || reading == null
          ? null
          : ReadAloudBridge(reader: reader!, link: reading, session: session.id, onVoiceMode: onVoiceMode ?? (_) {});
      popped = _popped[session.id] = _Popped(window, share: share, reading: bridge);
      share.serve(reading ?? link);
      bridge?.start();
      // The window's own dock back button detaches its view, which leaves nothing to show there.
      unawaited(share.events.firstWhere((event) => !event.attached, orElse: () => throw StateError('closed')).then((_) => _end(session.id), onError: (_) {}));
    }
    final made = popped;
    unawaited(window.closed.then((_) => _end(session.id, popped: made)));
    unawaited(made.live?.done.then((_) => _end(session.id, popped: made)));
    notifyListeners();
  }

  Widget _content(_Popped popped) => PopoutApp(
    launch: PopoutLaunch(link: popped.window.id, title: popped.window.title, look: look, density: density),
    conversation: popped.live!,
    speaker: popped.speaker,
    frame: frame,
    onClose: () => unawaited(popped.window.close()),
  );

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
    await held.end();
  }

  void _storeChanged() {
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
    // Voice mode turned on in a window reads that window's conversation as it grows, whichever one the main window shows.
    final pinned = reader?.pinned;
    if (reader != null && pinned != null && _popped.containsKey(pinned)) {
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
    super.dispose();
  }
}
