import 'dart:async';
import 'dart:convert';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/foundation.dart';

import '../profile/overlays.dart';
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

/// What every entry of every catalog provides, read ahead when the catalogs are read so that opening an entry shows it without
/// asking for anything. A reading is kept under the cache directory by catalog, and is current while the entry's pin (or, when
/// it has none, its catalog's commit) is the one it was made for and, for an entry with no pin, for [readAhead]'s `ttl`.
///
/// Reading fetches an entry's files as an install's review does, into a place nothing is installed from, and runs nothing.
/// One entry that cannot be read is noted against it and the rest go on.
class ContentsStore extends ChangeNotifier {
  ContentsStore({required this.overlays, required this.os, this.storage, DateTime Function()? now}) : _now = now ?? DateTime.now;

  final OverlayManager overlays;
  final String os;
  final ContentsStorage? storage;
  final DateTime Function() _now;

  final _entries = <String, EntryContents>{};
  final _reading = <String>{};
  final _loaded = <String>{};
  final _byName = <String, Future<void>>{};
  var _disposed = false;

  /// How many entries have been fetched to be read, since this store was made.
  int fetches = 0;

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

  /// Reads every entry of [catalogs] whose last reading is not current, at most [concurrency] at a time. [force] reads them all
  /// again. [ttl] is how long an entry with no pin stays current; zero reads again every time.
  Future<void> readAhead(List<LoadedCatalog> catalogs, {required Duration ttl, bool force = false, int concurrency = 3}) async {
    _running++;
    try {
      final work = <(String, String, Future<void> Function(String))>[];
      for (final loaded in catalogs) {
        final catalog = loaded.catalog;
        if (catalog == null) continue;
        await _load(loaded.label);
        final fallback = loaded.commit ?? '';
        void want(String kind, String name, String? pin, Future<OverlayContents> Function() read) {
          final id = contentsId(loaded.label, kind, name);
          final key = pin ?? fallback;
          final held = _entries[id];
          final current = held != null &&
              held.key == key &&
              (pin != null || ttl > Duration.zero && _now().difference(held.readAt) < ttl);
          if (force || !current) work.add((loaded.label, id, (id) => _read(id, key, read)));
        }

        for (final entry in catalog.entries.values) {
          want('overlay', entry.name, entry.pin, () => _readOverlay(entry));
        }
        for (final MapEntry(key: name, value: entry) in catalog.skills.entries) {
          want('skill', name, entry.pin, () => _readSkill(name, entry));
        }
      }
      var next = 0;
      final touched = <String>{};
      Future<void> worker() async {
        while (next < work.length) {
          final (label, id, run) = work[next++];
          touched.add(label);
          await run(id);
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

  Future<void> _read(String id, String key, Future<OverlayContents> Function() read) async {
    _reading.add(id);
    notifyListeners();
    EntryContents result;
    try {
      result = EntryContents(key: key, readAt: _now(), contents: await read());
    } on OverlayException catch (error) {
      result = EntryContents(key: key, readAt: _now(), problem: error.message);
    } on StoreRefusal catch (error) {
      result = EntryContents(key: key, readAt: _now(), problem: error.message);
    } on aos.AosError catch (error) {
      result = EntryContents(key: key, readAt: _now(), problem: 'Its files cannot be read as an overlay (${error.errorClass})');
    } on Object catch (error) {
      result = EntryContents(key: key, readAt: _now(), problem: '$error');
    }
    _entries[id] = result;
    _reading.remove(id);
    notifyListeners();
  }

  /// Reads one overlay: its files fetched as a review fetches them, then discarded when nothing is installed under the name.
  /// An entry that installs under the same name as another is read after it, since they share a place.
  Future<OverlayContents> _readOverlay(CatalogEntry entry) {
    final name = installName(entry);
    final before = _byName[name] ?? Future<void>.value();
    final done = Completer<OverlayContents>();
    _byName[name] = before.then((_) async {
      try {
        final update = await overlays.check(name, installSource(entry).pinnedTo(null));
        fetches++;
        try {
          done.complete(readContents(await overlays.read(update), name: entry.name, os: os));
        } finally {
          await overlays.discard(name);
        }
      } on Object catch (error, stack) {
        done.completeError(error, stack);
      }
    });
    return done.future;
  }

  Future<OverlayContents> _readSkill(String name, aos.CatalogEntry entry) async {
    fetches++;
    return readSkillContents(await overlays.fetchSkillFiles(entry.source), name: name);
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
