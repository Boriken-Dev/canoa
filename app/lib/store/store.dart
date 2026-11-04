/// The store: catalogs fetched as overlays are, their entries listed and searched, and an overlay fetched to show what
/// it would do before it is installed through the same path as an overlay named by hand in Canoa.cfg.
///
/// A catalog is advertising (AOS catalog.md, "Trust"): what a listing shows before fetching is the catalog's claim, and
/// what the overlay page shows after fetching comes from the overlay's own manifest and files.
library;

import 'dart:convert';

import '../config/canoa_config.dart';
import '../profile/aos.dart';
import '../profile/aos_catalog.dart';
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

/// One catalog as the store read it, or why it could not.
final class LoadedCatalog {
  const LoadedCatalog({required this.label, required this.source, this.commit, this.catalog, this.problem,
      this.icons = const {}, this.leftOut = 0});

  /// How many entries past `[store] catalog_max_entries` were left out.
  final int leftOut;

  final String label;
  final GitOverlay source;
  final String? commit;
  final AosCatalog? catalog;

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
  final AosCatalog read;
  try {
    read = readCatalog(source.path, bytes,
        base: AosSource(SourceKind.git, source.repo, source.reference, source.path), limits: settings.catalogLimits);
  } on AosException catch (error) {
    return failed('It is not a catalog Canoa reads (${error.code})', tree.commit);
  }
  // Entries are in byte order of their names: those past the limit are left out, and counted.
  final kept = read.entries.keys.take(settings.catalogMaxEntries).toList();
  final catalog = AosCatalog(form: read.form, name: read.name, description: read.description, homepage: read.homepage,
      entries: {for (final name in kept) name: read.entries[name]!}, refused: read.refused, ignored: read.ignored);
  // Each icon file is read once, however many entries name it, and no more files than the limit.
  final byPath = <String, CatalogIcon>{};
  final icons = <String, CatalogIcon>{};
  final folder = _dirname(source.path);
  for (final entry in catalog.entries.values) {
    final icon = entry.about['icon'];
    if (icon is! String) continue;
    final path = folder.isEmpty ? icon : '$folder/$icon';
    if (byPath[path] case final known?) {
      icons[entry.name] = known;
      continue;
    }
    if (byPath.length >= settings.catalogMaxIcons) {
      icons[entry.name] = (bytes: null, refused: 'icon-limit');
      continue;
    }
    icons[entry.name] = byPath[path] = await _icon(tree, path, settings);
  }
  return LoadedCatalog(label: label, source: source, commit: tree.commit, catalog: catalog, icons: icons,
      leftOut: read.entries.length - kept.length);
}

Future<CatalogIcon> _icon(FetchedTree tree, String path, StoreSettings settings) async {
  try {
    final found = await tree.file(path, maxBytes: settings.iconMaxBytes);
    if (found == null) return (bytes: null, refused: 'icon-missing');
    final bytes = found.bytes;
    if (bytes == null) return (bytes: null, refused: 'icon-too-large');
    checkIcon(path, bytes, limits: settings.iconLimits);
    return (bytes: bytes, refused: null);
  } on AosException catch (error) {
    return (bytes: null, refused: error.code);
  }
}

/// [source] as a store shows it: without user information or query, which can hold a credential.
String displaySource(AosSource source) {
  var location = source.location;
  final scheme = location.indexOf('://');
  if (scheme >= 0) {
    final start = scheme + 3;
    var end = location.indexOf(RegExp(r'[/?#]'), start);
    if (end < 0) end = location.length;
    final at = location.lastIndexOf('@', end - 1);
    if (at >= start) location = location.substring(0, start) + location.substring(at + 1);
    final query = location.indexOf('?');
    if (query >= 0) location = location.substring(0, query);
  }
  return '$location${source.reference == null ? '' : '@${source.reference}'}'
      '${source.path == null ? '' : '#${source.path}'}';
}

/// The listing of every catalog read, searchable.
final class StoreListing {
  StoreListing(this.catalogs) : listing = listCatalogs([
    for (final loaded in catalogs)
      if (loaded.catalog case final catalog?) (loaded.label, catalog),
  ]);

  final List<LoadedCatalog> catalogs;
  final CatalogListing listing;

  /// The [count] tags the most entries carry, the most used first, then by name.
  List<String> topTags(int count) {
    final uses = <String, int>{};
    for (final listed in listing.entries) {
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
      {for (final listed in listing.entries) ..._tags(listed.entry)}.toList()..sort();

  LoadedCatalog catalogOf(ListedEntry listed) => catalogs.firstWhere((loaded) => loaded.label == listed.label);

  /// The entries whose shown name, summary, description, tags, catalog or source hold every word of [text], ignoring
  /// case, and that carry every tag of [tags].
  List<ListedEntry> search(String text, {Set<String> tags = const {}}) {
    final words = text.toLowerCase().split(RegExp(r'\s+')).where((word) => word.isNotEmpty).toList();
    return [
      for (final listed in listing.entries)
        if (tags.every(_tags(listed.entry).contains) && words.every(_haystack(listed).contains)) listed,
    ];
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
  final normalized = normalizeName(entry.name);
  if (isEntryName(normalized)) return normalized;
  throw StoreRefusal('Canoa names an overlay with 1 to 64 letters, digits, - or _, and ${entry.name} has no such form');
}

/// The git source Canoa installs [entry] from, held to what an overlay named by hand in Canoa.cfg is held to.
GitOverlay installSource(CatalogEntry entry) {
  final source = entry.source;
  if (source.kind != SourceKind.git) {
    throw const StoreRefusal('Canoa installs overlays from git only, and this source is not git');
  }
  final reference = source.reference;
  if (reference == null) {
    throw const StoreRefusal('It names no branch, tag or commit, and Canoa fetches only a named one');
  }
  if (holdsPassword(source.location)) {
    throw const StoreRefusal('Its source holds a password, which Canoa.cfg may not: it is not installed from here');
  }
  try {
    return OverlaySource.parseGit('${source.location}@$reference${source.path == null ? '' : '#${source.path}'}');
  } on ProfileException {
    throw const StoreRefusal('Canoa would refuse this source in Canoa.cfg: a transport or ref git must not take');
  }
}

/// How an entry stands against Canoa.cfg's `[overlays]`.
enum EntryState {
  /// No overlay of its identity is named.
  absent,

  /// An overlay of its identity is named from this very source.
  installed,

  /// An overlay of its identity is named from another source: the store neither installs over it nor removes it.
  taken,
}

/// [name] as `[overlays]` has it, when an overlay of the same AOS identity is named there.
String? namedAs(CanoaConfig config, String name) =>
    config.overlays.keys.where((key) => normalizeName(key) == normalizeName(name)).firstOrNull;

EntryState entryState(CanoaConfig config, String name, GitOverlay source) {
  final named = namedAs(config, name);
  if (named == null) return EntryState.absent;
  final present = config.overlays[named];
  return present is GitOverlay && present.repo == source.repo && present.reference == source.reference &&
          present.path == source.path
      ? EntryState.installed
      : EntryState.taken;
}

/// An overlay fetched, not installed, and what it would do, all from its own manifest and files.
final class OverlayPreview {
  const OverlayPreview({required this.update, required this.manifest, required this.about, required this.ignored,
      this.icon, this.iconRefused, this.layer, this.report, this.refusal, this.pinRefusal});

  /// What installing it would change; the same object the install applies.
  final OverlayUpdate update;

  /// Its `overlay.cfg` as read, or empty when it has none or it cannot be read.
  final Map<String, Object?> manifest;

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
/// its weakenings and what Canoa does not apply. A catalog's [catalogPin] must match the tree fetched, or installing is
/// refused; Canoa.cfg's own pin is held by [installPreview].
Future<OverlayPreview> previewOverlay(OverlayManager overlays, String name, GitOverlay source, {
  required String os,
  required Map<String, LayerId> shared,
  String? catalogPin,
  String? harness,
  IconLimits iconLimits = const IconLimits(),
}) async {
  final update = await overlays.check(name, source.pinnedTo(null));
  final files = await overlays.read(update);
  final tree = [for (final MapEntry(:key, :value) in files.entries) AosTreeEntry.file(key, value)];
  if (!sameDigest(digestEntries(tree).digest, update.digest)) {
    throw const StoreRefusal('The files read do not have the digest fetched');
  }
  String? pinRefusal;
  if (catalogPin != null && !sameDigest(catalogPin, update.digest)) {
    pinRefusal = 'The catalog pins $catalogPin, and what was fetched is ${update.digest}: refused';
  }
  Map<String, Object?> manifest = const {};
  if (files['overlay.cfg'] case final bytes?) {
    try {
      manifest = readDocument(bytes);
    } on AosException {
      manifest = const {};
    }
  }
  final ignored = <String>[];
  final about = manifest.containsKey('about') ? storeAbout(manifest['about'], '/about', ignored) ?? const {} : const <String, Object>{};
  List<int>? icon;
  String? iconRefused;
  if (about['icon'] case final String path) {
    try {
      final bytes = files[path] ?? (throw const AosException('icon-missing'));
      checkIcon(path, bytes, limits: iconLimits);
      icon = bytes;
    } on AosException catch (error) {
      iconRefused = error.code;
    }
  }
  Layer? layer;
  OverlayReport? report;
  String? refusal;
  try {
    layer = overlayLayer(identity: name, root: overlays.directoryOf(name), os: os, tree: tree, harness: harness,
        pinned: true, iconLimits: iconLimits);
    report = overlayReport(layer.id, layer.aos!, shared);
  } on OverlayRefusal catch (error) {
    refusal = '$error';
  }
  return OverlayPreview(update: update, manifest: manifest, about: about, ignored: ignored, icon: icon,
      iconRefused: iconRefused, layer: layer, report: report, refusal: refusal, pinRefusal: pinRefusal);
}

/// The pin Canoa.cfg gives the overlay [name], when it names one, and the tree [preview] fetched has another digest:
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
/// commit, checked against its tree digest as written, then the overlay named in Canoa.cfg pinned to that digest. A
/// pin Canoa.cfg already gives it holds as it does for a sync from Settings, Profiles: a tree of another digest is
/// installed only with [movePin], the person's choice to move it.
Future<CanoaConfig> installPreview(OverlayManager overlays, CanoaConfig config, OverlayPreview preview,
    {bool movePin = false}) async {
  if (preview.pinRefusal case final why?) throw StoreRefusal(why);
  if (preview.refusal case final why?) throw StoreRefusal('Canoa would leave it out: $why');
  final name = preview.name;
  final named = namedAs(config, name);
  if (named != null && entryState(config, name, preview.update.source) != EntryState.installed) {
    throw StoreRefusal('Canoa.cfg names $named from another source: remove it there first');
  }
  if (pinMoved(config, preview) case final pin? when !movePin) {
    throw StoreRefusal('Canoa.cfg pins $pin, and what was fetched is ${preview.update.digest}: nothing was installed');
  }
  final record = await overlays.apply(preview.update);
  return config.withOverlay(named ?? name, preview.update.source.pinnedTo(null), record.digest!);
}

/// Removes the overlay [name] the store installed from [source]: its files and fetched history, and its entry in
/// `[overlays]`. One named from another source is refused, never removed.
Future<CanoaConfig> removeInstalled(OverlayManager overlays, CanoaConfig config, String name, GitOverlay source) async {
  final named = namedAs(config, name);
  if (named == null) return config;
  if (entryState(config, name, source) != EntryState.installed) {
    throw StoreRefusal('Canoa.cfg names $named from another source: the store does not remove it');
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
