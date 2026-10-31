import 'dart:io';

String? desktopConfigPath() {
  final env = Platform.environment;
  final named = env['CANOA_CONFIG'];
  if (named != null && named.isNotEmpty) return named;
  final home = env['HOME'];
  if (Platform.isWindows) {
    final base = env['APPDATA'];
    return base == null ? null : '$base\\Canoa\\Canoa.cfg';
  }
  if (Platform.isMacOS) {
    return home == null ? null :
        '$home/Library/Application Support/Canoa/Canoa.cfg';
  }
  final base = env['XDG_CONFIG_HOME'] ?? (home == null ? null : '$home/.config');
  return base == null ? null : '$base/Canoa/Canoa.cfg';
}
Future<String?> readDesktopConfig() async {
  final path = desktopConfigPath();
  if (path == null) return null;
  final file = File(path);
  return file.existsSync() ? file.readAsString() : null;
}
Future<void> writeDesktopConfig(String source) async {
  final path = desktopConfigPath();
  if (path == null) throw StateError('Canoa.cfg location unavailable');
  final file = File(path);
  await file.parent.create(recursive: true);
  await file.writeAsString(source, flush: true);
}
