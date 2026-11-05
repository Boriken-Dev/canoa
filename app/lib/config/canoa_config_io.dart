import 'dart:io';

String? desktopConfigPath() {
  final env = Platform.environment;
  final named = env['CANOA_CONFIG'];
  if (named != null && named.isNotEmpty) return named;
  final home = env['HOME'];
  if (Platform.isWindows) {
    final base = env['APPDATA'];
    return base == null ? null : '$base\\Canoa\\canoa.cfg';
  }
  if (Platform.isMacOS) {
    return home == null ? null :
        '$home/Library/Application Support/Canoa/canoa.cfg';
  }
  final base = env['XDG_CONFIG_HOME'] ?? (home == null ? null : '$home/.config');
  return base == null ? null : '$base/Canoa/canoa.cfg';
}
Future<String?> readDesktopConfig() async {
  final path = desktopConfigPath();
  if (path == null) return null;
  _adoptLegacyName(path);
  final file = File(path);
  return file.existsSync() ? file.readAsString() : null;
}
Future<void> writeDesktopConfig(String source) async {
  final path = desktopConfigPath();
  if (path == null) throw StateError('canoa.cfg location unavailable');
  _adoptLegacyName(path);
  final file = File(path);
  await file.parent.create(recursive: true);
  await file.writeAsString(source, flush: true);
}

// Only the default place is adopted: a file CANOA_CONFIG names is the person's own choice of name.
void _adoptLegacyName(String path) {
  final named = Platform.environment['CANOA_CONFIG'];
  if (named == null || named.isEmpty) adoptLegacyConfigName(path, log: stderr.writeln);
}

/// Gives a `Canoa.cfg` beside [path] the name [path], once, and says so through [log]. A file already at [path] is
/// never replaced, and where names ignore case the two are one file, so nothing is renamed. Returns whether it renamed.
bool adoptLegacyConfigName(String path, {required void Function(String line) log}) {
  if (Platform.isWindows) return false;
  // A link counts as an entry, so a dangling one is not written through.
  if (FileSystemEntity.typeSync(path, followLinks: false) != FileSystemEntityType.notFound) return false;
  final old = File('${File(path).parent.path}${Platform.pathSeparator}Canoa.cfg');
  if (FileSystemEntity.typeSync(old.path, followLinks: false) != FileSystemEntityType.file) return false;
  try {
    old.renameSync(path);
  } on FileSystemException catch (error) {
    log('Canoa.cfg was not renamed to canoa.cfg: ${error.message}');
    return false;
  }
  log('renamed ${old.path} to canoa.cfg');
  return true;
}