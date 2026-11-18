/// What the Loadout page holds between its three sections: the configuration, the catalogs read, the skills fetched, the layers
/// each installed overlay makes, and what a registry listed. Every change to canoa.cfg goes through [save], so the page shows
/// what the file now says.
library;

import 'dart:async';
import 'dart:convert';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart';
import '../profile/overlay_layer.dart';
import '../profile/profile.dart';
import '../profile/overlays.dart' show FetchedTree, OverlayException, ScannedSkill, SkillHit, SkillScan;
import '../profile/profile_home.dart';
import '../profile/profile_layers.dart';
import '../profile/skill_copies.dart';
import '../store/contents_reader.dart';
import '../store/contents_storage.dart';
import '../store/contents_store.dart';
import '../store/overlay_needs.dart';
import '../store/overlay_page.dart' show SaveConfig;
import '../config/catalog_specs.dart';
import '../store/catalog_types.dart';
import '../store/skill_sources.dart';
import '../store/skills.dart';
import '../store/store.dart';
import '../store/store_index.dart';
import 'mcp_registry.dart';

/// A program's absolute path as the machine's PATH finds it, or null.
typedef FindProgram = Future<String?> Function(String program);

/// A catalog worth offering to someone who has none: a label, a line about it and the source to add it from. The data of the
/// offer, so adding to it is a line here and no logic.
typedef KnownCatalog = ({String label, String about, String spec});

/// The catalogs the Store offers to start from: Boriken Dev's overlays alone.
const knownCatalogs = <KnownCatalog>[
  (
    label: defaultCatalogLabel,
    about: 'Boriken Dev\'s overlays: the engineering, language, network and release rules and tools.',
    spec: defaultCatalogSpec,
  ),
];

/// The label a catalog added from [spec] gets when the person gives none: its repository's name.
String catalogLabelFor(String spec) {
  final base = spec.split('@').first.split('#').first.split('/').where((part) => part.isNotEmpty).lastOrNull ?? 'catalog';
  var label = base.replaceAll(RegExp(r'\.git$'), '').replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '-');
  if (label.isEmpty || !RegExp(r'^[A-Za-z]').hasMatch(label)) label = 'catalog-$label';
  return label.length > 64 ? label.substring(0, 64) : label;
}

/// An MCP server the profile defines, with where it came from.
final class InstalledMcp {
  const InstalledMcp({required this.name, required this.server, required this.layer, this.origin});
  final String name;
  final McpServer server;

  /// `yours` for the person's own profile, or `overlay:NAME`.
  final String layer;

  /// Where the store got it, when it did.
  final McpOrigin? origin;

  bool get fromOverlay => layer.startsWith('overlay:');
}

class LoadoutController extends ChangeNotifier {
  LoadoutController({
    required CanoaConfig config,
    required this.home,
    required this.save,
    this.agents = const [],
    this.registry = officialRegistry,
    this.fetch,
    this.findProgram,
    this.overlayNeeds,
    this.index,
    this.catalogTypes,
    ContentsStorage? contentsStorage,
    ContentsReader? contentsReader,
    DateTime Function()? now,
    bool readAhead = true,
  }) : _config = config {
    final home = this.home;
    if (home != null && index != null && readAhead && contentsReader != null) {
      contents = ContentsStore(reader: contentsReader, storage: contentsStorage, now: now)..addListener(notifyListeners);
    }
  }

  final ProfileHome? home;
  final SaveConfig save;

  /// The host's catalog index, which every catalog the store lists is read from; none where the app has no host.
  final CatalogIndex? index;

  /// Asks the host which Store types read catalogs, what each calls them, which it lists and which sources can add one; none where
  /// the app has no host.
  final CatalogTypesRead? catalogTypes;

  /// What the host last said of each type that reads catalogs.
  List<CatalogType> types = const [];

  /// What the host says of the type [id], or null before it has said or for a type that reads no catalogs.
  CatalogType? typeOf(String id) => types.where((type) => type.id == id).firstOrNull;

  /// The agents the app offers, which an MCP server is allowed or turned off for.
  final List<String> agents;
  /// The official registry, which is always offered first.
  final String registry;
  final RegistryFetch? fetch;
  final FindProgram? findProgram;

  /// Asks the host what an installed overlay needs of this computer; none where the app has no host.
  final OverlayNeeds? overlayNeeds;

  /// What each installed overlay needs and whether this computer has it, by overlay name, as of the last refresh.
  Map<String, List<NeedLine>> needs = const {};

  /// What every entry of the catalogs provides, read ahead whenever the catalogs are read; none without a host to read them.
  ContentsStore? contents;

  /// The reading ahead in progress or last finished, which a refresh starts.
  Future<void> readingAhead = Future.value();

  CanoaConfig _config;
  CanoaConfig get config => _config;

  List<LoadedCatalog> loaded = const [];
  StoreListing? listing;
  bool loadingCatalogs = false;

  /// The skills fetched and accepted, by lower-case name.
  Map<String, SkillCopy> skills = const {};

  /// Each installed overlay's layer and why one is missing.
  ProfileLayers? layers;

  /// The registry the Discover list reads: [registry] or one the person added.
  late String selectedRegistry = registry;

  /// The registries offered: the official one, then those `[store.mcps.catalogs]` names.
  List<String> get registries => [registry, ...config.storeRegistries.where((address) => address != registry)];

  List<McpRegistryEntry> registryEntries = const [];
  String? registryNext, registryProblem;
  bool loadingRegistry = false, registryRead = false;
  String _registryQuery = '';

  String? busy, problem;
  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    contents?.dispose();
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads everything the page shows: the catalogs from the host's index, the skills here and each overlay's layer.
  /// [again] makes the host ask its sources before answering.
  Future<void> load({bool again = false}) async {
    final home = this.home;
    final index = this.index;
    if (home == null || index == null) return;
    loadingCatalogs = true;
    notifyListeners();
    var found = const <LoadedCatalog>[];
    try {
      found = await index(refresh: again);
      problem = null;
    } on Object catch (error) {
      // The host not being there, or refusing, leaves the catalogs empty and says why; it never takes the page down.
      problem = 'The catalogs could not be read: $error';
    }
    loaded = found;
    listing = StoreListing(found);
    await loadTypes();
    loadingCatalogs = false;
    // Discovery reads what every entry provides, so that opening one asks for nothing; a refresh the person asked for reads it
    // all again.
    if (contents case final store?) {
      readingAhead = store.readAhead(found, ttl: Duration(seconds: config.store.contentsTtlSeconds), force: again);
    }
    await refresh();
  }

  /// Reads what the host says of the types that read catalogs. A host that does not answer leaves what was known.
  Future<void> loadTypes() async {
    final read = catalogTypes;
    if (read == null) return;
    try {
      types = await read();
    } on Object {
      // The catalogs themselves say why they could not be read; the forms stay as they were.
    }
    notifyListeners();
  }

  /// Adds the catalog [label] of the Store type [type], read by [provider] from [fields]. The type's own reader checks a source the
  /// `read` provider reads; any other provider's fields are checked against what its form needs. Gives why it cannot be added, or
  /// null. The label is optional: a catalog gets one from where it reads.
  Future<String?> addCatalogOf(String type, String label, String provider, Map<String, Object?> fields) async {
    final source = provider == readProvider ? '${fields['source'] ?? ''}'.trim() : null;
    var name = label.trim();
    if (name.isEmpty) name = source != null && source.isNotEmpty ? catalogLabelFor(source) : provider;
    if (provider == readProvider) {
      if (source == null || source.isEmpty) return 'Name a source';
      switch (type) {
        case 'skills':
          return addSkillSource(source);
        case 'mcps':
          return addRegistry(source);
      }
    }
    final taken = config.catalogSpecs(type).keys.toList();
    if (type == 'overlays') taken.addAll(config.overlayCatalogs.keys);
    if (taken.any((key) => aos.normalizeName(key) == aos.normalizeName(name))) return 'Another catalog has the label $name';
    final CanoaConfig changed;
    try {
      changed = type == 'overlays' && provider == readProvider
          ? config.withCatalog(name, source!)
          : config.withCatalogOf(type, name, CatalogSpec(provider, fields));
    } on FormatException catch (error) {
      return error.message.replaceFirst('canoa.cfg: ', '');
    }
    await apply(changed);
    await load();
    return null;
  }

  /// Takes the catalog [label] of the Store type [type] away. What the type's default is, the host said.
  Future<void> removeCatalogOf(String type, String label) async {
    switch (type) {
      case 'overlays':
        return removeCatalog(label);
      case 'skills':
        return removeSkillSource(label);
      case 'mcps':
        final address = config.storeRegistryLabels[label];
        if (address != null) return removeRegistry(address);
    }
    await apply(config.withoutCatalogOf(type, label, fallback: typeOf(type)?.defaultLabel));
    await load();
  }

  /// Reads what changes without a fetch: the skills here and the layers.
  Future<void> refresh() async {
    final home = this.home;
    if (home != null) {
      // The skills and layers are the host's to read; a host that is down or serves no control channel shows none, and says why.
      try {
        skills = await home.overlays.skillCopies();
        layers = await profileLayers(
          overlays: config.overlays,
          user: config.profileWithChosenSkills,
          agents: config.agentProfiles,
          agent: agents.firstOrNull ?? '',
          home: home,
          iconLimits: config.store.iconLimits,
          skillPins: config.skillPins,
        );
        needs = await _readNeeds();
      } on Object catch (error) {
        skills = const {};
        layers = null;
        needs = const {};
        problem = 'The host could not be asked for the skills and layers here: $error';
      }
    }
    notifyListeners();
  }

  /// What each installed overlay that made a layer needs, asked of the host all at once.
  Future<Map<String, List<NeedLine>>> _readNeeds() async {
    final ask = overlayNeeds;
    if (ask == null) return const {};
    final names = [for (final name in config.overlays.keys) if (layerOf(name) != null) name];
    final found = await Future.wait([for (final name in names) ask(name)]);
    return {for (var i = 0; i < names.length; i++) names[i]: found[i]};
  }

  /// Saves [changed], and gives the configuration now in the file, or null when it was not saved.
  Future<CanoaConfig?> apply(CanoaConfig changed) async {
    final saved = await save(changed);
    if (saved != null) {
      final storeChanged = '${_config.tables['store']}' != '${saved.tables['store']}';
      _config = saved;
      await refresh();
      // What the host lists under each type follows `[store]`.
      if (storeChanged) await loadTypes();
    }
    return saved;
  }

  /// Takes a configuration that something else saved, such as an overlay's own page.
  Future<void> adopt(CanoaConfig changed) async {
    _config = changed;
    await refresh();
  }

  Future<T?> run<T>(String what, Future<T> Function() action) async {
    busy = what;
    problem = null;
    notifyListeners();
    try {
      return await action();
    } on Object catch (error) {
      problem = '$what: $error';
      return null;
    } finally {
      busy = null;
      notifyListeners();
    }
  }

  // Overlays.

  /// Whether new sessions start with the overlay [name].
  bool isDefaultOn(String name) => config.startingLoadout.overlays.contains(name);

  /// Prepares the installed overlay [name] on the host: its dependencies and setup, as the plan the person said yes to listed.
  Future<void> prepareOverlay(String name) async {
    final home = this.home;
    if (home == null) return;
    await run('Preparing $name', () => home.overlays.prepare(name));
    await refresh();
  }

  Future<void> setDefault(String name, bool on) {
    return apply(config.withStartingOverlay(name, on));
  }

  /// The layer an installed overlay makes, null when it was left out.
  Layer? layerOf(String name) =>
      layers?.layers.where((layer) => layer.id.kind == LayerKind.overlay && layer.id.name == name).firstOrNull;

  String? leftOutBecause(String name) =>
      layers?.notes.where((note) => note.layer.kind == LayerKind.overlay && note.layer.name == name).firstOrNull?.text;

  OverlayReport? reportOf(String name) => layers?.effective.overlays.where((report) => report.layer.name == name).firstOrNull;

  Future<void> addCatalog(String label, String spec) async {
    final name = label.trim().isEmpty ? catalogLabelFor(spec.trim()) : label.trim();
    if (config.overlayCatalogs.keys.any((key) => aos.normalizeName(key) == aos.normalizeName(name))) {
      problem = 'Another catalog has the label $name';
      notifyListeners();
      return;
    }
    final CanoaConfig changed;
    try {
      changed = config.withCatalog(name, spec.trim());
    } on FormatException catch (error) {
      problem = error.message;
      notifyListeners();
      return;
    }
    problem = null;
    await apply(changed);
    await load();
  }

  Future<void> removeCatalog(String label) async {
    await home?.overlays.removeCatalog(label);
    await apply(config.withoutCatalog(label));
    await load();
  }

  // Skills.

  /// Whether new sessions start with the skill [name] the person chose.
  bool skillIsOn(String name) => config.loadoutSkills.contains(name);

  CanoaConfig _withSkillOn(CanoaConfig config, String name, bool on) =>
      config.withLoadoutSkills({...config.loadoutSkills}..remove(name)..addAll(on ? [name] : const []));

  /// The skills the catalogs offer that are not here, as `(catalog label, name, entry)`.
  List<(String, String, aos.CatalogEntry)> skillsOffered() => [
    for (final catalog in loaded)
      if (catalog.catalog case final read?)
        for (final MapEntry(key: name, value: offer) in read.skills.entries)
          if (!skills.containsKey(skillKey(name))) (catalog.label, name, offer),
  ];

  Future<void> setSkillOn(String name, bool on) => apply(_withSkillOn(config, name, on));

  /// Where the Skills page looks for skills besides the catalogs, by label: the default, then those the person added.
  Map<String, SkillSource> get skillSources => config.skillSources;

  /// What each skill source held when it was last read, by label. Reading one fetches its repository, so it is the person's
  /// own action and never happens by itself.
  final skillScans = <String, SkillScan>{};

  /// Why the last read of a source failed, by label, and the label being read now.
  final skillScanProblems = <String, String>{};
  String? scanningSource;

  /// Whether [label]'s source was typed by the person, which is then fetched with their own git settings, credentials and keys.
  /// The default one is Canoa's offer, and any source it fetches gets none of them.
  bool skillSourceIsOwn(String label) {
    final source = skillSources[label];
    return source != null && !(aos.normalizeName(label) == aos.normalizeName(defaultSkillSourceLabel) && source.spec == defaultSkillSourceSpec);
  }

  /// Reads which skills [label]'s source holds, at the commit its ref names now.
  Future<void> scanSkillSource(String label) async {
    final home = this.home;
    final source = skillSources[label];
    if (home == null || source == null || scanningSource != null) return;
    scanningSource = label;
    skillScanProblems.remove(label);
    notifyListeners();
    try {
      skillScans[label] = await home.overlays.scanSkills(source.repo, ref: source.ref, path: source.path, own: skillSourceIsOwn(label));
    } on OverlayException catch (error) {
      skillScans.remove(label);
      skillScanProblems[label] = error.message;
    } on Object catch (error) {
      skillScans.remove(label);
      skillScanProblems[label] = 'It could not be read (${error.runtimeType})';
    } finally {
      scanningSource = null;
      notifyListeners();
    }
  }

  /// Keeps [text] as a skill source from now on. Gives why it cannot be one, or null. The source is not fetched.
  Future<String?> addSkillSource(String text) async {
    final SkillSource source;
    try {
      source = SkillSource.parse(text);
    } on FormatException catch (error) {
      return error.message;
    }
    if (skillSources.values.any((each) => each.spec == source.spec)) return 'That source is already here';
    var label = source.label;
    for (var n = 2; skillSources.keys.any((key) => aos.normalizeName(key) == aos.normalizeName(label)); n++) {
      final base = source.label;
      label = '${base.length > 60 ? base.substring(0, 60) : base}-$n';
    }
    final CanoaConfig changed;
    try {
      changed = config.withSkillSource(label, source.spec);
    } on FormatException catch (error) {
      return error.message;
    }
    await apply(changed);
    return null;
  }

  /// Forgets the skill source [label], what was read from it and the reason it failed. The skills installed from it stay.
  Future<void> removeSkillSource(String label) async {
    if (!skillSources.containsKey(label)) return;
    await apply(config.withoutSkillSource(label));
    skillScans.remove(label);
    skillScanProblems.remove(label);
    notifyListeners();
  }

  /// The skills directory the Skills page searches, or null while the search is off, which it is until the person turns it on.
  String? get skillsSearch => config.skillsSearch;

  /// The last search's results, the text it was for, why it failed, and whether one is running; and the result being looked up.
  List<SkillHit> skillHits = const [];
  String? skillSearchedFor, skillSearchProblem, resolvingHit;
  bool searchingSkills = false;

  /// Turns the skills search on or off in canoa.cfg. Off forgets what was found.
  Future<void> setSkillsSearch(bool on) async {
    await apply(config.withSkillsSearch(on));
    if (!on) {
      skillHits = const [];
      skillSearchedFor = null;
      skillSearchProblem = null;
    }
    notifyListeners();
  }

  /// Asks the directory for the skills that match [text], through the host. Only what the person typed leaves this computer.
  Future<void> searchSkills(String text) async {
    final home = this.home;
    final query = text.trim();
    if (home == null || searchingSkills || skillsSearch == null) return;
    skillSearchProblem = null;
    if (query.runes.length < 2) {
      skillSearchProblem = 'Type at least two characters to search';
      notifyListeners();
      return;
    }
    searchingSkills = true;
    notifyListeners();
    try {
      skillHits = await home.overlays.searchSkills(query);
      skillSearchedFor = query;
    } on OverlayException catch (error) {
      skillHits = const [];
      skillSearchedFor = null;
      skillSearchProblem = error.message;
    } on Object catch (error) {
      skillHits = const [];
      skillSearchedFor = null;
      skillSearchProblem = 'The search could not be made (${error.runtimeType})';
    } finally {
      searchingSkills = false;
      notifyListeners();
    }
  }

  /// Finds [hit] in its repository, the way any source is read: the repository is fetched with none of the person's git settings,
  /// credentials or keys, and its `SKILL.md` files say which skill is meant. What the directory sent is only where to look. Null,
  /// with [skillSearchProblem] saying why, when the repository cannot be read or holds no such skill.
  Future<({SkillSource source, SkillScan scan, ScannedSkill skill})?> resolveSkillHit(SkillHit hit) async {
    final home = this.home;
    if (home == null || resolvingHit != null) return null;
    resolvingHit = hit.id;
    skillSearchProblem = null;
    notifyListeners();
    try {
      final source = SkillSource.parse(hit.source);
      final scan = await home.overlays.scanSkills(source.repo, own: false);
      final wanted = hit.skillId.toLowerCase();
      final skill = scan.skills.where((each) => each.name.toLowerCase() == wanted).firstOrNull ??
          scan.skills.where((each) => each.path.split('/').last.toLowerCase() == wanted).firstOrNull;
      if (skill == null) {
        skillSearchProblem = '${hit.source} holds no skill called ${hit.skillId}: the directory lists it, but the repository has no SKILL.md for it';
        return null;
      }
      return (source: source, scan: scan, skill: skill);
    } on OverlayException catch (error) {
      skillSearchProblem = error.message;
      return null;
    } on Object catch (error) {
      skillSearchProblem = 'The repository could not be read (${error.runtimeType})';
      return null;
    } finally {
      resolvingHit = null;
      notifyListeners();
    }
  }

  /// Fetches the skill [item] reviewed, as the person's own choice, and turns it on for new sessions.
  Future<SkillOutcome?> installSkill(SkillItem item) => run('Installing ${item.name}', () async {
    final outcome = await installChosenSkill(home!.overlays, item);
    if (outcome.result == SkillResult.installed || outcome.result == SkillResult.kept) {
      await apply(_withSkillOn(config, item.name, true));
    } else {
      problem = '${item.name}: ${outcome.message}';
    }
    await refresh();
    return outcome;
  });

  Future<void> removeSkill(SkillCopy copy) => run('Removing ${copy.name}', () async {
    if (config.loadoutSkills.contains(copy.name)) await apply(_withSkillOn(config, copy.name, false));
    if (copy.chosen) {
      await releaseChosenSkill(home!.overlays, copy.name);
    } else {
      await removeOrphans(home!.overlays, [copy.name]);
    }
    await refresh();
  });

  // MCP servers.

  /// Every MCP server the profile defines: the person's own, then each overlay's.
  List<InstalledMcp> installedMcp() => [
    for (final MapEntry(:key, :value) in config.profile.mcp.entries)
      if (value != null) InstalledMcp(name: key, server: value, layer: 'yours', origin: config.storeMcp[key]),
    for (final layer in layers?.layers ?? const <Layer>[])
      if (layer.id.kind == LayerKind.overlay)
        for (final MapEntry(:key, :value) in layer.profile.mcp.entries)
          if (value != null) InstalledMcp(name: key, server: value, layer: '${layer.id}'),
  ];

  /// How many of [agents] may start the person's own server [name]: one a choice names `false` is off for it.
  int allowedFor(String name) => agents.where((agent) => config.agentMcp[agent]?[name] != false).length;

  Future<void> setMcpAllowed(String name, bool on) {
    var changed = config;
    for (final agent in agents) {
      changed = changed.withAgentMcp(agent, {...?changed.agentMcp[agent], name: on});
    }
    return apply(changed.withStartingMcp(name, on));
  }

  Future<void> removeMcp(String name) => apply(config.withoutMcp(name));

  /// Installs [entry]'s [option] as the server [name]: written to the profile and off for every agent until allowed.
  Future<CanoaConfig?> installMcp(McpRegistryEntry entry, McpOption option, String name) {
    final definition = option.definition;
    if (definition == null || option.refusal != null) throw StateError('That option is refused: ${option.refusal}');
    return apply(config.withInstalledMcp(name, definition, entry.origin(selectedRegistry), agents: agents));
  }

  /// Reads [address] from now on, after adding it to `[store.mcps.catalogs]`. Gives why it cannot be one, or null.
  Future<String?> addRegistry(String address) async {
    address = address.trim();
    if (registryProblemOf(address) case final why?) return why;
    if (registries.contains(address)) return 'That registry is already here';
    if (registries.length >= 17) return 'Canoa keeps at most 16 registries beside the official one';
    await apply(config.withStoreRegistries([...config.storeRegistries, address]));
    await selectRegistry(address);
    return null;
  }

  /// Forgets a registry the person added; the official one stays.
  Future<void> removeRegistry(String address) async {
    if (!config.storeRegistries.contains(address)) return;
    await apply(config.withStoreRegistries([...config.storeRegistries]..remove(address)));
    if (isGitRegistry(address)) await home?.overlays.removeCatalog(gitRegistryLabel(address));
    if (selectedRegistry == address) await selectRegistry(registry);
  }

  /// Lists [address]'s servers in Discover.
  Future<void> selectRegistry(String address) {
    selectedRegistry = address;
    return loadRegistry();
  }

  /// A git registry's one page: the list file at the ref's commit.
  Future<McpRegistryPage> _readGitRegistry(String address) async {
    final home = this.home;
    if (home == null) throw const RegistryRefusal('A git registry is read through the host, and this app has none');
    final source = gitRegistrySource(address);
    final FetchedTree tree;
    try {
      tree = await home.overlays.fetchCatalog(gitRegistryLabel(address), source);
    } on OverlayException catch (error) {
      throw RegistryRefusal('It cannot be fetched: $error');
    }
    final file = await tree.file(source.path, maxBytes: maxRegistryBytes);
    if (file == null) throw RegistryRefusal('It has no file ${source.path} at that commit');
    final bytes = file.bytes;
    if (bytes == null) throw const RegistryRefusal("The registry's file is larger than Canoa reads");
    return parseRegistryPage(utf8.decode(bytes, allowMalformed: true));
  }

  /// Reads the registry's first page for [search], or the next page of the last search.
  Future<void> loadRegistry({String? search, bool more = false}) async {
    final read = fetch;
    final git = isGitRegistry(selectedRegistry);
    if (read == null && !git) {
      registryProblem = 'Registries are read through the host, and this app has none';
      registryRead = true;
      notifyListeners();
      return;
    }
    if (!more) {
      _registryQuery = search ?? '';
      registryEntries = const [];
      registryNext = null;
    }
    loadingRegistry = true;
    registryProblem = null;
    notifyListeners();
    try {
      final page = git
          ? await _readGitRegistry(selectedRegistry)
          : parseRegistryPage(await read!(listUri(selectedRegistry, search: _registryQuery, cursor: more ? registryNext : null)));
      // A search the registry does not take is narrowed here, so a registry that ignores `search` still answers it.
      final words = _registryQuery.toLowerCase().split(RegExp(r'\s+')).where((word) => word.isNotEmpty);
      registryEntries = [
        ...registryEntries,
        for (final entry in page.entries)
          if (words.every('${entry.name}\n${entry.title}\n${entry.description}'.toLowerCase().contains)) entry,
      ];
      registryNext = page.next;
    } on RegistryRefusal catch (error) {
      registryProblem = error.message;
    } on Object catch (error) {
      registryProblem = 'The registry cannot be read (${error.runtimeType})';
    } finally {
      loadingRegistry = false;
      registryRead = true;
      notifyListeners();
    }
  }
}
