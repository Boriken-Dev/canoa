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
import 'store_index.dart';
import 'store_mcps.dart';
import 'contents_storage.dart';
import 'store.dart' show LoadedCatalog;
import 'store_overlays.dart';
import 'store_selection.dart';
import 'store_skills.dart';
import 'store_view.dart';

/// The Store: what an agent can be given, by type (**Agents**, **Overlays**, **Skills**, **MCPs**), each with what is
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

  /// The store the section's index chose, which Overlays and Skills are scoped to; none where the section has no index.
  final StoreSelection? selection;

  /// Where what was read of every entry is kept between runs; none keeps it only while the app runs.
  final ContentsStorage? contentsStorage;

  /// Whether the catalogs' entries are read for what they provide when the catalogs are read.
  final bool readAhead;

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
  final views = {for (final type in StoreType.values) type: StoreView(type)};
  late List<StoreType> types = _typesFor(widget.programs);
  late TabController tabs = TabController(length: types.length, vsync: this)..addListener(_changed);

  static List<StoreType> _typesFor(ProgramStores? programs) => [
    if (programs != null) StoreType.agents,
    StoreType.overlays,
    StoreType.skills,
    StoreType.mcps,
  ];

  DateTime _now() => (widget.now ?? DateTime.now)();

  StoreSelection? get selection => widget.selection;

  StoreType get type => types[tabs.index.clamp(0, types.length - 1)];
  StoreView get view => views[type]!;

  @override
  void initState() {
    super.initState();
    unawaited(controller.load());
    _scope(switchTab: false);
    for (final each in views.values) {
      each.addListener(_viewChanged);
    }
    widget.programs?.addListener(_changed);
    selection?.addListener(_selectionChanged);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _publish();
    });
  }

  @override
  void didUpdateWidget(StoreSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.selection != widget.selection) {
      oldWidget.selection?.removeListener(_selectionChanged);
      widget.selection?.addListener(_selectionChanged);
      _scope(switchTab: false);
    }
    if (oldWidget.programs != widget.programs) {
      oldWidget.programs?.removeListener(_changed);
      widget.programs?.addListener(_changed);
      final next = _typesFor(widget.programs);
      if (next.length != types.length) {
        final keep = type;
        tabs.dispose();
        types = next;
        tabs = TabController(length: types.length, vsync: this, initialIndex: types.contains(keep) ? types.indexOf(keep) : 0)
          ..addListener(_changed);
      }
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

  /// Tells the index which stores have been read.
  void _publish() => selection?.publish(controller.loaded, loading: controller.loadingCatalogs);

  /// The store chosen in the index scopes Overlays and Skills to it, and shows what it lists.
  void _selectionChanged() {
    if (!mounted) return;
    _scope(switchTab: true);
    setState(() {});
  }

  void _scope({required bool switchTab}) {
    final chosen = selection?.selected;
    for (final each in [views[StoreType.overlays]!, views[StoreType.skills]!]) {
      final scoped = chosen == null ? const <String>{} : {chosen};
      final same = each.catalogs.length == scoped.length && each.catalogs.containsAll(scoped);
      if (same) continue;
      each.catalogs
        ..clear()
        ..addAll(scoped);
      // A tag is of the store it was picked in.
      each.tags.clear();
      if (chosen != null) each.discover = true;
      each.changed();
    }
    if (chosen != null && switchTab && type != StoreType.overlays && type != StoreType.skills) {
      final overlays = types.indexOf(StoreType.overlays);
      if (overlays >= 0) tabs.animateTo(overlays);
    }
  }

  /// A switch to Discover on the MCPs type reads the registry the first time.
  void _viewChanged() {
    final mcps = views[StoreType.mcps]!;
    if (mcps.discover && !controller.registryRead && !controller.loadingRegistry) unawaited(controller.loadRegistry());
    _changed();
  }

  @override
  void dispose() {
    widget.programs?.removeListener(_changed);
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

  int _installedCount(StoreType type) => switch (type) {
    StoreType.agents => widget.programs?.statuses.where((status) => status.installed).length ?? 0,
    StoreType.overlays => controller.config.overlays.length,
    StoreType.skills => controller.skills.length,
    StoreType.mcps => controller.installedMcp().length,
  };

  int? _discoverCount(StoreType type) => switch (type) {
    StoreType.agents => widget.programs?.statuses.where((status) => !status.installed).length,
    StoreType.overlays => controller.listing?.entries.length,
    StoreType.skills => controller.home == null ? null : controller.skillsOffered().length,
    StoreType.mcps => controller.registryRead ? controller.registryEntries.length : null,
  };

  int get _updates => widget.programs?.statuses.where((status) => status.updateAvailable).length ?? 0;

  /// "Cached · updated 3 min ago", "Stale · updated 2 h ago", or what is happening instead.
  String _statusText(StoreType type) {
    final c = controller;
    switch (type) {
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
        final shown = _scoped;
        final times = [for (final catalog in shown) ?catalog.checked];
        if (times.isEmpty) return 'Not read yet';
        final oldest = times.reduce((a, b) => a.isBefore(b) ? a : b);
        final stale = shown.any((catalog) => catalog.stale || catalog.staleBecause != null);
        return '${stale ? 'Stale' : 'Cached'} · updated ${ageText(_now().difference(oldest))}';
    }
  }

  /// The catalogs the content is about: the chosen store, or every one.
  List<LoadedCatalog> get _scoped {
    final chosen = selection?.selected;
    return chosen == null ? controller.loaded : [for (final catalog in controller.loaded) if (catalog.label == chosen) catalog];
  }

  bool _staleNow(StoreType type) =>
      (type == StoreType.overlays || type == StoreType.skills) &&
      _scoped.any((catalog) => catalog.stale || catalog.staleBecause != null || catalog.problem != null);

  bool get _canRefresh => switch (type) {
    StoreType.agents => widget.programs?.loading == false,
    StoreType.mcps => !controller.loadingRegistry,
    _ => controller.busy == null && controller.home != null && controller.index != null && !controller.loadingCatalogs,
  };

  void _refresh() {
    switch (type) {
      case StoreType.agents:
        unawaited(widget.programs?.refresh(latest: true, force: true));
      case StoreType.mcps:
        unawaited(controller.loadRegistry(search: views[StoreType.mcps]!.query.text));
      default:
        unawaited(controller.load(again: true));
    }
  }

  void _openSettings() => Navigator.of(context).push<void>(selectablePage(builder: (_) => StoreSettingsPage(controller: controller)));

  @override
  Widget build(BuildContext context) {
    final current = type;
    return PopScope(
      canPop: widget.embedded,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(controller.config);
      },
      child: Scaffold(
        appBar: AppBar(
          leading: widget.embedded ? ShellScope.menuButton(context) : null,
          automaticallyImplyLeading: !widget.embedded,
          title: const Text('Store'),
          actions: [?_layoutControls(context)],
          bottom: TabBar(
            key: const Key('store-tabs'),
            controller: tabs,
            tabs: [
              for (final each in types)
                if (each == StoreType.agents)
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
                  Tab(key: Key('store-tab-${each.id}'), text: '${each.label} ${_installedCount(each)}'),
            ],
          ),
        ),
        body: Column(children: [
          _toolbar(context, current),
          if (controller.busy != null) const LinearProgressIndicator(),
          Expanded(
            child: TabBarView(controller: tabs, children: [
              for (final each in types)
                switch (each) {
                  StoreType.agents => AgentsBody(programs: widget.programs!, view: views[each]!),
                  StoreType.overlays => OverlaysBody(controller: controller, view: views[each]!),
                  StoreType.skills => SkillsBody(controller: controller, view: views[each]!),
                  StoreType.mcps => McpsBody(controller: controller, view: views[each]!),
                },
            ]),
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

  Widget _toolbar(BuildContext context, StoreType current) {
    final view = views[current]!;
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final updates = current == StoreType.agents ? _updates : 0;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Wrap(spacing: 12, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.center, children: [
          InstalledOrDiscover(
            section: current.id,
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
                labelText: 'Search ${current.label.toLowerCase()}',
                isDense: true,
              ),
              onChanged: (_) => view.changed(),
              onSubmitted: (_) {
                if (current == StoreType.mcps && view.discover) unawaited(controller.loadRegistry(search: view.query.text));
              },
            ),
          ),
          IconButton(
            key: const Key('store-refresh'),
            tooltip: switch (current) {
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

  /// Discover's catalogs and tags, and Installed's updates.
  List<Widget> _filters(BuildContext context, StoreType current, StoreView view, int updates) {
    final chips = <Widget>[];
    if (current == StoreType.agents && !view.discover && updates > 0) {
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
    if ((current == StoreType.overlays || current == StoreType.skills) && view.discover && controller.config.catalogs.isNotEmpty) {
      if (current == StoreType.overlays) {
        if (controller.listing case final listing? when listing.tags.isNotEmpty) {
          // The tags of the chosen store only: another store's tag would find nothing here.
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
    }
    if (chips.isEmpty) return const [];
    return [
      const SizedBox(height: 8),
      Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: chips),
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
