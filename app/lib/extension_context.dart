import 'context_stub.dart'
    if (dart.library.js_interop) 'context_web.dart' as platform;

Future<String?> claimExtensionPageContext() => platform.claimExtensionPageContext();
