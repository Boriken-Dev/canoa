import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../profile/aos_support.dart' show holdsPassword;
import 'contents_reader.dart';
import 'contents_storage.dart';
import 'overlay_contents.dart';
import 'store.dart';

/// What reading one entry ahead gave: what it provides, or why it could not be read, and what the reading was of.
final class EntryContents {
  const EntryContents({required this.key, required this.readAt, this.contents, this.problem});

  /// What the reading was of: the entry's pin, else the commit of its catalog. A reading is current only for the same key.
  final String key;
  final DateTime readAt;
  final OverlayContents? contents;
  final String? problem;

  Map<String, Object?> toJson() => {
    'key': key,
    'read_at': readAt.millisecondsSinceEpoch,
    'contents': ?contents?.toJson(),
    'problem': ?problem,
  };

  static EntryContents? fromJson(Object? json) {
    if (json is! Map<Object?, Object?> || json['key'] is! String || json['read_at'] is! int) return null;
    return EntryContents(
      key: json['key']! as String,
      readAt: DateTime.fromMillisecondsSinceEpoch(json['read_at']! as int),
      contents: json['contents'] is Map<Object?, Object?> ? OverlayContents.fromJson(json['contents']! as Map<Object?, Object?>) : null,
      problem: json['problem'] is String ? json['problem']! as String : null,
    );
  }
}

/// The id [ContentsStore] keeps an entry under.
String contentsId(String label, String kind, String name) => '$label/$kind:$name';

/// What the entries the person looks at provide, read when they open one so that its page shows what it holds. A reading is kept under
/// the cache directory by catalog, and is current while the entry's pin (or, when it has none, its catalog's commit) is the one it
/// was made for: a catalog at the commit it was read at has the same entries, so nothing is read again. A catalog with no commit, or
/// a `ttl` of zero, reads an entry with no pin again after `ttl`.
///
/// The host reads: it fetches an entry's files as an install's review does, into a place nothing is installed from, and runs
/// nothing. This store asks for the entries that are not current in batches and keeps the answers. One entry that cannot be read
/// is noted against it and the rest go on.
class ContentsStore extends ChangeNotifier {
  ContentsStore({required this.reader, this.storage, DateTime Function()? now}) : _now = now ?? DateTime.now;

  final ContentsReader reader;
  final ContentsStorage? storage;
  final DateTime Function() _now;

  final _entries = <String, EntryContents>{};
  final _reading = <String>{};
  final _loaded = <String>{};
  var _disposed = false;

  /// How many entries have been asked of the host to be read, since this store was made.
  int fetches = 0;

  /// How many entries one request to the host holds.
  static const batchSize = 8;

  /// What was read of the entry [id], or null when nothing has been.
  EntryContents? of(String id) => _entries[id];

  /// Whether the entry [id] is being read now.
  bool isReading(String id) => _reading.contains(id);

  bool get busy => _reading.isNotEmpty || _running > 0;
  int _running = 0;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads the entries of [catalogs] whose last reading is not current, [concurrency] requests at a time: only those named in
  /// [only] (ids from [contentsId]) when it is given, else every one. [force] reads them again. An entry with no pin is current
  /// while its catalog is at the commit it was read at; a catalog with no commit has [ttl] for it, and zero reads again every time.
  Future<void> readAhead(
    List<LoadedCatalog> catalogs, {
    required Duration ttl,
    bool force = false,
    int concurrency = 3,
    Set<String>? only,
  }) async {
    _running++;
    try {
      final work = <(String label, String id, String key, ContentsRequest request)>[];
      for (final loaded in catalogs) {
        final catalog = loaded.catalog;
        if (catalog == null) continue;
        await _load(loaded.label);
        final fallback = loaded.commit ?? '';
        void want(String kind, String name, String? pin, ContentsRequest Function(String id) request) {
          final id = contentsId(loaded.label, kind, name);
          if (only != null && !only.contains(id)) return;
          final key = pin ?? fallback;
          final held = _entries[id];
          final current = held != null &&
              held.key == key &&
              (pin != null || ttl > Duration.zero && (fallback.isNotEmpty || _now().difference(held.readAt) < ttl));
          if (!force && current) return;
          try {
            work.add((loaded.label, id, key, request(id)));
          } on StoreRefusal catch (error) {
            _entries[id] = EntryContents(key: key, readAt: _now(), problem: error.message);
          }
        }

        for (final entry in catalog.entries.values) {
          want('overlay', entry.name, entry.pin, (id) {
            final source = installSource(entry);
            return ContentsRequest(id: id, kind: 'overlay', name: installName(entry), repo: source.repo, ref: source.reference, path: source.path);
          });
        }
        for (final MapEntry(key: name, value: entry) in catalog.skills.entries) {
          want('skill', name, entry.pin, (id) {
            final source = entry.source;
            if (source.kind != 'git') throw const StoreRefusal('Canoa fetches skills from git only, and this source is not git');
            if (holdsPassword(source.location)) throw const StoreRefusal('A skill source holds a password, which none may');
            return ContentsRequest(id: id, kind: 'skill', name: name, repo: source.location, ref: source.reference, path: source.path);
          });
        }
      }
      final batches = [for (var at = 0; at < work.length; at += batchSize) work.sublist(at, (at + batchSize).clamp(0, work.length))];
      var next = 0;
      final touched = <String>{};
      Future<void> worker() async {
        while (next < batches.length) {
          final batch = batches[next++];
          touched.addAll(batch.map((item) => item.$1));
          await _readBatch(batch);
        }
      }

      await Future.wait([for (var i = 0; i < concurrency.clamp(1, 16); i++) worker()]);
      for (final label in touched) {
        await _save(label);
      }
    } finally {
      _running--;
      notifyListeners();
    }
  }

  Future<void> _readBatch(List<(String label, String id, String key, ContentsRequest request)> batch) async {
    for (final item in batch) {
      _reading.add(item.$2);
    }
    fetches += batch.length;
    notifyListeners();
    Map<String, ContentsAnswer> answers;
    String? failure;
    try {
      answers = await reader.read([for (final item in batch) item.$4]);
    } on Object catch (error) {
      answers = const {};
      failure = '$error';
    }
    for (final (_, id, key, _) in batch) {
      final answer = answers[id];
      _entries[id] = EntryContents(
        key: key,
        readAt: _now(),
        contents: answer?.contents,
        problem: answer == null ? (failure ?? 'The host gave no answer for it') : answer.problem,
      );
      _reading.remove(id);
    }
    notifyListeners();
  }

  Future<void> _load(String label) async {
    if (!_loaded.add(label)) return;
    final text = await storage?.read(label);
    if (text == null) return;
    try {
      final json = jsonDecode(text);
      if (json is Map<Object?, Object?> && json['entries'] is Map<Object?, Object?>) {
        for (final MapEntry(:key, :value) in (json['entries']! as Map<Object?, Object?>).entries) {
          if (EntryContents.fromJson(value) case final held?) _entries.putIfAbsent('$label/$key', () => held);
        }
      }
    } on FormatException {
      // A cache that cannot be read is read again from the sources.
    }
  }

  Future<void> _save(String label) async {
    final prefix = '$label/';
    final json = jsonEncode({
      'version': 1,
      'entries': {
        for (final MapEntry(:key, :value) in _entries.entries)
          if (key.startsWith(prefix)) key.substring(prefix.length): value.toJson(),
      },
    });
    try {
      await storage?.write(label, json);
    } on Object {
      // The cache is only a cache.
    }
  }
}
