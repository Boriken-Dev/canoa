/// A page has no file system of its own: it picks files in the browser and the host holds the folders.
const bool canRunLocalAgents = false;

Future<List<int>> readLocalFile(String path, {int? maxBytes}) => throw UnsupportedError('A page cannot read a file by its path');

Future<int?> localFileSize(String path) async => null;

/// A file or folder inside a session's folder, as the host's `fs/resolve` answers.
typedef InsideFile = ({String path, bool folder, int size});
