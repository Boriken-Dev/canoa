// A stand-in for the host's catalog index in widget tests: reads the catalogs a MemoryOverlays holds the way the host's index
// does, and gives them as the store lists them. The host's own reading is tested in Rust; this keeps the widget tests on
// their in-memory repositories.
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_index.dart';
import 'package:canoa/store/store_settings.dart';
import 'package:coqui_aos/coqui_aos.dart' as aos;

/// The catalogs of [config] as [overlays] holds them: what `store/index` would answer.
Future<List<LoadedCatalog>> standInCatalogs(OverlayManager overlays, CanoaConfig config) async => [
  for (final MapEntry(key: name, value: from) in config.catalogs.entries) await loadCatalog(overlays, name, from, config.store),
];

String _dirname(String path) {
  final slash = path.lastIndexOf('/');
  return slash < 0 ? '' : path.substring(0, slash);
}

/// Fetches and reads the catalog labelled [label]: the file [source]'s `#path` names, at most
/// [StoreSettings.catalogMaxBytes], and each entry's icon beside it, its size and header checked against the icon
/// limits before anything decodes it. A refusal is the catalog's problem, never an exception.
Future<LoadedCatalog> loadCatalog(OverlayManager overlays, String label, GitOverlay source, StoreSettings settings) async {
  LoadedCatalog failed(String problem, [String? commit]) =>
      LoadedCatalog(label: label, source: source, commit: commit, problem: problem);
  final FetchedTree tree;
  try {
    tree = await overlays.fetchCatalog(label, source);
  } on OverlayException catch (error) {
    return failed('It cannot be fetched: $error');
  }
  final file = await tree.file(source.path, maxBytes: settings.catalogMaxBytes);
  final StoreCatalog catalog;
  final int all;
  if (file == null) {
    // A directory is a catalog in the minimal form: each subdirectory is an overlay of that name.
    final directories = await tree.directories(source.path);
    if (directories == null) return failed('It has no file ${source.path} at that commit', tree.commit);
    final listed = directoryCatalog(source, directories, settings);
    if (listed.catalog.entries.isEmpty && listed.catalog.refused.isEmpty) {
      return failed('It is a directory with no overlays at that commit', tree.commit);
    }
    (catalog, all) = (listed.catalog, listed.all);
  } else {
    final bytes = file.bytes;
    if (bytes == null) {
      return failed('Its file holds ${file.size} bytes, more than the ${settings.catalogMaxBytes} [store] allows',
          tree.commit);
    }
    final aos.Catalog read;
    try {
      read = aos.readCatalog(Uint8List.fromList(bytes), source.path,
          'git+${source.repo}@${source.reference}${source.path.isEmpty ? '' : '#${source.path}'}', settings.catalogLimits);
    } on aos.AosError catch (error) {
      return failed('It is not a catalog Canoa reads (${error.errorClass})', tree.commit);
    }
    // Entries are in byte order of their names: those past the limit are left out, and counted.
    final every = <String, CatalogEntry>{
      for (final name in read.overlays.entries.keys.toList()..sort((a, b) => compareBytes(utf8.encode(a), utf8.encode(b))))
        name: catalogEntryOf(name, read.overlays.entries[name]!),
    };
    final kept = every.keys.take(settings.catalogMaxEntries).toList();
    all = every.length;
    catalog = StoreCatalog(name: read.name, description: read.description, homepage: read.homepage,
        entries: {for (final name in kept) name: every[name]!}, refused: read.overlays.refused, ignored: read.ignored,
        skills: {
          for (final name in ([...?read.skills?.entries.keys]
                ..sort((a, b) => compareBytes(utf8.encode(a), utf8.encode(b))))
              .take(settings.catalogMaxEntries))
            name: read.skills!.entries[name]!,
        });
  }
  // Each icon file is read once, however many entries name it, and no more files than the limit, all in one batch.
  final wanted = <String>{};
  final pathOf = <String, String?>{};
  final folder = _dirname(source.path);
  for (final entry in catalog.entries.values) {
    final icon = entry.about['icon'];
    if (icon is! String) continue;
    final path = folder.isEmpty ? icon : '$folder/$icon';
    if (!wanted.contains(path) && wanted.length >= settings.catalogMaxIcons) {
      pathOf[entry.name] = null;
    } else {
      wanted.add(path);
      pathOf[entry.name] = path;
    }
  }
  final fetched = wanted.isEmpty ? const <String, TreeFile?>{} : await tree.readFiles(wanted.toList(), maxBytes: settings.iconMaxBytes);
  final byPath = {for (final path in wanted) path: _icon(fetched[path], path, settings)};
  final icons = <String, CatalogIcon>{
    for (final MapEntry(key: name, value: path) in pathOf.entries)
      name: path == null ? (bytes: null, refused: 'icon-limit') : byPath[path]!,
  };
  return LoadedCatalog(label: label, source: source, commit: tree.commit, catalog: catalog, icons: icons,
      leftOut: all - catalog.entries.length);
}

/// The catalog a directory of overlays is: a subdirectory with a valid overlay name is an entry of that name whose source
/// is the subdirectory at the catalog's repository and ref; a name that is not valid, or that another's normalized form
/// equals, is refused. `all` counts the entries before [StoreSettings.catalogMaxEntries] left some out.
({StoreCatalog catalog, int all}) directoryCatalog(GitOverlay source, List<String> names, StoreSettings settings) {
  final refused = <(String, String)>[];
  final byIdentity = <String, List<String>>{};
  for (final name in names) {
    if (aos.validOverlayName(name)) {
      byIdentity.putIfAbsent(aos.normalizeName(name), () => []).add(name);
    } else {
      refused.add((name, 'invalid-name'));
    }
  }
  final folder = source.path.replaceAll(RegExp(r'/+$'), '');
  final entries = <String, CatalogEntry>{};
  for (final same in byIdentity.values) {
    if (same.length > 1) {
      refused.addAll([for (final name in same) (name, 'duplicate-name')]);
    } else {
      final name = same.single;
      entries[name] = CatalogEntry(name, aos.Source('git', source.repo, source.reference, folder.isEmpty ? name : '$folder/$name'));
    }
  }
  final sorted = entries.keys.toList()..sort((a, b) => compareBytes(utf8.encode(a), utf8.encode(b)));
  return (
    catalog: StoreCatalog(entries: {for (final name in sorted.take(settings.catalogMaxEntries)) name: entries[name]!},
        refused: refused, ignored: const []),
    all: sorted.length,
  );
}

CatalogEntry catalogEntryOf(String name, aos.CatalogEntry entry) =>
    CatalogEntry(name, entry.source, pin: entry.pin, description: entry.description, about: entry.about ?? const {});

CatalogIcon _icon(TreeFile? found, String path, StoreSettings settings) {
  try {
    if (found == null) return (bytes: null, refused: 'icon-missing');
    final bytes = found.bytes;
    if (bytes == null) return (bytes: null, refused: 'icon-too-large');
    aos.readIcon(Uint8List.fromList(bytes), path, settings.iconLimits);
    return (bytes: bytes, refused: null);
  } on aos.AosError catch (error) {
    return (bytes: null, refused: error.errorClass);
  }
}

/// The index a page is given in a widget test: [current] is the configuration the person has now, since the host reads
/// `[store.overlays.catalogs]` from its canoa.cfg each time it is asked.
CatalogIndex standInIndex(OverlayManager overlays, CanoaConfig Function() current) =>
    ({bool refresh = false}) => standInCatalogs(overlays, current());
