import 'dart:convert';
import 'dart:io';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:coqui_aos/io.dart' as aos_io;

import 'apply.dart';
import 'aos_support.dart';
import 'overlay_store_io.dart';
import 'overlays.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'project_trust.dart';

/// The profile home beside canoa.cfg at [configPath]: overlays in `overlays/`, trusted projects in `trust.json`.
ProfileHome? platformProfileHome({String? configPath}) =>
    configPath == null ? null : IoProfileHome(File(configPath).parent.path);

final class IoProfileHome implements ProfileHome, SessionFiles {
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

  /// In the system's temporary directory, which on Windows and macOS is the person's own and on Linux is made private.
  @override
  Future<String> write(Map<String, List<int>> files) async {
    final dir = await Directory.systemTemp.createTemp('canoa-session-');
    if (!Platform.isWindows) await Process.run('chmod', ['700', dir.path]);
    try {
      for (final MapEntry(:key, :value) in files.entries) {
        if (!pathAllowed(key, confined: true)) throw ArgumentError.value(key, 'files', 'not a relative path below');
        final file = File(join(dir.path, key));
        await file.parent.create(recursive: true);
        // Created exclusively, so nothing already there, a link included, is written through.
        await file.create(exclusive: true);
        final out = await file.open(mode: FileMode.writeOnly);
        try {
          await out.writeFrom(value);
        } finally {
          await out.close();
        }
      }
    } on Object {
      await dir.delete(recursive: true);
      rethrow;
    }
    return dir.path;
  }

  @override
  Future<void> remove(String directory) async {
    final dir = Directory(directory);
    if (await dir.exists()) await dir.delete(recursive: true);
  }
}
