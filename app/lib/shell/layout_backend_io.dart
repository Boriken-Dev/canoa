import 'dart:io';

import 'layout_state.dart';

/// `layout.json` beside canoa.cfg at [configPath]; in memory when there is no such place.
LayoutBackend platformLayoutBackend({String? configPath}) => configPath == null
    ? MemoryLayoutBackend()
    : FileLayoutBackend('${File(configPath).parent.path}${Platform.pathSeparator}layout.json');

/// One file, written whole through a file beside it.
final class FileLayoutBackend implements LayoutBackend {
  FileLayoutBackend(this.path);
  final String path;

  @override
  Future<String?> read() async {
    final file = File(path);
    return await file.exists() ? file.readAsString() : null;
  }

  @override
  Future<void> write(String json) async {
    final file = File(path);
    await file.parent.create(recursive: true);
    final next = File('$path.new');
    await next.writeAsString(json, flush: true);
    await next.rename(path);
  }
}
