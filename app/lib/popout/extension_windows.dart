import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

import 'popout_window.dart';

/// What the browser extension's worker does for windows: it opens one with `chrome.windows.create` at the extension's own page,
/// and tells when a window of the panel's has gone. A window gains nothing from being one: what it may ask of the worker is limited
/// there (bindings/browser/chromium/tab-sessions.js).
abstract interface class PopoutBridge {
  /// Who this page is to the worker; only the panel opens windows.
  String get client;

  /// Opens the window for [session], which attaches over the channel [link] names; returns its browser window id.
  Future<int> open({required String session, required String link, required String title, required String look, required String density});

  Future<void> focus(String session);
  Future<void> close(String session);

  /// The id of each of the browser's windows as it goes.
  Stream<int> get closed;
}

final class _ExtensionWindow implements PopoutWindow {
  _ExtensionWindow(this.link, this.bridge, this.session, this.windowId) {
    _listen = bridge.closed.where((id) => id == windowId).listen((_) => _gone());
  }

  @override
  final ChatViewLink link;
  final PopoutBridge bridge;
  final String session;
  final int windowId;
  final _closed = Completer<void>();
  late final StreamSubscription<int> _listen;

  void _gone() {
    unawaited(_listen.cancel());
    if (!_closed.isCompleted) _closed.complete();
  }

  @override
  Future<void> get closed => _closed.future;

  @override
  Future<void> focus() => bridge.focus(session);

  @override
  Future<void> close() async {
    if (!_closed.isCompleted) {
      try {
        await bridge.close(session);
      } on Object {
        // A window already gone has nothing to close.
      }
    }
    _gone();
    await link.close();
  }
}

/// Windows of the browser extension, each a second view of a session the side panel holds, over a channel only the two pages know.
final class ExtensionPopoutWindows implements PopoutWindows {
  ExtensionPopoutWindows(this.bridge, {required this.linkFor, required this.newLinkId});

  final PopoutBridge bridge;

  /// This end of the channel named by a link id.
  final ChatViewLink Function(String id) linkFor;
  final String Function() newLinkId;

  @override
  bool get available => bridge.client == 'panel';

  @override
  Future<PopoutWindow> open({required String title, required String look, required String density, String? session}) async {
    if (session == null) throw StateError('A window needs the session it shows');
    final id = newLinkId();
    final link = linkFor(id);
    try {
      final windowId = await bridge.open(session: session, link: id, title: title, look: look, density: density);
      return _ExtensionWindow(link, bridge, session, windowId);
    } on Object catch (error) {
      await link.close();
      throw StateError('The browser did not open the window: ${error is StateError ? error.message : error}');
    }
  }
}
