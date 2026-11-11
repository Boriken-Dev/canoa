import 'dart:async';
import 'dart:io';
import 'dart:math';

import '../permissions/permission_rules.dart' show LockableBackend;
import 'session_ledger.dart';

/// `<name>.json` (`sessions.json` for the session list) beside canoa.cfg at [configPath], in the app's config directory; in
/// memory when there is no such place.
LedgerBackend platformLedgerBackend({String? configPath, String name = 'sessions'}) =>
    configPath == null
        ? MemoryLedgerBackend()
        : FileLedgerBackend('${File(configPath).parent.path}${Platform.pathSeparator}$name.json');

/// The ledger in one file, replaced whole through a file beside it, named for this write alone, so that a write cut short leaves the
/// last one and two writers never share a half-written file. [exclusively] holds a lock file while a caller reads, changes and
/// replaces it, so another process cannot write between.
final class FileLedgerBackend implements LedgerBackend, LockableBackend {
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
    final next = File('$path.$pid.${Random.secure().nextInt(1 << 32).toRadixString(16)}.tmp');
    try {
      await next.writeAsString(document, flush: true);
      await next.rename(path);
    } on Object {
      if (await next.exists()) await next.delete();
      rethrow;
    }
  }

  /// Who holds each file in this process: a file lock does not keep out a second holder in the same process everywhere.
  static final Map<String, Future<void>> _tails = {};

  @override
  Future<T> exclusively<T>(Future<T> Function() body) async {
    final prior = _tails[path] ?? Future<void>.value();
    final done = Completer<void>();
    _tails[path] = done.future;
    await prior;
    try {
      return await _locked(body);
    } finally {
      done.complete();
      if (identical(_tails[path], done.future)) _tails.remove(path);
    }
  }

  Future<T> _locked<T>(Future<T> Function() body) async {
    final lock = File('$path.lock');
    await lock.parent.create(recursive: true);
    final handle = await lock.open(mode: FileMode.write);
    try {
      await handle.lock(FileLock.blockingExclusive);
      return await body();
    } finally {
      try {
        await handle.unlock();
      } on FileSystemException {
        // Closing the handle releases it.
      }
      await handle.close();
    }
  }
}
