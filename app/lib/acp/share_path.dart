final RegExp _drive = RegExp(r'^[A-Za-z]:[\\/]');
final RegExp _verbatimShare = RegExp(r'^\\\\\?\\UNC\\[^\\/]+\\[^\\/]+', caseSensitive: false);
final RegExp _share = RegExp(r'^[\\/]{2}([^\\/]+)[\\/][^\\/]+');

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
