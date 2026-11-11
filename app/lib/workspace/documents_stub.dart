/// A page has no folders of its own, so it has no scratch folder either.
const bool hasWorkspaceFolders = false;

String? documentsFolder({Map<String, String>? environment, String? system, String? Function()? knownFolder}) => null;

String? homeFolder({Map<String, String>? environment, String? system}) => null;

String? localHostName() => null;

String get pathSeparator => '/';

/// Makes [path] and what is above it; true when it was not there.
Future<bool> makeFolder(String path) async => false;

/// Removes [path] when it is empty, and says nothing when it is not or cannot be.
Future<void> removeEmptyFolder(String path) async {}
