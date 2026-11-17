import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../config/canoa_config.dart';
import '../loadout/loadout_controller.dart';
import '../loadout/mcp_registry.dart';
import '../profile/profile_home.dart';
import '../selectable.dart';
import '../shell/layout_controls.dart';
import '../shell/shell.dart' show ShellScope;
import 'overlay_page.dart' show SaveConfig, shownTags;
import 'programs.dart';
import 'programs_view.dart';
import 'store_catalogs.dart';
import 'store_extension.dart';
import 'store_index.dart';
import 'store_mcps.dart';
import 'contents_storage.dart';
import 'store.dart' show LoadedCatalog;
import 'store_overlays.dart';
import 'store_selection.dart';
import 'store_skills.dart';
import 'store_view.dart';

/// The Store: what an agent can be given, by type (**Overlays**, **Skills**, **Agents**, **MCP servers**, and those extensions add), each with what is
/// installed and a place to discover more, behind one toolbar. An entry is shown before it is installed (what it holds, where it
/// comes from, the digest it is pinned to), and nothing in the Store runs anything on being installed.
class StoreSection extends StatefulWidget {
  const StoreSection({
    super.key,
    required this.config,
    required this.home,
    required this.onSave,
    this.agents = const [],
    this.programs,
    this.fetchRegistry,
    this.findProgram,
    this.registry = officialRegistry,
    this.embedded = false,
    this.index,
    this.now,
    this.selection,
    this.contentsStorage,
    this.readAhead = true,
    this.extensionTypes = const [],
  });

  /// Whether the page is a section of the shell, which never pops: what it changes is saved as it is changed.
  final bool embedded;

  final CanoaConfig config;

  /// Where overlays install and catalogs are fetched; none in a web build.
  final ProfileHome? home;
  final SaveConfig onSave;

  /// The agents an MCP server is allowed or turned off for.
  final List<String> agents;

  /// The programs the connected host's Store keeps; none where that host does not grant them, which hides the Agents type.
  final ProgramStores? programs;

  /// Reads a registry's answer; null where the app reads no registry.
  final RegistryFetch? fetchRegistry;
  final FindProgram? findProgram;
  final String registry;

  /// The host's catalog index; none where the app has no host, which leaves the catalogs empty.
  final CatalogIndex? index;

  /// The clock the "updated … ago" line reads; the time of day when null.
  final DateTime Function()? now;

  /// The type the section's index chose; none where the section has no index, which then shows its types as tabs.
  final StoreSelection? selection;

  /// Where what was read of every entry is kept between runs; none keeps it only while the app runs.
  final ContentsStorage? contentsStorage;

  /// Whether the catalogs' entries are read for what they provide when the catalogs are read.
  final bool readAhead;

  /// The store types extensions add, listed after the built-in ones.
  final List<ExtensionStoreType> extensionTypes;

  @override
  State<StoreSection> createState() => StoreSectionState();
}

class StoreSectionState extends State<StoreSection> with TickerProviderStateMixin {
  late final LoadoutController controller = LoadoutController(
    config: widget.config,
    home: widget.home,
    save: widget.onSave,
    agents: widget.agents,
    registry: widget.registry,
    fetch: widget.fetchRegistry,
    findProgram: widget.findProgram,
    index: widget.index,
    contentsStorage: widget.contentsStorage,
    now: widget.now,
    readAhead: widget.readAhead,
  )..addListener(_changed);

  /// What each type's toolbar holds, by the type's id.
  final views = <String, StoreView>{};

  /// The ids of the types, in the order of the index: the built-in ones, then those extensions add.
  late List<String> types = _typesFor(widget.programs, widget.extensionTypes);
  late TabController tabs = TabController(length: types.length, vsync: this, initialIndex: _indexOf(selection?.selected))
    ..addListener(_changed);

  static List<String> _typesFor(ProgramStores? programs, List<ExtensionStoreType> extensions) => [
    StoreType.overlays.id,
    StoreType.skills.id,
    if (programs != null) StoreType.agents.id,
    StoreType.mcps.id,
    for (final extension in extensions)
      if (StoreType.values.every((type) => type.id != extension.id)) extension.id,
  ];

  int _indexOf(String? id) => id == null ? 0 : types.indexOf(id).clamp(0, types.length - 1);

  DateTime _now() => (widget.now ?? DateTime.now)();

  StoreSelection? get selection => widget.selection;

  /// The id of the type shown.
  String get type => types[tabs.index.clamp(0, types.length - 1)];
  StoreView get view => viewOf(type);

  /// The toolbar state of the type [id]. With an index to choose by, a catalog-backed type opens on what the catalogs offer.
  StoreView viewOf(String id) => views.putIfAbsent(id, () {
    final made = StoreView(id)..addListener(_viewChanged);
    if (selection != null && (id == StoreType.overlays.id || id == StoreType.skills.id)) made.discover = true;
    return made;
  });

  StoreType? _builtin(String id) => StoreType.values.where((type) => type.id == id).firstOrNull;
  ExtensionStoreType? _extension(String id) => widget.extensionTypes.where((type) => type.id == id).firstOrNull;
  String _labelOf(String id) => _builtin(id)?.label ?? _extension(id)?.label ?? id;
  IconData _iconOf(String id) => _builtin(id)?.icon ?? _extension(id)?.icon ?? Icons.extension_outlined;

  @override
  void initState() {
    super.initState();
    unawaited(controller.load());
    for (final id in types) {
      viewOf(id);
    }
    widget.programs?.addListener(_changed);
    _typeChanges = _merged(widget.extensionTypes)?..addListener(_changed);
    selection?.addListener(_selectionChanged);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _publish();
    });
  }

  /// Tells when what any extension type lists changed.
  Listenable? _typeChanges;

  static Listenable? _merged(List<ExtensionStoreType> types) {
    final each = [for (final type in types) ?type.changes];
    return each.isEmpty ? null : Listenable.merge(each);
  }

  @override
  void didUpdateWidget(StoreSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.extensionTypes != widget.extensionTypes) {
      _typeChanges?.removeListener(_changed);
      _typeChanges = _merged(widget.extensionTypes)?..addListener(_changed);
    }
    if (oldWidget.selection != widget.selection) {
      oldWidget.selection?.removeListener(_selectionChanged);
      widget.selection?.addListener(_selectionChanged);
    }
    final programsChanged = oldWidget.programs != widget.programs;
    if (programsChanged) {
      oldWidget.programs?.removeListener(_changed);
      widget.programs?.addListener(_changed);
    }
    final next = _typesFor(widget.programs, widget.extensionTypes);
    if (next.length != types.length || [for (var i = 0; i < next.length; i++) next[i] != types[i]].any((differs) => differs)) {
      final keep = type;
      tabs.dispose();
      types = next;
      for (final id in types) {
        viewOf(id);
      }
      tabs = TabController(length: types.length, vsync: this, initialIndex: _indexOf(selection?.selected ?? keep))..addListener(_changed);
    }
  }

  /// Redraws. A type that asks the host as it first appears changes its programs while the frame is still being built, so
  /// the redraw then waits for the next frame.
  void _changed() {
    if (!mounted) return;
    if (SchedulerBinding.instance.schedulerPhase == SchedulerPhase.persistentCallbacks) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) {
          setState(() {});
          _publish();
        }
      });
    } else {
      setState(() {});
      _publish();
    }
  }

  /// Tells the index which types there are and how much of each.
  void _publish() => selection?.publish([
    for (final id in types)
      StoreTypeRow(
        id: id,
        label: _labelOf(id),
        icon: _iconOf(id),
        installed: _installedCount(id),
        available: _discoverCount(id),
      ),
  ]);

  /// The type chosen in the index is the one shown.
  void _selectionChanged() {
    if (!mounted) return;
    final index = _indexOf(selection?.selected);
    if (tabs.index != index) tabs.index = index;
    setState(() {});
  }

  /// A switch to Discover on the MCP servers type reads the registry the first time.
  void _viewChanged() {
    final mcps = viewOf(StoreType.mcps.id);
    if (mcps.discover && !controller.registryRead && !controller.loadingRegistry) unawaited(controller.loadRegistry());
    _changed();
  }

  @override
  void dispose() {
    widget.programs?.removeListener(_changed);
    _typeChanges?.removeListener(_changed);
    selection?.removeListener(_selectionChanged);
    controller
      ..removeListener(_changed)
      ..dispose();
    for (final each in views.values) {
      each
        ..removeListener(_viewChanged)
        ..dispose();
    }
    tabs.dispose();
    super.dispose();
  }

  // What each type counts.

  int _installedCount(String id) => switch (_builtin(id)) {
    StoreType.agents => widget.programs?.statuses.where((status) => status.installed).length ?? 0,
    StoreType.overlays => controller.config.overlays.length,
    StoreType.skills => controller.skills.length,
    StoreType.mcps => controller.installedMcp().length,
    null => _extension(id)?.items().where((item) => item.installed).length ?? 0,
  };

  int? _discoverCount(String id) => switch (_builtin(id)) {
    StoreType.agents => widget.programs?.statuses.where((status) => !status.installed).length,
    StoreType.overlays => controller.listing?.entries.length,
    StoreType.skills => controller.home == null ? null : controller.skillsOffered().length,
    StoreType.mcps => controller.registryRead ? controller.registryEntries.length : null,
    null => _extension(id)?.items().where((item) => !item.installed).length,
  };

  int get _updates => widget.programs?.statuses.where((status) => status.updateAvailable).length ?? 0;

  /// "Cached · updated 3 min ago", "Stale · updated 2 h ago", or what is happening instead.
  String _statusText(String id) {
    final c = controller;
    switch (_builtin(id)) {
      case StoreType.agents:
        final programs = widget.programs;
        if (programs == null) return '';
        if (programs.loading) return 'Asking the host…';
        final times = [for (final status in programs.statuses) ?status.latestAt];
        if (times.isEmpty) return 'Newest versions not asked yet';
        final oldest = times.reduce((a, b) => a.isBefore(b) ? a : b);
        return 'Cached · updated ${ageText(_now().difference(oldest))}';
      case StoreType.mcps:
        return 'Read live from ${registryShown(c.selectedRegistry)}, not kept';
      case StoreType.overlays:
      case StoreType.skills:
        if (c.index == null || c.home == null) return 'The desktop app reads the catalogs';
        if (c.config.catalogs.isEmpty) return 'No catalogs yet';
        if (c.loadingCatalogs) return 'Reading the catalogs…';
        final shown = _scoped(id);
        final times = [for (final catalog in shown) ?catalog.checked];
        if (times.isEmpty) return 'Not read yet';
        final oldest = times.reduce((a, b) => a.isBefore(b) ? a : b);
        final stale = shown.any((catalog) => catalog.stale || catalog.staleBecause != null);
        return '${stale ? 'Stale' : 'Cached'} · updated ${ageText(_now().difference(oldest))}';
      case null:
        return 'Listed by an extension';
    }
  }

  /// The catalogs the type [id]'s page is about: the one its filter chose, or every one.
  List<LoadedCatalog> _scoped(String id) {
    final chosen = viewOf(id).catalogs;
    return chosen.isEmpty ? controller.loaded : [for (final catalog in controller.loaded) if (chosen.contains(catalog.label)) catalog];
  }

  bool _staleNow(String id) =>
      (id == StoreType.overlays.id || id == StoreType.skills.id) &&
      _scoped(id).any((catalog) => catalog.stale || catalog.staleBecause != null || catalog.problem != null);

  bool get _canRefresh => switch (_builtin(type)) {
    StoreType.agents => widget.programs?.loading == false,
    StoreType.mcps => !controller.loadingRegistry,
    null => false,
    _ => controller.busy == null && controller.home != null && controller.index != null && !controller.loadingCatalogs,
  };

  void _refresh() {
    switch (_builtin(type)) {
      case StoreType.agents:
        unawaited(widget.programs?.refresh(latest: true, force: true));
      case StoreType.mcps:
        unawaited(controller.loadRegistry(search: viewOf(StoreType.mcps.id).query.text));
      default:
        unawaited(controller.load(again: true));
    }
  }

  void _openSettings() => Navigator.of(context).push<void>(selectablePage(builder: (_) => StoreSettingsPage(controller: controller)));

  @override
  Widget build(BuildContext context) {
    final current = type;
    final indexed = selection != null;
    return PopScope(
      canPop: widget.embedded,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(controller.config);
      },
      child: Scaffold(
        appBar: AppBar(
          leading: widget.embedded ? ShellScope.menuButton(context) : null,
          automaticallyImplyLeading: !widget.embedded,
          title: Text(indexed ? _labelOf(current) : 'Store', key: const Key('store-title')),
          actions: [?_layoutControls(context)],
          // With the index beside it the type is chosen there; a Store on its own page has these tabs instead.
          bottom: indexed
              ? null
              : TabBar(
                  key: const Key('store-tabs'),
                  controller: tabs,
                  tabs: [
                    for (final each in types)
                      if (each == StoreType.agents.id)
                        Tab(
                          key: const Key('store-tab-agents'),
                          child: Badge(
                            key: const Key('store-updates-badge'),
                            isLabelVisible: _updates > 0,
                            label: Text('$_updates'),
                            offset: const Offset(10, -6),
                            child: Text('Agents ${_installedCount(each)}'),
                          ),
                        )
                      else
                        Tab(key: Key('store-tab-$each'), text: '${_labelOf(each)} ${_installedCount(each)}'),
                  ],
                ),
        ),
        body: Column(children: [
          _toolbar(context, current),
          if (controller.busy != null) const LinearProgressIndicator(),
          Expanded(
            child: TabBarView(
              controller: tabs,
              physics: indexed ? const NeverScrollableScrollPhysics() : null,
              children: [
                for (final each in types)
                  switch (_builtin(each)) {
                    StoreType.agents => AgentsBody(programs: widget.programs!, view: viewOf(each)),
                    StoreType.overlays => OverlaysBody(controller: controller, view: viewOf(each)),
                    StoreType.skills => SkillsBody(controller: controller, view: viewOf(each)),
                    StoreType.mcps => McpsBody(controller: controller, view: viewOf(each)),
                    null => ExtensionBody(type: _extension(each)!, view: viewOf(each)),
                  },
              ],
            ),
          ),
        ]),
      ),
    );
  }

  /// The index toggle at the header's right, where the layout controls are; none without an index or in a drawer.
  Widget? _layoutControls(BuildContext context) {
    final scope = ShellScope.maybeOf(context);
    if (widget.selection == null || scope == null || scope.narrow) return null;
    return Padding(
      padding: const EdgeInsets.only(right: 8),
      child: LayoutControls(layout: scope.layout, section: 'store', hasIndex: true, hasRight: false, hasBottom: false, menu: false),
    );
  }

  Widget _toolbar(BuildContext context, String current) {
    final view = viewOf(current);
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final builtin = _builtin(current);
    final updates = builtin == StoreType.agents ? _updates : 0;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Wrap(spacing: 12, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.center, children: [
          if (builtin != null)
            InstalledOrDiscover(
              section: current,
              discover: view.discover,
              installedCount: _installedCount(current),
              discoverCount: _discoverCount(current),
              onChanged: (value) => view.discover = value,
            ),
          SizedBox(
            width: 320,
            child: TextField(
              key: const Key('store-search'),
              controller: view.query,
              decoration: InputDecoration(
                prefixIcon: const Icon(Icons.search),
                labelText: 'Search ${_labelOf(current).toLowerCase()}',
                isDense: true,
              ),
              onChanged: (_) => view.changed(),
              onSubmitted: (_) {
                if (builtin == StoreType.mcps && view.discover) unawaited(controller.loadRegistry(search: view.query.text));
              },
            ),
          ),
          IconButton(
            key: const Key('store-refresh'),
            tooltip: switch (builtin) {
              StoreType.agents => 'Check for updates',
              StoreType.mcps => 'Ask the registry again',
              _ => 'Fetch the catalogs again',
            },
            onPressed: _canRefresh ? _refresh : null,
            icon: const Icon(Icons.refresh),
          ),
          if (controller.home != null)
            IconButton(
              key: const Key('store-settings'),
              tooltip: 'Store settings',
              onPressed: _openSettings,
              icon: const Icon(Icons.tune),
            ),
        ]),
        const SizedBox(height: 4),
        Text(
          _statusText(current),
          key: const Key('store-status'),
          style: text.bodySmall?.copyWith(color: _staleNow(current) ? scheme.error : scheme.onSurfaceVariant),
        ),
        ..._filters(context, current, view, updates),
      ]),
    );
  }

  /// The catalogs an overlay, a skill or an extension's item may come from, as "All" and one chip for each.
  Widget? _catalogFilter(StoreView view, List<String> labels) {
    if (labels.length < 2) return null;
    return Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [
      ChoiceChip(
        key: const Key('catalog-filter-all'),
        label: const Text('All catalogs'),
        selected: view.catalogs.isEmpty,
        onSelected: (_) {
          view.catalogs.clear();
          view.changed();
        },
      ),
      for (final label in labels)
        ChoiceChip(
          key: Key('catalog-filter-$label'),
          label: Text(label),
          selected: view.catalogs.contains(label),
          onSelected: (_) {
            // A tag is of the catalog it was picked in.
            view.tags.clear();
            view.catalogs
              ..clear()
              ..add(label);
            view.changed();
          },
        ),
    ]);
  }

  /// A page's catalogs and tags, and Installed's updates.
  List<Widget> _filters(BuildContext context, String current, StoreView view, int updates) {
    final chips = <Widget>[];
    final builtin = _builtin(current);
    if (builtin == StoreType.agents && !view.discover && updates > 0) {
      chips.add(FilterChip(
        key: const Key('agents-updates-filter'),
        label: Text('Updates $updates'),
        selected: view.updatesOnly,
        onSelected: (on) {
          view.updatesOnly = on;
          view.changed();
        },
      ));
    }
    Widget? byCatalog;
    if (builtin == StoreType.overlays || builtin == StoreType.skills) {
      byCatalog = _catalogFilter(view, [for (final catalog in controller.loaded) catalog.label]);
    } else if (_extension(current) case final extension?) {
      byCatalog = _catalogFilter(view, {for (final item in extension.items()) item.catalog}.toList());
    }
    if (builtin == StoreType.overlays && view.discover && controller.config.catalogs.isNotEmpty) {
      if (controller.listing case final listing? when listing.tags.isNotEmpty) {
        // The tags of the chosen catalog only: another catalog's tag would find nothing here.
        for (final tag in {...listing.topTags(shownTags, catalogs: view.catalogs), ...view.tags}) {
          chips.add(FilterChip(
            key: Key('store-tag-$tag'),
            label: Text(tag),
            selected: view.tags.contains(tag),
            onSelected: (on) {
              on ? view.tags.add(tag) : view.tags.remove(tag);
              view.changed();
            },
          ));
        }
      }
    }
    return [
      if (byCatalog != null) ...[const SizedBox(height: 8), byCatalog],
      if (chips.isNotEmpty) ...[
        const SizedBox(height: 8),
        Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: chips),
      ],
    ];
  }
}

/// Installed and Discover, and the counts of each.
class InstalledOrDiscover extends StatelessWidget {
  const InstalledOrDiscover({super.key, required this.section, required this.discover, required this.installedCount,
      required this.discoverCount, required this.onChanged});

  /// The type's name, for the keys.
  final String section;
  final bool discover;
  final int installedCount;
  final int? discoverCount;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) => SegmentedButton<bool>(
    key: Key('$section-switch'),
    showSelectedIcon: false,
    segments: [
      ButtonSegment(value: false, label: Text('Installed $installedCount', key: Key('$section-installed'))),
      ButtonSegment(value: true, label: Text(discoverCount == null ? 'Discover' : 'Discover $discoverCount', key: Key('$section-discover'))),
    ],
    selected: {discover},
    onSelectionChanged: (chosen) => onChanged(chosen.first),
  );
}
