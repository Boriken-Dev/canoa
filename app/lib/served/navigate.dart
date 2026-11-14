import 'navigate_io.dart' if (dart.library.js_interop) 'navigate_web.dart' as platform;

/// Sends this page to [address] (a full navigation, so the server answers it); nothing where the app is no web page.
void navigateTo(Uri address) => platform.navigateTo(address);
