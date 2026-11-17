import 'dart:io';

/// Whether this build runs on a computer with a file system of its own, which a native folder picker and an attached file's bytes
/// need. What a session's folder holds is the host's to say.
const bool canRunLocalAgents = true;

/// The bytes of the file at [path] on this computer: a file the person picked.
Future<List<int>> readLocalFile(String path) => File(path).readAsBytes();

/// The size in bytes of the file at [path], or null for anything that is not a file that can be read.
Future<int?> localFileSize(String path) async {
  final file = File(path);
  return await file.exists() ? file.length() : null;
}

/// A file or folder inside a session's folder, as the host's `fs/resolve` answers.
typedef InsideFile = ({String path, bool folder, int size});
