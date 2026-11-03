import 'dart:convert';
import 'dart:io';

import 'overlay_store_io.dart';
import 'overlays.dart';
import 'profile_home.dart';
import 'project_trust.dart';

/// The profile home beside Canoa.cfg at [configPath]: overlays in `overlays/`, trusted projects in `trust.json`.
ProfileHome? platformProfileHome({String? configPath}) =>
    configPath == null ? null : IoProfileHome(File(configPath).parent.path);

final class IoProfileHome implements ProfileHome {
  IoProfileHome(this.configDir, {String git = 'git'})
      : overlays = OverlayStore('$configDir${Platform.pathSeparator}overlays', git: git) {
    final record = File('$configDir${Platform.pathSeparator}trust.json');
    trust = ProjectTrust(
      read: () async => record.existsSync() ? record.readAsString() : null,
      write: (text) async {
        await record.parent.create(recursive: true);
        final next = File('${record.path}.new');
        await next.writeAsString(text, flush: true);
        await next.rename(record.path);
      },
      caseInsensitive: Platform.isWindows || Platform.isMacOS,
    );
  }

  @override
  final String configDir;
  @override
  final OverlayManager overlays;
  @override
  late final ProjectTrust trust;

  @override
  String join(String dir, String relative) =>
      '$dir${Platform.pathSeparator}${relative.replaceAll('/', Platform.pathSeparator)}';

  @override
  String? canonicalDirectory(String directory) {
    try {
      final resolved = Directory(directory).resolveSymbolicLinksSync();
      return FileSystemEntity.isDirectorySync(resolved) ? resolved : null;
    } on FileSystemException {
      return null;
    }
  }

  @override
  Future<List<int>?> readBytes(String path) async {
    try {
      return await File(path).readAsBytes();
    } on FileSystemException {
      return null;
    }
  }

  @override
  Future<String?> readSecret(String path, {String? within}) async {
    try {
      final file = File(path);
      if (within != null) {
        final root = Directory(within).resolveSymbolicLinksSync();
        final resolved = file.resolveSymbolicLinksSync();
        final prefix = root.endsWith(Platform.pathSeparator) ? root : '$root${Platform.pathSeparator}';
        final inside = Platform.isWindows || Platform.isMacOS
            ? resolved.toLowerCase().startsWith(prefix.toLowerCase())
            : resolved.startsWith(prefix);
        if (!inside) return null;
      }
      return utf8.decode(await file.readAsBytes());
    } on Object {
      return null;
    }
  }
}
