/// AOS v0 catalogs (spec/v0/catalog.md): a catalog read by its file name, the entries it lists with each source made
/// absolute against where the catalog came from, the entries it refuses and the informational fields it leaves out, and
/// the listing of several catalogs by the label the person gave each. Held to the specification's `catalog.json` and
/// the catalog cases of `reading.json`. Only Canoa's store reads catalogs; nothing here fetches, runs or reads a file.
///
/// A catalog is advertising: its fields say only what its author wrote. What a store shows of an overlay once fetched
/// comes from the overlay's own manifest and files.
library;

import 'dart:convert';

import 'aos.dart';

/// A catalog's reading limits: settings with the specification's recommended defaults, 4 MiB and a depth of 64.
const defaultCatalogLimits = AosLimits(maxBytes: 4 * 1024 * 1024, maxDepth: 64);

/// Reads a catalog file named [name]: `.toml` and `.json`, in any ASCII case, name their format and are parsed whole
/// after one byte-order mark; any other name is read as `overlay.cfg` is, by its format line, else TOML. YAML (`.yaml`,
/// `.yml`, or a `#!yaml` line) is refused as an unsupported format, as Canoa refuses it in a manifest.
Map<String, Object> readCatalogDocument(String name, List<int> bytes, {AosLimits limits = defaultCatalogLimits}) {
  final last = name.replaceAll(r'\', '/').split('/').last;
  final dot = last.lastIndexOf('.');
  final extension = dot < 0 ? '' : _asciiLower(last.substring(dot));
  return switch (extension) {
    '.toml' => readDocumentAs('toml', bytes, limits: limits),
    '.json' => readDocumentAs('json', bytes, limits: limits),
    '.yaml' || '.yml' => bytes.length > limits.maxBytes
        ? throw const AosException('too-large')
        : throw const AosException('unsupported-format', 'yaml'),
    _ => readDocument(bytes, limits: limits),
  };
}

/// A catalog in the minimal form lists every overlay by a string; one table entry makes it full.
enum CatalogForm { minimal, full }

/// One overlay a catalog lists, as its author wrote it: never merged with another catalog's.
final class CatalogEntry {
  const CatalogEntry({required this.name, required this.source, this.pin, this.description, this.about = const {}});

  /// The name as the catalog writes it, a valid overlay name.
  final String name;

  /// Its source, made absolute against the catalog's own location, with the entry's `ref` applied.
  final AosSource source;

  /// The tree digest the catalog's author reviewed, as written, or null: the entry is then unpinned.
  final String? pin;

  final String? description;

  /// The valid fields of its `about`; an icon here is a file beside the catalog.
  final Map<String, Object> about;

  String get identity => normalizeName(name);
}

/// What a store reads from one catalog.
final class AosCatalog {
  const AosCatalog({required this.form, required this.entries, required this.refused, required this.ignored,
      this.name, this.description, this.homepage});

  final CatalogForm form;

  /// The catalog's own informational fields, when valid.
  final String? name, description, homepage;

  /// The entries it lists, by name, in byte order of their names.
  final Map<String, CatalogEntry> entries;

  /// Each entry refused alone, `(name, class)`, by name.
  final List<(String, String)> refused;

  /// JSON pointers to each informational field left out, sorted.
  final List<String> ignored;
}

final _ref = RegExp(r'^[^-/:\\@#\x00-\x20\x7f][^/:\\@#\x00-\x20\x7f]*$');
final _space = RegExp(r'\s', unicode: true);
final _drive = RegExp(r'^[A-Za-z]:');

/// Whether a URL [location] carries a password in its user information (`scheme://user:secret@host`), which a source
/// Canoa stores or shows may not.
bool holdsPassword(String location) {
  final scheme = location.indexOf('://');
  if (scheme < 0) return false;
  final rest = location.substring(scheme + 3);
  final end = rest.indexOf(RegExp(r'[/?#]'));
  final authority = end < 0 ? rest : rest.substring(0, end);
  final at = authority.lastIndexOf('@');
  return at >= 0 && authority.substring(0, at).contains(':');
}
final _digest = RegExp(r'^sha256:[0-9a-fA-F]{64}$');
const _entryKeys = {'source', 'ref', 'pin', 'description', 'about'};

String _asciiLower(String text) =>
    String.fromCharCodes(text.codeUnits.map((c) => c >= 0x41 && c <= 0x5a ? c + 32 : c));

String _escape(String token) => token.replaceAll('~', '~0').replaceAll('/', '~1');

int _byBytes(String a, String b) => compareBytes(utf8.encode(a), utf8.encode(b));

final class _Refused implements Exception {
  const _Refused(this.code);
  final String code;
}

bool _absoluteDirectory(String location) => location.startsWith('/') || location.startsWith(r'\') || _drive.hasMatch(location);

/// A Git location on this machine: a path or `file:`, not a URL or `host:path`.
bool _localGit(String location) {
  final scheme = location.indexOf('://');
  if (scheme >= 0) return _asciiLower(location.substring(0, scheme)) == 'file';
  return _drive.hasMatch(location) || !RegExp(r'^[^/\\:]+:').hasMatch(location);
}

/// `/`-separated [parts] joined with `.` and `..` removed, as POSIX `normpath` does: `..` past the start of a relative
/// path is kept, past the root of an absolute one dropped; an empty result is `.`.
String _join(List<String> parts) {
  final joined = parts.where((part) => part.isNotEmpty).join('/');
  if (joined.isEmpty) return '.';
  final absolute = joined.startsWith('/');
  final out = <String>[];
  for (final segment in joined.split('/')) {
    if (segment.isEmpty || segment == '.') continue;
    if (segment == '..') {
      if (out.isNotEmpty && out.last != '..') {
        out.removeLast();
      } else if (!absolute) {
        out.add('..');
      }
      continue;
    }
    out.add(segment);
  }
  final body = out.join('/');
  return absolute ? '/$body' : (body.isEmpty ? '.' : body);
}

String _dirname(String path) {
  final slash = path.lastIndexOf('/');
  if (slash < 0) return '';
  return slash == 0 ? '/' : path.substring(0, slash);
}

/// [entry] made absolute against [origin], the source the catalog file came from, or refused.
AosSource _resolve(AosSource entry, AosSource? origin) {
  final location = entry.location;
  final relative = entry.kind == SourceKind.directory && !_absoluteDirectory(location);
  final local = (entry.kind == SourceKind.directory && !relative) || (entry.kind == SourceKind.git && _localGit(location));
  if (origin == null || origin.kind == SourceKind.directory) {
    if (relative && origin != null) {
      final folder = _dirname(origin.location.replaceAll(r'\', '/'));
      return AosSource(entry.kind, _join([folder, location]), entry.reference, entry.path);
    }
    return entry;
  }
  if (local) throw const _Refused('local-source');
  if (!relative) return entry;
  if (origin.kind == SourceKind.git) {
    final path = _join([_dirname(origin.path ?? ''), location, entry.path ?? '']);
    if (path == '..' || path.startsWith('../')) throw const _Refused('leaves-repository');
    if (path != '.' && !isRelativePath(path)) throw const _Refused('invalid-source');
    return AosSource(SourceKind.git, origin.location, origin.reference, path == '.' ? null : path);
  }
  final Uri resolved;
  try {
    resolved = Uri.parse(origin.location).resolve(location);
  } on FormatException {
    throw const _Refused('invalid-source');
  }
  return AosSource(SourceKind.url, '$resolved', null, entry.path);
}

/// What a store shows of an `about` table at [at]: each invalid or unknown field left out and its pointer added to
/// [ignored]; null when nothing is left, or when it is not a table, which is then left out whole.
Map<String, Object>? storeAbout(Object? about, String at, List<String> ignored) {
  if (about is! Map<String, Object?>) {
    ignored.add(at);
    return null;
  }
  final shown = <String, Object>{};
  for (final MapEntry(key: field, :value) in about.entries) {
    if (value != null && aboutFieldValid(field, value)) {
      shown[field] = value;
    } else {
      ignored.add('$at/${_escape(field)}');
    }
  }
  return shown.isEmpty ? null : shown;
}

CatalogEntry _entry(String name, Object? value, String at, AosSource? origin, List<String> ignored) {
  final Map<String, Object?> fields;
  if (value is String) {
    fields = {'source': value};
  } else if (value is Map<String, Object?>) {
    if (value.keys.any((key) => !_entryKeys.contains(key))) throw const _Refused('invalid-entry');
    fields = value;
  } else {
    throw const _Refused('invalid-entry');
  }
  final text = fields['source'];
  if (text is! String) throw const _Refused('invalid-entry');
  final AosSource parsed;
  try {
    parsed = parseSource(text);
  } on AosException {
    throw const _Refused('invalid-source');
  }
  var source = _resolve(parsed, origin);
  if (fields.containsKey('ref')) {
    final ref = fields['ref'];
    if (ref is! String || !_ref.hasMatch(ref) || ref.runes.any((c) => c >= 0x80 && c <= 0x9f) ||
        _space.hasMatch(ref)) {
      throw const _Refused('invalid-entry');
    }
    if (parsed.kind != SourceKind.git || parsed.reference != null) throw const _Refused('ref-not-allowed');
    source = AosSource(source.kind, source.location, ref, source.path);
  }
  String? pin;
  if (fields.containsKey('pin')) {
    final value = fields['pin'];
    if (value is! String || !_digest.hasMatch(value)) throw const _Refused('invalid-pin');
    pin = value;
  }
  String? description;
  if (fields.containsKey('description')) {
    if (fields['description'] case final String text) {
      description = text;
    } else {
      ignored.add('$at/description');
    }
  }
  Map<String, Object>? about;
  if (fields.containsKey('about')) about = storeAbout(fields['about'], '$at/about', ignored);
  return CatalogEntry(name: name, source: source, pin: pin, description: description, about: about ?? const {});
}

/// The catalog in [data], read from [base], the source of the catalog file itself (null when it has none): its form,
/// informational fields, the entries it lists, those it refuses and the fields it leaves out. Throws `not-a-catalog`
/// when [data] has no `overlays` mapping.
AosCatalog parseCatalog(Map<String, Object?> data, {AosSource? base}) {
  final overlays = data['overlays'];
  if (overlays is! Map<String, Object?>) throw const AosException('not-a-catalog');
  final ignored = <String>[];
  String? field(String key, bool Function(Object) valid) {
    if (!data.containsKey(key)) return null;
    final value = data[key];
    if (value is String && valid(value)) return value;
    ignored.add('/$key');
    return null;
  }

  final name = field('name', (value) => isDisplayLine(value as String));
  final description = field('description', (_) => true);
  final homepage = field('homepage', (value) => isWebUrl(value as String));
  final holders = <String, int>{};
  for (final key in overlays.keys) {
    if (validOverlayName(key)) holders.update(normalizeName(key), (count) => count + 1, ifAbsent: () => 1);
  }
  final entries = <String, CatalogEntry>{};
  final refused = <(String, String)>[];
  for (final key in overlays.keys.toList()..sort(_byBytes)) {
    if (!validOverlayName(key)) {
      refused.add((key, 'invalid-name'));
    } else if (holders[normalizeName(key)]! > 1) {
      refused.add((key, 'duplicate-name'));
    } else {
      try {
        entries[key] = _entry(key, overlays[key], '/overlays/${_escape(key)}', base, ignored);
      } on _Refused catch (refusal) {
        refused.add((key, refusal.code));
      }
    }
  }
  return AosCatalog(
    form: overlays.values.every((value) => value is String) ? CatalogForm.minimal : CatalogForm.full,
    name: name,
    description: description,
    homepage: homepage,
    entries: entries,
    refused: refused,
    ignored: ignored..sort(_byBytes),
  );
}

/// [readCatalogDocument] then [parseCatalog].
AosCatalog readCatalog(String name, List<int> bytes, {AosSource? base, AosLimits limits = defaultCatalogLimits}) =>
    parseCatalog(readCatalogDocument(name, bytes, limits: limits), base: base);

/// One entry in a listing of several catalogs.
final class ListedEntry {
  const ListedEntry(this.label, this.entry, {required this.shown});

  /// The label of the catalog it comes from, the person's own name for that catalog.
  final String label;
  final CatalogEntry entry;

  /// Its name as shown: `<label>/<name>` when more than one catalog offers its identity, else its name.
  final String shown;

  String get identity => entry.identity;
}

/// Several catalogs listed together, by the label the person gave each.
final class CatalogListing {
  const CatalogListing(this.entries, this.collisions);

  /// Every entry, by identity and then label; one per catalog that offers it, never merged.
  final List<ListedEntry> entries;

  /// Each identity more than one catalog offers, with their labels, sorted.
  final Map<String, List<String>> collisions;

  /// The entry [requested] names: `<label>/<name>`, or a name one catalog alone offers. Throws `ambiguous` for a name
  /// more than one offers, which a store never settles by itself, and `not-found`.
  ListedEntry choose(String requested) {
    final slash = requested.lastIndexOf('/');
    final label = slash < 0 ? null : requested.substring(0, slash);
    final name = slash < 0 ? requested : requested.substring(slash + 1);
    if (!validOverlayName(name) || (label != null && !validOverlayName(label))) {
      throw const AosException('not-found');
    }
    final found = [
      for (final listed in entries)
        if (listed.identity == normalizeName(name) &&
            (label == null || normalizeName(listed.label) == normalizeName(label)))
          listed,
    ];
    if (found.isEmpty) throw const AosException('not-found');
    if (found.length > 1) throw const AosException('ambiguous');
    return found.single;
  }
}

/// Lists [catalogs], each `(label, catalog)`. A label is a valid overlay name, unique when normalized, else this throws
/// `invalid-label`. An identity two catalogs offer is listed once per catalog, qualified by its label.
CatalogListing listCatalogs(List<(String, AosCatalog)> catalogs) {
  final labels = {for (final (label, _) in catalogs) normalizeName(label)};
  if (labels.length != catalogs.length || catalogs.any((pair) => !validOverlayName(pair.$1))) {
    throw const AosException('invalid-label');
  }
  final offered = <String, List<(String, CatalogEntry)>>{};
  for (final (label, catalog) in catalogs) {
    for (final entry in catalog.entries.values) {
      (offered[entry.identity] ??= []).add((label, entry));
    }
  }
  final entries = <ListedEntry>[];
  final collisions = <String, List<String>>{};
  for (final identity in offered.keys.toList()..sort(_byBytes)) {
    final holders = offered[identity]!
      ..sort((a, b) {
        final byLabel = _byBytes(a.$1, b.$1);
        return byLabel != 0 ? byLabel : _byBytes(a.$2.name, b.$2.name);
      });
    final qualified = holders.length > 1;
    if (qualified) collisions[identity] = [for (final (label, _) in holders) label];
    for (final (label, entry) in holders) {
      entries.add(ListedEntry(label, entry, shown: qualified ? '$label/${entry.name}' : entry.name));
    }
  }
  return CatalogListing(entries, collisions);
}
