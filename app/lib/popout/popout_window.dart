import 'dart:async';
import 'dart:ui' show Size;

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/widgets.dart' show ChangeNotifier, Listenable, Widget, WidgetBuilder;

// What a platform gives the pop-out: windows to open, find and close. Only an implementation knows what a window is made of.

/// How big a window for a conversation opens.
const Size popoutSize = Size(560, 780);

/// What a window that is another page draws in place of a conversation: a file's viewer or a terminal, and what that page needs to
/// know to draw it. The page asks the page that opened it for the rest over its link.
final class PopoutItem {
  const PopoutItem(this.kind, [this.args = const {}]);

  /// The viewer of one file.
  static const file = 'file';

  /// The shells of a session.
  static const terminal = 'terminal';

  final String kind;
  final Map<String, String> args;
}

/// A window a conversation is shown in, as the window that holds it sees it.
abstract interface class PopoutWindow {
  /// Names this window among the ones its [PopoutWindows] holds.
  String get id;

  /// The title the window was given.
  String get title;

  /// The size of the window's drawable area now; [Size.zero] once it is gone.
  Size get size;

  /// This end of the link to the window's view, for a window that is another page or engine and only shows what it is sent; its
  /// far end is in the window. Null for a window of this engine, which draws the conversation itself.
  ChatViewLink? get link;

  /// Completes when the window is gone, however it went.
  Future<void> get closed;

  void setTitle(String title);

  void setSize(Size size);

  /// Brings the window to the front.
  Future<void> focus();

  /// Closes the window.
  Future<void> close();
}

/// Where a conversation can be shown in a window of its own: the platform's way to make one and to reach it. It tells its listeners
/// when a window comes or goes.
abstract interface class PopoutWindows implements Listenable {
  /// Whether this platform can open one.
  bool get available;

  /// Whether a window draws a widget this engine gives it, as a file or a terminal needs; false where a window is another page or
  /// engine that only shows a conversation it is linked to.
  bool get drawsWidgets;

  /// Whether a window that is another page can be given a [PopoutItem] to show, and so a file or a terminal: it draws it from what
  /// its link brings. A platform that [drawsWidgets] does not need it.
  bool get linksItems;

  /// The windows open now, oldest first.
  List<PopoutWindow> get windows;

  /// Opens a window titled [title], [size] big and drawn with [look] and [density] (the names of the app's own settings).
  ///
  /// A window of this engine draws [builder]'s widget, and its [PopoutWindow.link] is null. A window that is another page or engine
  /// cannot, and gets a link whose far end attaches to the conversation, which [session] names for a platform that needs it (the
  /// browser extension's worker keeps the tab it works with by it). A window that is another page given an [item] shows that instead
  /// of a conversation, over a link that serves it.
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
    PopoutItem? item,
  });

  /// [app] with the place this platform draws its windows of this engine; the app's root goes through it.
  Widget host(Widget app);
}

/// A [PopoutWindows] that keeps the list and tells when it changes, for an implementation to build on.
abstract class WindowList extends ChangeNotifier implements PopoutWindows {
  final _windows = <PopoutWindow>[];

  @override
  List<PopoutWindow> get windows => List.unmodifiable(_windows);

  @override
  bool get drawsWidgets => false;

  @override
  bool get linksItems => false;

  @override
  Widget host(Widget app) => app;

  /// Lists [window] until it is gone.
  T track<T extends PopoutWindow>(T window) {
    _windows.add(window);
    notifyListeners();
    unawaited(
      window.closed.then((_) {
        if (_windows.remove(window)) notifyListeners();
      }),
    );
    return window;
  }
}

/// A platform with no way to open one.
final class NoPopoutWindows extends WindowList {
  NoPopoutWindows();

  @override
  bool get available => false;

  @override
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
    PopoutItem? item,
  }) => throw UnsupportedError('This platform has no windows of its own for a conversation');
}
