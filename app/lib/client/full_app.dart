// Opening the whole app from the chat-only one: a page of its own, which only a browser page can make.
import 'full_app_stub.dart' if (dart.library.js_interop) 'full_app_web.dart' as platform;

/// Opens [address] in a page of its own; false where this app cannot.
bool openPage(Uri address) => platform.openPage(address);
