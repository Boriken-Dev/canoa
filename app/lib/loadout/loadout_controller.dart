/// What the Loadout page holds between its three sections: the configuration, the catalogs read, the skills fetched, the layers
/// each installed overlay makes, and what a registry listed. Every change to canoa.cfg goes through [save], so the page shows
/// what the file now says.
library;

import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart';
import '../profile/overlay_layer.dart';
import '../profile/profile.dart';
import '../profile/profile_home.dart';
import '../profile/profile_layers.dart';
import '../profile/skill_copies.dart';
import '../store/overlay_page.dart' show SaveConfig;
import '../store/skills.dart';
import '../store/store.dart';
import 'mcp_registry.dart';

/// A program's absolute path as the machine's PATH finds it, or null.
typedef FindProgram = Future<String?> Function(String program);

/// A catalog worth offering to someone who has none: a label, a line about it and the source to add it from. The data of the
/// offer, so adding to it is a line here and no logic.
typedef KnownCatalog = ({String label, String about, String spec});

/// Where the sample catalogs of the Agent Overlay Specification live.
const knownCatalogs = <KnownCatalog>[
  (
    label: 'aos-examples',
    about: 'The specification\'s own example catalog: small overlays that show what a catalog lists.',
    spec: 'https://github.com/Boriken-Dev/agent-overlay-spec.git@main#examples/registry/catalog.toml',
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
  }) : _config = config;

  final ProfileHome? home;
  final SaveConfig save;

  /// The agents the app offers, which an MCP server is allowed or turned off for.
  final List<String> agents;
  final String registry;
  final RegistryFetch? fetch;
  final FindProgram? findProgram;

  CanoaConfig _config;
  CanoaConfig get config => _config;

  List<LoadedCatalog> loaded = const [];
  StoreListing? listing;
  bool loadingCatalogs = false;

  /// The skills fetched and accepted, by lower-case name.
  Map<String, SkillCopy> skills = const {};

  /// Each installed overlay's layer and why one is missing.
  ProfileLayers? layers;

  List<McpRegistryEntry> registryEntries = const [];
  String? registryNext, registryProblem;
  bool loadingRegistry = false, registryRead = false;
  String _registryQuery = '';

  String? busy, problem;
  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads everything the page shows: the catalogs, the skills here and each overlay's layer.
  Future<void> load() async {
    final home = this.home;
    if (home == null) return;
    loadingCatalogs = true;
    notifyListeners();
    final found = <LoadedCatalog>[];
    for (final MapEntry(key: name, value: from) in config.catalogs.entries) {
      found.add(await loadCatalog(home.overlays, name, from, config.store));
    }
    loaded = found;
    listing = StoreListing(found);
    loadingCatalogs = false;
    await refresh();
  }

  /// Reads what changes without a fetch: the skills here and the layers.
  Future<void> refresh() async {
    final home = this.home;
    if (home != null) {
      skills = await home.overlays.skillCopies();
      layers = await profileLayers(
        overlays: config.overlays,
        user: config.profile,
        agents: config.agentProfiles,
        agent: agents.firstOrNull ?? '',
        home: home,
        iconLimits: config.store.iconLimits,
        skillPins: config.skillPins,
      );
    }
    notifyListeners();
  }

  /// Saves [changed], and gives the configuration now in the file, or null when it was not saved.
  Future<CanoaConfig?> apply(CanoaConfig changed) async {
    final saved = await save(changed);
    if (saved != null) {
      _config = saved;
      await refresh();
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
  bool isDefaultOn(String name) => !config.loadoutOff.contains(name);

  Future<void> setDefault(String name, bool on) {
    final off = {...config.loadoutOff};
    on ? off.remove(name) : off.add(name);
    return apply(config.withLoadoutOff(off));
  }

  /// The layer an installed overlay makes, null when it was left out.
  Layer? layerOf(String name) =>
      layers?.layers.where((layer) => layer.id.kind == LayerKind.overlay && layer.id.name == name).firstOrNull;

  String? leftOutBecause(String name) =>
      layers?.notes.where((note) => note.layer.kind == LayerKind.overlay && note.layer.name == name).firstOrNull?.text;

  OverlayReport? reportOf(String name) => layers?.effective.overlays.where((report) => report.layer.name == name).firstOrNull;

  Future<void> addCatalog(String label, String spec) async {
    final name = label.trim().isEmpty ? catalogLabelFor(spec.trim()) : label.trim();
    if (config.catalogs.keys.any((key) => aos.normalizeName(key) == aos.normalizeName(name))) {
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

  /// The path a chosen skill's profile entry takes, relative to canoa.cfg's directory.
  static String skillPath(String name) => 'overlays/$skillsDirectory/${skillKey(name)}';

  /// Whether new sessions start with the skill [name] the person chose.
  bool skillIsOn(String name) => config.profile.skills[name]?.path == skillPath(name);

  /// The skills the catalogs offer that are not here, as `(catalog label, name, entry)`.
  List<(String, String, aos.CatalogEntry)> skillsOffered() => [
    for (final catalog in loaded)
      if (catalog.catalog case final read?)
        for (final MapEntry(key: name, value: offer) in read.skills.entries)
          if (!skills.containsKey(skillKey(name))) (catalog.label, name, offer),
  ];

  Future<void> setSkillOn(String name, bool on) => apply(config.withProfileSkill(name, on ? skillPath(name) : null));

  /// Fetches the skill [item] reviewed, as the person's own choice, and turns it on for new sessions.
  Future<SkillOutcome?> installSkill(SkillItem item) => run('Installing ${item.name}', () async {
    final outcome = await installChosenSkill(home!.overlays, item);
    if (outcome.result == SkillResult.installed || outcome.result == SkillResult.kept) {
      await apply(config.withProfileSkill(item.name, skillPath(item.name)));
    } else {
      problem = '${item.name}: ${outcome.message}';
    }
    await refresh();
    return outcome;
  });

  Future<void> removeSkill(SkillCopy copy) => run('Removing ${copy.name}', () async {
    if (config.profile.skills.containsKey(copy.name)) await apply(config.withProfileSkill(copy.name, null));
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
      if (value != null) InstalledMcp(name: key, server: value, layer: 'yours', origin: config.loadoutMcp[key]),
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
    return apply(changed);
  }

  Future<void> removeMcp(String name) => apply(config.withoutMcp(name));

  /// Installs [entry]'s [option] as the server [name]: written to the profile and off for every agent until allowed.
  Future<CanoaConfig?> installMcp(McpRegistryEntry entry, McpOption option, String name) {
    final definition = option.definition;
    if (definition == null || option.refusal != null) throw StateError('That option is refused: ${option.refusal}');
    return apply(config.withInstalledMcp(name, definition, entry.origin(registry), agents: agents));
  }

  /// Reads the registry's first page for [search], or the next page of the last search.
  Future<void> loadRegistry({String? search, bool more = false}) async {
    final read = fetch;
    if (read == null) {
      registryProblem = 'Registries are read by the desktop app';
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
      final page = parseRegistryPage(await read(listUri(registry, search: _registryQuery, cursor: more ? registryNext : null)));
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
