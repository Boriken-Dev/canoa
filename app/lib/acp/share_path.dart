final RegExp _drive = RegExp(r'^[A-Za-z]:[\\/]');
final RegExp _verbatimShare = RegExp(r'^\\\\\?\\UNC\\[^\\/]+\\[^\\/]+', caseSensitive: false);
final RegExp _verbatimUnc = RegExp(r'^\\\\\?\\UNC\\', caseSensitive: false);
final RegExp _verbatimDrive = RegExp(r'^\\\\\?\\(?=[A-Za-z]:)');
final RegExp _device = RegExp(r'^\\\\[?.]\\');
final RegExp _share = RegExp(r'^[\\/]{2}([^\\/]+)[\\/][^\\/]+');

/// [folder] as the client package takes a session's folder, which it checks is a root or a drive path: a folder on a share is
/// written with forward slashes (`//server/share/work`), and the host reads that back as `\\server\share\work`. A verbatim
/// form (`\\?\UNC\server\share`, `\\?\C:\`) is written plain first. Any other folder is as it is.
String folderForAgent(String folder) {
  if (_verbatimDrive.hasMatch(folder)) return folder.replaceFirst(_verbatimDrive, '');
  final plain = folder.replaceFirst(_verbatimUnc, r'\\');
  if (!plain.startsWith(r'\\') || _device.hasMatch(plain)) return plain;
  return plain.replaceAll('\\', '/');
}

/// Whether [folder] names a place on its own, whatever the current folder is: a root (`/home/ana`), a drive (`C:\work`) or a
/// folder on a Windows share (`\\server\share\work`). A device path (`\\.\pipe\x`), a server with no share and a path
/// that starts at the current drive's root (`\work`) are not.
bool isWholeFolderPath(String folder) {
  if (_drive.hasMatch(folder) || _verbatimShare.hasMatch(folder)) return true;
  if (folder.startsWith(RegExp(r'[\\/]{2}'))) {
    final server = _share.firstMatch(folder)?.group(1);
    return server != null && server != '.' && server != '?';
  }
  return folder.startsWith('/');
}
