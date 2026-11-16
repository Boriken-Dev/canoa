import 'dart:io';

import 'contents_storage.dart';

/// The cache beside canoa.cfg at [configDir]: `cache/store/LABEL.json`; none without a directory.
ContentsStorage? platformContentsStorage(String? configDir) =>
    configDir == null ? null : FileContentsStorage('$configDir${Platform.pathSeparator}cache${Platform.pathSeparator}store');

final class FileContentsStorage implements ContentsStorage {
  FileContentsStorage(this.directory);

  final String directory;

  File _file(String label) => File('$directory${Platform.pathSeparator}${label.replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '_')}.json');

  @override
  Future<String?> read(String label) async {
    try {
      final file = _file(label);
      return await file.exists() ? await file.readAsString() : null;
    } on Object {
      return null;
    }
  }

  @override
  Future<void> write(String label, String json) async {
    final file = _file(label);
    await file.parent.create(recursive: true);
    final next = File('${file.path}.new');
    await next.writeAsString(json, flush: true);
    await next.rename(file.path);
  }
}
