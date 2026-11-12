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

/// What a failed rename says that waiting can cure: another process has the target open (Windows: access denied, a sharing or a
/// lock violation; elsewhere: permission denied or busy).
bool _transient(FileSystemException error) => const {5, 13, 16, 32, 33}.contains(error.osError?.errorCode);

/// The ledger in one file, replaced whole through a file beside it, named for this write alone, so that a write cut short leaves the
/// last one and two writers never share a half-written file. A rename another process's open file refuses is tried again for up to
/// [retryFor]. [exclusively] holds a lock file while a caller reads, changes and replaces it, so another process cannot write
/// between; it gives up after its timeout.
final class FileLedgerBackend implements LedgerBackend, LockableBackend {
  FileLedgerBackend(this.path, {this.retryFor = const Duration(seconds: 2), this.renameFile});
  final String path;
  final Duration retryFor;

  /// Replaces [to] with [from]; the real rename unless a test gives another.
  final Future<void> Function(File from, String to)? renameFile;

  /// A temp file left by a writer that was killed is taken away by the next write once it is this old.
  static const staleAfter = Duration(minutes: 1);

  @override
  String get location => path;

  @override
  Future<String?> read() async {
    final file = File(path);
    return await file.exists() ? file.readAsString() : null;
  }

  Future<void> _removeStale(File file) async {
    final prefix = '${file.uri.pathSegments.last}.';
    try {
      await for (final entity in file.parent.list()) {
        final name = entity.uri.pathSegments.where((part) => part.isNotEmpty).last;
        if (!name.startsWith(prefix) || !name.endsWith('.tmp') || entity is! File) continue;
        if (DateTime.now().difference(await entity.lastModified()) > staleAfter) await entity.delete();
      }
    } on FileSystemException {
      // Another writer got there first, or the folder is not listable: nothing here depends on it.
    }
  }

  @override
  Future<void> write(String document) async {
    final file = File(path);
    await file.parent.create(recursive: true);
    await _removeStale(file);
    final next = File('$path.$pid.${Random.secure().nextInt(1 << 32).toRadixString(16)}.tmp');
    try {
      await next.writeAsString(document, flush: true);
      final until = DateTime.now().add(retryFor);
      var wait = 10;
      while (true) {
        try {
          await (renameFile?.call(next, path) ?? next.rename(path));
          break;
        } on FileSystemException catch (error) {
          if (!_transient(error) || DateTime.now().isAfter(until)) rethrow;
          await Future<void>.delayed(Duration(milliseconds: wait));
          wait = min(wait * 2, 200);
        }
      }
    } on Object {
      try {
        if (await next.exists()) await next.delete();
      } on FileSystemException {
        // It is taken away by a later write.
      }
      rethrow;
    }
  }

  /// Who holds each file in this process: a file lock does not keep out a second holder in the same process everywhere.
  static final Map<String, Future<void>> _tails = {};

  @override
  Future<T> exclusively<T>(Future<T> Function() body, {Duration timeout = const Duration(seconds: 2)}) async {
    final deadline = DateTime.now().add(timeout);
    final prior = _tails[path] ?? Future<void>.value();
    final done = Completer<void>();
    final mine = done.future;
    _tails[path] = mine;
    var acquired = false;
    try {
      try {
        await prior.timeout(timeout);
      } on TimeoutException {
        throw TimeoutException('another writer of $path did not finish in ${timeout.inMilliseconds} ms', timeout);
      }
      acquired = true;
      return await _locked(body, deadline, timeout);
    } finally {
      // Whoever comes next waits for the holder before us too, so giving up never lets two in at once.
      if (acquired) {
        done.complete();
      } else {
        unawaited(prior.whenComplete(done.complete));
      }
      if (identical(_tails[path], mine) && acquired) _tails.remove(path);
    }
  }

  Future<T> _locked<T>(Future<T> Function() body, DateTime deadline, Duration timeout) async {
    final lock = File('$path.lock');
    await lock.parent.create(recursive: true);
    final handle = await lock.open(mode: FileMode.write);
    var held = false;
    try {
      while (!held) {
        try {
          await handle.lock(FileLock.exclusive);
          held = true;
        } on FileSystemException {
          if (DateTime.now().isAfter(deadline)) {
            throw TimeoutException('another process held $path.lock for ${timeout.inMilliseconds} ms', timeout);
          }
          await Future<void>.delayed(const Duration(milliseconds: 20));
        }
      }
      return await body();
    } finally {
      if (held) {
        try {
          await handle.unlock();
        } on FileSystemException {
          // Closing the handle releases it.
        }
      }
      await handle.close();
    }
  }
}
