import 'site_documents_stub.dart' if (dart.library.js_interop) 'site_documents_web.dart' as platform;

/// The Documents folder of the agent's machine, as the person last gave it for the panel's site sessions; null when none.
/// Only this browser's own storage keeps it, and it is a path, nothing secret.
String? loadSiteDocuments() => platform.load();

void saveSiteDocuments(String? folder) => platform.save(folder);
