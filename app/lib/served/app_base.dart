import 'app_base_io.dart' if (dart.library.js_interop) 'app_base_web.dart' as platform;

/// Where the app's own files are: the page's `<base href>`, which is the site's root even when the page is a route under it (an
/// Office task pane frames the app at `/office/app/`). Beside it are `canoa.json` and the sign-in's pages. Elsewhere, the address
/// the app was started at.
Uri appBase() => platform.appBase();
