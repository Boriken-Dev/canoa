import 'dart:convert';
import 'dart:io';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:coqui_aos/io.dart' as aos_io;

import 'aos_support.dart';
import 'overlay_store_io.dart';
import 'overlays.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'project_trust.dart';

/// The profile home beside canoa.cfg at [configPath]: overlays in `overlays/`, trusted projects in `trust.json`.
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
  String get os => Platform.isAndroid ? 'linux' : Platform.operatingSystem;
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
  Future<ReadTree> readTree(String directory, {int maxFiles = 4096, int maxBytes = 64 * 1024 * 1024}) =>
      aos.readTree(const aos_io.IoFileSystem(), directory, treeSettings(maxFiles, maxBytes));

  @override
  Future<String?> readSecret(String path, {String? within}) async {
    try {
      // The path checked is the path read: links resolved once, then never followed again.
      final resolved = File(path).resolveSymbolicLinksSync();
      if (within != null) {
        final root = Directory(within).resolveSymbolicLinksSync();
        final prefix = root.endsWith(Platform.pathSeparator) ? root : '$root${Platform.pathSeparator}';
        final inside = Platform.isWindows || Platform.isMacOS
            ? resolved.toLowerCase().startsWith(prefix.toLowerCase())
            : resolved.startsWith(prefix);
        if (!inside) return null;
      }
      if (FileSystemEntity.typeSync(resolved, followLinks: false) != FileSystemEntityType.file) return null;
      final file = await File(resolved).open();
      try {
        final bytes = await file.read(maxSecretFile + 1);
        return bytes.length > maxSecretFile ? null : utf8.decode(bytes);
      } finally {
        await file.close();
      }
    } on Object {
      return null;
    }
  }
}
