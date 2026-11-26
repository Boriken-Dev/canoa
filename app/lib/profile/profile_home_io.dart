import 'dart:convert';
import 'dart:io';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:coqui_aos/io.dart' as aos_io;

import 'aos_support.dart';
import 'overlays.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'project_trust.dart';

/// The profile home beside canoa.cfg at [configPath]: overlays in `overlays/`.
///
/// [overlays] is the host's manager of overlays; the git work is the host's, so none without a host. The record of trusted projects
/// is `[project_trust]` of canoa.cfg, which the host keeps: without a host nothing is trusted.
ProfileHome? platformProfileHome({String? configPath, OverlayManager overlays = const NoOverlays()}) =>
    configPath == null ? null : IoProfileHome(File(configPath).parent.path, overlays: overlays);

final class IoProfileHome implements ProfileHome {
  /// [trust] is the record of trusted projects; the default trusts nothing and cannot be written, since the host keeps it.
  IoProfileHome(this.configDir, {this.overlays = const NoOverlays(), ProjectTrust? trust})
      : trust = trust ?? _hostKeeps;

  static final ProjectTrust _hostKeeps = ProjectTrust(
    read: () async => null,
    write: (_) async => throw StateError('The host keeps the record of trusted projects'),
    caseInsensitive: Platform.isWindows || Platform.isMacOS,
  );

  @override
  final String configDir;
  @override
  String get os => Platform.isAndroid ? 'linux' : Platform.operatingSystem;
  @override
  final OverlayManager overlays;
  @override
  final ProjectTrust trust;

  @override
  String join(String dir, String relative) =>
      '$dir${Platform.pathSeparator}${relative.replaceAll('/', Platform.pathSeparator)}';

  @override
  Future<String?> canonicalDirectory(String directory) async {
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
