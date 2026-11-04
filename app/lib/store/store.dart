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
      this.icons = const {}});

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
  final AosCatalog catalog;
  try {
    catalog = readCatalog(source.path, bytes,
        base: AosSource(SourceKind.git, source.repo, source.reference, source.path), limits: settings.catalogLimits);
  } on AosException catch (error) {
    return failed('It is not a catalog Canoa reads (${error.code})', tree.commit);
  }
  final icons = <String, CatalogIcon>{};
  final folder = _dirname(source.path);
  for (final entry in catalog.entries.values) {
    final icon = entry.about['icon'];
    if (icon is! String) continue;
    final path = folder.isEmpty ? icon : '$folder/$icon';
    try {
      final found = await tree.file(path, maxBytes: settings.iconMaxBytes);
      if (found == null) {
        icons[entry.name] = (bytes: null, refused: 'icon-missing');
      } else if (found.bytes == null) {
        icons[entry.name] = (bytes: null, refused: 'icon-too-large');
      } else {
        checkIcon(path, found.bytes!, limits: settings.iconLimits);
        icons[entry.name] = (bytes: found.bytes, refused: null);
      }
    } on AosException catch (error) {
      icons[entry.name] = (bytes: null, refused: error.code);
    }
  }
  return LoadedCatalog(label: label, source: source, commit: tree.commit, catalog: catalog, icons: icons);
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

  /// Every tag the listed entries carry, sorted.
  List<String> get tags =>
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
    for (final tag in (entry.about['tags'] as List?) ?? const []) '$tag',
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

  List<String> get requires => [for (final item in (manifest['requires'] as List?) ?? const []) '$item'];

  /// Each setup the manifest declares: its path, the OSes it is for, and whether it says it needs the network.
  List<(String, List<String>?, bool)> get setup => [
    for (final entry in (manifest['setup'] as List?) ?? const [])
      if (entry is Map)
        ('${entry['path'] ?? ''}', entry['os'] is List ? [for (final os in entry['os'] as List) '$os'] : null,
            entry['network'] == true),
  ];
}

/// Fetches [source] as [name] and computes what installing it would do, touching nothing installed: the manifest and
/// files are read from the fetched tree, the layer is the one Settings, Profiles would make of it, and the report is
/// its weakenings and what Canoa does not apply. A catalog's [catalogPin], and [configPin] when Canoa.cfg pins the same
/// source, must each match the tree fetched, or installing is refused.
Future<OverlayPreview> previewOverlay(OverlayManager overlays, String name, GitOverlay source, {
  required String os,
  required Map<String, LayerId> shared,
  String? catalogPin,
  String? configPin,
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
  } else if (configPin != null && !sameDigest(configPin, update.digest)) {
    pinRefusal = 'Canoa.cfg pins $configPin, and what was fetched is ${update.digest}';
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

/// Installs or updates what [preview] showed, through the same path an overlay named by hand takes: the files at its
/// commit, checked against its tree digest as written, then the overlay named in Canoa.cfg pinned to that digest.
Future<CanoaConfig> installPreview(OverlayManager overlays, CanoaConfig config, OverlayPreview preview) async {
  if (preview.pinRefusal case final why?) throw StoreRefusal(why);
  if (preview.refusal case final why?) throw StoreRefusal('Canoa would leave it out: $why');
  final name = preview.name;
  final named = namedAs(config, name);
  if (named != null && entryState(config, name, preview.update.source) != EntryState.installed) {
    throw StoreRefusal('Canoa.cfg names $named from another source: remove it there first');
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

/// A tag line for an overlay's `[about]` tags, as plain words.
String tagLine(Map<String, Object> about) => [for (final tag in (about['tags'] as List?) ?? const []) '$tag'].join(', ');

/// [bytes] as text for showing, never interpreted.
String plainText(List<int> bytes) => utf8.decode(bytes, allowMalformed: true);
