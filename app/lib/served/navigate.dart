import 'navigate_io.dart' if (dart.library.js_interop) 'navigate_web.dart' as platform;

/// Sends this page to [address] (a full navigation, so the server answers it); nothing where the app is no web page. Where the page
/// is framed by a task pane, whose frame cannot show a sign-in page, the pane is told to show its own instead.
void navigateTo(Uri address) => platform.navigateTo(address);

/// Loads this page again, which starts the app over with what changed (a key paired, say); nothing where the app is no web page.
void reloadPage() => platform.reloadPage();
