/// The store: catalogs fetched as overlays are, their entries listed and searched, and an overlay fetched to show what
/// it would do before it is installed through the same path as an overlay named by hand in canoa.cfg.
///
/// A catalog is advertising (AOS catalog.md, "Trust"): what a listing shows before fetching is the catalog's claim, and
/// what the overlay page shows after fetching comes from the overlay's own manifest and files.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../config/canoa_config.dart';
import '../profile/aos_support.dart';
import '../profile/overlay_layer.dart';
import '../profile/overlays.dart';
import '../profile/profile.dart';
import 'store_settings.dart';

/// Why the store will not do something, said to the person. Never quotes a file's content.
final class StoreRefusal implements Exception {
  const StoreRefusal(this.message);
  final String message;
  @override
  String toString() => message;
}

/// A catalog's icon for one entry: its bytes once the header passed the limits, or the class it was refused by.
typedef CatalogIcon = ({List<int>? bytes, String? refused});

/// One entry of a catalog as the store shows it: its name there, where it comes from and what the catalog claims.
final class CatalogEntry {
  const CatalogEntry(this.name, this.source, {this.pin, this.description, this.about = const {}});

  final String name;
  final aos.Source source;
  final String? pin;
  final String? description;

  /// The valid fields of the entry's `about`.
  final Map<String, Object> about;
}

/// A catalog as the store keeps it: its entries by name, in byte order of their names.
final class StoreCatalog {
  const StoreCatalog({required this.entries, required this.refused, required this.ignored, this.name, this.description,
      this.homepage});

  final String? name, description, homepage;
  final Map<String, CatalogEntry> entries;

  /// `(name, class)` of each entry refused.
  final List<(String, String)> refused;

  /// The pointers of the fields left out.
  final List<String> ignored;
}

/// One catalog as the store read it, or why it could not.
final class LoadedCatalog {
  const LoadedCatalog({required this.label, required this.source, this.commit, this.catalog, this.problem,
      this.icons = const {}, this.leftOut = 0});

  /// How many entries past `[store] catalog_max_entries` were left out.
  final int leftOut;

  final String label;
  final GitOverlay source;
  final String? commit;
  final StoreCatalog? catalog;

  /// Why the whole catalog is left out.
  final String? problem;

  /// Each entry's icon, by entry name, for entries whose `about` names one.
  final Map<String, CatalogIcon> icons;
}

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
  if (file == null) return failed('It has no file ${source.path} at that commit', tree.commit);
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
  final all = <String, CatalogEntry>{
    for (final name in read.overlays.entries.keys.toList()..sort((a, b) => compareBytes(utf8.encode(a), utf8.encode(b))))
      name: catalogEntryOf(name, read.overlays.entries[name]!),
  };
  final kept = all.keys.take(settings.catalogMaxEntries).toList();
  final catalog = StoreCatalog(name: read.name, description: read.description, homepage: read.homepage,
      entries: {for (final name in kept) name: all[name]!}, refused: read.overlays.refused, ignored: read.ignored);
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
      leftOut: all.length - kept.length);
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

/// [source] as a store shows it: without user information or query, which can hold a credential.
String displaySource(aos.Source source) => source.shown();

/// One entry in a listing of several catalogs.
final class ListedEntry {
  const ListedEntry(this.label, this.entry, {required this.shown});

  /// The label of the catalog it comes from, the person's own name for that catalog.
  final String label;
  final CatalogEntry entry;

  /// Its name as shown: `<label>/<name>` when more than one catalog offers its identity, else its name.
  final String shown;

  String get identity => aos.normalizeName(entry.name);
}

/// The listing of every catalog read, searchable.
final class StoreListing {
  StoreListing(this.catalogs) : listing = aos.listCatalogs([for (final loaded in catalogs) ?_read(loaded)]) {
    entries = [
      for (final listed in listing.entries)
        ListedEntry(listed.catalog, catalogs.firstWhere((loaded) => loaded.label == listed.catalog).catalog!.entries[listed.name]!,
            shown: listed.shown),
    ];
  }

  static (String, aos.Catalog)? _read(LoadedCatalog loaded) {
    final catalog = loaded.catalog;
    if (catalog == null) return null;
    // listCatalogs reads names only: the entries need no source.
    final read = aos.Catalog(
      form: 'minimal',
      name: null,
      description: null,
      homepage: null,
      skills: null,
      overlays: aos.CatalogSection()
        ..entries.addAll({
          for (final entry in catalog.entries.values)
            entry.name: aos.CatalogEntry(source: entry.source, pin: entry.pin, description: entry.description, about: entry.about),
        }),
      ignored: const [],
    );
    return (loaded.label, read);
  }

  final List<LoadedCatalog> catalogs;
  final aos.Listing listing;

  /// Every entry, in the order [aos.listCatalogs] lists them.
  late final List<ListedEntry> entries;

  /// The [count] tags the most entries carry, the most used first, then by name.
  List<String> topTags(int count) {
    final uses = <String, int>{};
    for (final listed in entries) {
      for (final tag in _tags(listed.entry).toSet()) {
        uses.update(tag, (n) => n + 1, ifAbsent: () => 1);
      }
    }
    return (uses.keys.toList()..sort((a, b) => uses[a] != uses[b] ? uses[b]! - uses[a]! : a.compareTo(b)))
        .take(count)
        .toList();
  }

  /// Every tag the listed entries carry, sorted.
  late final List<String> tags =
      {for (final listed in entries) ..._tags(listed.entry)}.toList()..sort();

  LoadedCatalog catalogOf(ListedEntry listed) => catalogs.firstWhere((loaded) => loaded.label == listed.label);

  /// The entries whose shown name, summary, description, tags, catalog or source hold every word of [text], ignoring
  /// case, and that carry every tag of [tags].
  List<ListedEntry> search(String text, {Set<String> tags = const {}}) {
    final words = text.toLowerCase().split(RegExp(r'\s+')).where((word) => word.isNotEmpty).toList();
    return [
      for (final listed in entries)
        if (tags.every(_tags(listed.entry).contains) && words.every(_haystack(listed).contains)) listed,
    ];
  }

  /// The entry [requested] names, as `<name>` or `<label>/<name>`. Throws [aos.AosError] `not-found` or `ambiguous`.
  ListedEntry choose(String requested) {
    final chosen = aos.chooseListed(listing, requested);
    return entries.firstWhere((listed) => listed.label == chosen.catalog && listed.entry.name == chosen.name);
  }

  static List<String> _tags(CatalogEntry entry) => [
    if (entry.about['tags'] case final List<Object?> tags) for (final tag in tags) '$tag',
  ];

  static String _haystack(ListedEntry listed) => [
    listed.shown,
    listed.entry.name,
    listed.label,
    '${listed.entry.about['summary'] ?? ''}',
    listed.entry.description ?? '',
    ..._tags(listed.entry),
    displaySource(listed.entry.source),
  ].join('\n').toLowerCase();
}

/// The name an entry installs under in `[overlays]`: its own when Canoa can name a directory so, else its normalized
/// form, which is the same AOS identity.
String installName(CatalogEntry entry) {
  if (isEntryName(entry.name)) return entry.name;
  final normalized = aos.normalizeName(entry.name);
  if (isEntryName(normalized)) return normalized;
  throw StoreRefusal('Canoa names an overlay with 1 to 64 letters, digits, - or _, and ${entry.name} has no such form');
}

/// The git source Canoa installs [entry] from, held to what an overlay named by hand in canoa.cfg is held to.
GitOverlay installSource(CatalogEntry entry) {
  final source = entry.source;
  if (source.kind != 'git') {
    throw const StoreRefusal('Canoa installs overlays from git only, and this source is not git');
  }
  final reference = source.reference;
  if (reference == null) {
    throw const StoreRefusal('It names no branch, tag or commit, and Canoa fetches only a named one');
  }
  if (holdsPassword(source.location)) {
    throw const StoreRefusal('Its source holds a password, which canoa.cfg may not: it is not installed from here');
  }
  try {
    return OverlaySource.parseGit('${source.location}@$reference${source.path == null ? '' : '#${source.path}'}');
  } on ProfileException {
    throw const StoreRefusal('Canoa would refuse this source in canoa.cfg: a transport or ref git must not take');
  }
}

/// How an entry stands against canoa.cfg's `[overlays]`.
enum EntryState {
  /// No overlay of its identity is named.
  absent,

  /// An overlay of its identity is named from this very source, and the store installed it.
  installed,

  /// An overlay of its identity is named from this very source, but not by the store: the store neither updates nor
  /// removes what a person named by hand.
  byHand,

  /// An overlay of its identity is named from another source: the store neither installs over it nor removes it.
  taken,
}

/// [name] as `[overlays]` has it, when an overlay of the same AOS identity is named there.
String? namedAs(CanoaConfig config, String name) =>
    config.overlays.keys.where((key) => aos.normalizeName(key) == aos.normalizeName(name)).firstOrNull;

/// How the entry [name] from [source] stands against `[overlays]` and what [overlays] recorded as installed by the store.
EntryState entryState(CanoaConfig config, String name, GitOverlay source, {OverlayManager? overlays}) {
  final named = namedAs(config, name);
  if (named == null) return EntryState.absent;
  final present = config.overlays[named];
  if (present is! GitOverlay || present.repo != source.repo || present.reference != source.reference ||
      present.path != source.path) {
    return EntryState.taken;
  }
  return overlays?.installed(named)?.catalog == null ? EntryState.byHand : EntryState.installed;
}

/// An overlay fetched, not installed, and what it would do, all from its own manifest and files.
final class OverlayPreview {
  const OverlayPreview({required this.update, required this.manifest, required this.about, required this.ignored,
      this.icon, this.iconRefused, this.layer, this.report, this.refusal, this.pinRefusal});

  /// What installing it would change; the same object the install applies.
  final OverlayUpdate update;

  /// Its `overlay.cfg` as read, or empty when it has none or it cannot be read.
  final aos.Document manifest;

  /// The valid fields of its own `[about]`.
  final Map<String, Object> about;

  /// The pointers of the `[about]` fields left out.
  final List<String> ignored;

  /// Its own icon's bytes, once the header passed the limits.
  final List<int>? icon;
  final String? iconRefused;

  /// The layer Canoa would make of it, as Settings, Profiles makes it; null when Canoa would refuse it.
  final Layer? layer;
  final OverlayReport? report;

  /// Why Canoa would leave it out whole, when it would.
  final String? refusal;

  /// Why it may not be installed from this entry: a pin that does not match the tree fetched.
  final String? pinRefusal;

  bool get installable => refusal == null && pinRefusal == null;

  String get name => update.name;

  /// The overlays its manifest requires, as written; none when `requires` is not a list.
  List<String> get requires => [
    if (manifest['requires'] case final List<Object?> items) for (final item in items) '$item',
  ];

  /// Each setup the manifest declares: its path, the OSes it is for, and whether it says it needs the network. What is
  /// not of the form AOS defines is shown as written, never acted on.
  List<(String, List<String>?, bool)> get setup => [
    if (manifest['setup'] case final List<Object?> entries)
      for (final entry in entries)
        if (entry is Map)
          ('${entry['path'] ?? ''}', entry['os'] is List ? [for (final os in entry['os'] as List) '$os'] : null,
              entry['network'] == true),
  ];

  /// The harnesses its manifest gives hints for.
  List<String> get harnesses => [
    if (manifest['harness'] case final Map<Object?, Object?> hints) for (final id in hints.keys) '$id',
  ];
}

/// Fetches [source] as [name] and computes what installing it would do, touching nothing installed: the manifest and
/// files are read from the fetched tree, the layer is the one Settings, Profiles would make of it, and the report is
/// its non-default settings and what Canoa does not apply. A catalog's [catalogPin] must match the tree fetched, or installing is
/// refused; canoa.cfg's own pin is held by [installPreview].
Future<OverlayPreview> previewOverlay(OverlayManager overlays, String name, GitOverlay source, {
  required String os,
  required Map<String, LayerId> shared,
  String? catalogPin,
  String? harness,
  aos.Settings? iconLimits,
}) async {
  final update = await overlays.check(name, source.pinnedTo(null));
  final files = await overlays.read(update);
  final ReadTree read;
  try {
    read = treeOfFiles(files);
  } on aos.AosError catch (error) {
    throw StoreRefusal('The files read cannot be an overlay (${error.errorClass})');
  }
  if (!sameDigest('${read.digest.digest}', update.digest)) {
    throw const StoreRefusal('The files read do not have the digest fetched');
  }
  String? pinRefusal;
  if (catalogPin != null && !sameDigest(catalogPin, update.digest)) {
    pinRefusal = 'The catalog pins $catalogPin, and what was fetched is ${update.digest}: refused';
  }
  aos.Document manifest = const {};
  if (files['overlay.cfg'] case final bytes?) {
    try {
      manifest = aos.readDocument(Uint8List.fromList(bytes), aos.defaultSettings);
    } on aos.AosError {
      manifest = const {};
    }
  }
  final ignored = <String>[];
  final about = manifest.containsKey('about') ? _storeAbout(manifest['about'], '/about', ignored) ?? const {} : const <String, Object>{};
  List<int>? icon;
  String? iconRefused;
  if (about['icon'] case final String path) {
    try {
      final bytes = files[path] ?? (throw aos.AosError('icon-missing'));
      aos.readIcon(Uint8List.fromList(bytes), path, iconLimits ?? aos.defaultSettings);
      icon = bytes;
    } on aos.AosError catch (error) {
      iconRefused = error.errorClass;
    }
  }
  Layer? layer;
  OverlayReport? report;
  String? refusal;
  try {
    layer = overlayLayer(identity: name, root: overlays.directoryOf(name), os: os, tree: read.tree, harness: harness,
        pinned: true, iconLimits: iconLimits);
    report = overlayReport(layer.id, layer.aos!, shared);
  } on OverlayRefusal catch (error) {
    refusal = '$error';
  }
  return OverlayPreview(update: update, manifest: manifest, about: about, ignored: ignored, icon: icon,
      iconRefused: iconRefused, layer: layer, report: report, refusal: refusal, pinRefusal: pinRefusal);
}

/// The pin canoa.cfg gives the overlay [name], when it names one, and the tree [preview] fetched has another digest:
/// installing it moves that pin, which [installPreview] does only when asked to.
String? pinMoved(CanoaConfig config, OverlayPreview preview) {
  final named = namedAs(config, preview.name);
  if (named == null) return null;
  return switch (config.overlays[named]) {
    GitOverlay(:final pin?) when !sameDigest(pin, preview.update.digest) => pin,
    _ => null,
  };
}

/// Installs or updates what [preview] showed, through the same path an overlay named by hand takes: the files at its
/// commit, checked against its tree digest as written, then the overlay named in canoa.cfg pinned to that digest. A
/// pin canoa.cfg already gives it holds as it does for a sync from Settings, Profiles: a tree of another digest is
/// installed only with [movePin], the person's choice to move it.
Future<CanoaConfig> installPreview(OverlayManager overlays, CanoaConfig config, OverlayPreview preview,
    {required String catalog, bool movePin = false}) async {
  if (preview.pinRefusal case final why?) throw StoreRefusal(why);
  if (preview.refusal case final why?) throw StoreRefusal('Canoa would leave it out: $why');
  final name = preview.name;
  final named = namedAs(config, name);
  if (named != null) {
    switch (entryState(config, name, preview.update.source, overlays: overlays)) {
      case EntryState.installed || EntryState.absent:
        break;
      case EntryState.byHand:
        throw StoreRefusal('canoa.cfg names $named by hand: remove it there first to let the store manage it');
      case EntryState.taken:
        throw StoreRefusal('canoa.cfg names $named from another source: remove it there first');
    }
  }
  if (pinMoved(config, preview) case final pin? when !movePin) {
    throw StoreRefusal('canoa.cfg pins $pin, and what was fetched is ${preview.update.digest}: nothing was installed');
  }
  final record = await overlays.apply(preview.update, catalog: catalog);
  return config.withOverlay(named ?? name, preview.update.source.pinnedTo(null), record.digest!);
}

/// Removes the overlay [name] the store installed from [source]: its files and fetched history, and its entry in
/// `[overlays]`. One named from another source, or by hand, is refused, never removed.
Future<CanoaConfig> removeInstalled(OverlayManager overlays, CanoaConfig config, String name, GitOverlay source) async {
  final named = namedAs(config, name);
  if (named == null) return config;
  if (entryState(config, name, source, overlays: overlays) != EntryState.installed) {
    throw StoreRefusal('canoa.cfg names $named from another source or by hand: the store does not remove it');
  }
  await overlays.remove(named);
  return config.withoutOverlay(named);
}

/// Removes what previewing [name] fetched, when nothing is installed under that name.
Future<void> discardPreview(OverlayManager overlays, String name) => overlays.discard(name);

/// A tag line for an overlay's `[about]` tags, as plain words.
String tagLine(Map<String, Object> about) => [for (final tag in (about['tags'] as List?) ?? const []) '$tag'].join(', ');

/// [bytes] as text for showing, never interpreted.
String plainText(List<int> bytes) => utf8.decode(bytes, allowMalformed: true);

/// What a store shows of an `about` table at [at]: each invalid or unknown field left out and its pointer added to
/// [ignored]; null when nothing is left, or when it is not a table, which is then left out whole.
Map<String, Object>? _storeAbout(Object? about, String at, List<String> ignored) {
  if (about is! Map<String, Object>) {
    ignored.add(at);
    return null;
  }
  final shown = <String, Object>{};
  for (final MapEntry(key: field, :value) in about.entries) {
    if (aos.aboutFieldValid(field, value)) {
      shown[field] = value;
    } else {
      ignored.add('$at/${field.replaceAll('~', '~0').replaceAll('/', '~1')}');
    }
  }
  return shown.isEmpty ? null : shown;
}
