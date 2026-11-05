import 'dart:io';

import 'session_ledger.dart';

/// `sessions.json` beside canoa.cfg at [configPath], in the app's config directory; in memory when there is no such place.
LedgerBackend platformLedgerBackend({String? configPath}) =>
    configPath == null
        ? MemoryLedgerBackend()
        : FileLedgerBackend('${File(configPath).parent.path}${Platform.pathSeparator}sessions.json');

/// The ledger in one file, replaced whole through a file beside it so that a write cut short leaves the last one.
final class FileLedgerBackend implements LedgerBackend {
  FileLedgerBackend(this.path);
  final String path;

  @override
  String get location => path;

  @override
  Future<String?> read() async {
    final file = File(path);
    return await file.exists() ? file.readAsString() : null;
  }

  @override
  Future<void> write(String document) async {
    final file = File(path);
    await file.parent.create(recursive: true);
    final next = File('$path.new');
    await next.writeAsString(document, flush: true);
    await next.rename(path);
  }
}
