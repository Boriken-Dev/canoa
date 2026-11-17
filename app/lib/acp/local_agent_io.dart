import 'dart:io';
import 'dart:typed_data' show BytesBuilder;

/// Whether this build runs on a computer with a file system of its own, which a native folder picker and an attached file's bytes
/// need. What a session's folder holds is the host's to say.
const bool canRunLocalAgents = true;

/// The bytes of the file at [path] on this computer: a file the person picked. With [maxBytes], no more than that many are read
/// from the opened file, however large it has become.
Future<List<int>> readLocalFile(String path, {int? maxBytes}) async {
  final file = File(path);
  if (maxBytes == null) return file.readAsBytes();
  final bytes = BytesBuilder(copy: false);
  await for (final chunk in file.openRead(0, maxBytes)) {
    bytes.add(chunk);
  }
  return bytes.takeBytes();
}

/// The size in bytes of the file at [path], or null for anything that is not a file that can be read.
Future<int?> localFileSize(String path) async {
  final file = File(path);
  return await file.exists() ? file.length() : null;
}

/// A file or folder inside a session's folder, as the host's `fs/resolve` answers.
typedef InsideFile = ({String path, bool folder, int size});
