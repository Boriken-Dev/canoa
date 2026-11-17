// The link between the chat-only app and the page that frames it: an Office task pane's page. The pane learns what is open in
// Outlook or the document and tells the app, which switches its related list without a reload.
import 'relation.dart';
import 'pane_link_stub.dart' if (dart.library.js_interop) 'pane_link_web.dart' as platform;

/// What the framing page and the app say to each other.
abstract interface class PaneLink {
  /// The link of this page, or null where none frames it (a page of its own, or an app that is no web page).
  static PaneLink? ofPage() => platform.paneLink();

  /// What the pane says is in front of the client: a relation, or null for nothing.
  Stream<Relation?> get relations;

  /// Says the app listens, so that the pane sends what is in front now. A change before this is not heard.
  void ready();

  /// Says the sign-in ended, so that the pane shows its own.
  void signIn();

  void dispose();
}
