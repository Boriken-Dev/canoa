import '../chat/agent_events.dart' show shortText;
import '../session_store.dart';
import 'notifier_stub.dart' if (dart.library.io) 'notifier_io.dart' if (dart.library.js_interop) 'notifier_web.dart'
    as platform;

/// Tells the person, outside the window, that a session needs them. What it shows names the session and the kind, never a
/// command, a path or anything else the agent sent, since a lock screen or a shared screen may show it.
abstract interface class Notifier {
  /// Prepares to show notifications; [onOpen] is given the local id of the session whose notification the person chose.
  Future<void> initialize(void Function(String sessionId) onOpen);

  /// Whether notifications can be shown here now, the browser's permission included.
  Future<bool> get available;

  /// Asks the browser for leave to notify; true when it is given. A desktop needs none.
  Future<bool> requestPermission();

  /// Whether the person is looking at the app: its window shown and focused, or its page visible and focused.
  Future<bool> attended();

  /// Shows one notification for the session [sessionId].
  Future<void> show({required String title, required String body, required String sessionId});
}

/// This platform's notifier: Windows and Linux desktops, or the browser's Notification API.
Notifier platformNotifier() => platform.createNotifier();

/// The words of a notification about [session] needing the person for [kind]: the session's name and its agent's, and what
/// kind of thing it wants, each one line long.
({String title, String body}) attentionText(ChatSession session, AttentionKind kind) {
  String named(String text, String otherwise) => shortText(text) ?? otherwise;
  final what = switch (kind) {
    AttentionKind.permission => 'asks leave to use a tool',
    AttentionKind.question => 'has a question for you',
    AttentionKind.finished => 'finished its turn',
  };
  return (title: named(session.title, 'A conversation'), body: '${named(session.agent, 'The agent')} $what');
}
