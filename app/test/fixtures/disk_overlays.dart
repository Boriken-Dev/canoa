// A stand-in for the overlays a host has installed beside canoa.cfg: the folder of each, and the record the host keeps of it. The
// host's own fetching, digesting and installing is tested in canoa-host; a test of what the app does with an installed overlay
// writes one here and reads it back as the app reads the host's.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart' show GitOverlay;

final class DiskOverlays extends NoOverlays {
  DiskOverlays(this.root);

  final String root;

  @override
  InstalledOverlay? installed(String name) {
    final record = File('$root/$name.json');
    if (!record.existsSync()) return null;
    return InstalledOverlay.fromJson(name, (jsonDecode(record.readAsStringSync()) as Map).cast<String, dynamic>());
  }

  @override
  String directoryOf(String name) => '$root/$name';

  /// Installs [files] as [name] from [source], as the host does: the folder written whole and a record naming the commit, the
  /// tree digest and each file.
  InstalledOverlay install(String name, GitOverlay source, Map<String, String> files) {
    final folder = Directory(directoryOf(name));
    if (folder.existsSync()) folder.deleteSync(recursive: true);
    for (final MapEntry(:key, :value) in files.entries) {
      File('${folder.path}/$key')
        ..parent.createSync(recursive: true)
        ..writeAsStringSync(value);
    }
    final tree = treeOfFiles({for (final MapEntry(:key, :value) in files.entries) key: utf8.encode(value)});
    final record = InstalledOverlay(
      name: name,
      source: source.spec,
      commit: 'c' * 40,
      digest: '${tree.digest.digest}',
      files: {for (final key in files.keys) key: 'b' * 40},
      skipped: const [],
      installed: DateTime.now(),
    );
    File('$root/$name.json')
      ..parent.createSync(recursive: true)
      ..writeAsStringSync(jsonEncode(record.toJson()));
    return record;
  }
}
