import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

// What a platform gives the pop-out, with nothing of Flutter in it.

/// A window another conversation is shown in, as the window that holds it sees it.
abstract interface class PopoutWindow {
  /// This end of the link to the window's view; its far end is in the window.
  ChatViewLink get link;

  /// Completes when the window is gone, however it went.
  Future<void> get closed;

  /// Brings the window to the front.
  Future<void> focus();

  /// Closes the window.
  Future<void> close();
}

/// Where a conversation can be shown in a window of its own: the platform's way to make one and to reach it.
abstract interface class PopoutWindows {
  /// Whether this platform can open one.
  bool get available;

  /// Opens a window that attaches to the conversation on the other end of the returned window's link, titled [title], drawn with
  /// [look] and [density] (the names of the app's own settings). [session] names the conversation for a platform that needs it (the
  /// browser extension's worker keeps the tab it works with by it).
  Future<PopoutWindow> open({required String title, required String look, required String density, String? session});
}

/// A platform with no way to open one.
final class NoPopoutWindows implements PopoutWindows {
  const NoPopoutWindows();

  @override
  bool get available => false;

  @override
  Future<PopoutWindow> open({required String title, required String look, required String density, String? session}) =>
      throw UnsupportedError('This platform has no windows of its own for a conversation');
}
