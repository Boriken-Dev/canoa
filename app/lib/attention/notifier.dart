import 'notifier_stub.dart' if (dart.library.io) 'notifier_io.dart' if (dart.library.js_interop) 'notifier_web.dart'
    as platform;

/// Shows a notice the host sent, outside the window. The host decides which events happen and who is told, and builds the words, so
/// a lock screen or a shared screen never shows a command, a path or anything else the agent sent; this only draws what it is given
/// and says whether the person is looking at the window.
abstract interface class Notifier {
  /// Prepares to show notifications; [onOpen] is given the id of the session whose notification the person chose.
  Future<void> initialize(void Function(String sessionId) onOpen);

  /// Whether notifications can be shown here now, the browser's permission included.
  Future<bool> get available;

  /// Asks the browser for leave to notify; true when it is given. A desktop needs none.
  Future<bool> requestPermission();

  /// Calls [onChange] with whether the person is looking at the app (its window shown and focused, or its page visible and
  /// focused) each time that changes, and once now.
  void watchAttention(void Function(bool attended) onChange);

  /// Shows one notification for the session [sessionId].
  Future<void> show({required String title, required String body, required String sessionId});
}

/// This platform's notifier: Windows and Linux desktops, or the browser's Notification API.
Notifier platformNotifier() => platform.createNotifier();
