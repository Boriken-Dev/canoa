import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../profile/profile.dart';
import '../profile/profile_home.dart';
import '../profile/profiles_dialog.dart' show OverlayIcon, OverlayUpdateDialog, visible;
import '../selectable.dart';
import '../store/programs.dart';
import '../store/programs_view.dart';
import '../store/store.dart';
import '../store/store_icon.dart';
import '../store/store_page.dart' show OverlayPage, oneLine, shownTags;
import 'loadout_controller.dart';
import 'loadout_mcps.dart';
import 'loadout_skills.dart';
import 'mcp_registry.dart';

/// Loadout: what an agent is given beyond itself, in three sections: **Overlays**, **Skills** and **MCPs**, each with what is
/// installed and a place to discover more. An overlay, a skill and an MCP server are all shown before they are installed
/// (what they hold, where they come from, the digest they are pinned to), and none of them runs anything on being installed.
class LoadoutPage extends StatefulWidget {
  const LoadoutPage({
    super.key,
    required this.config,
    required this.home,
    required this.onSave,
    this.agents = const [],
    this.programs,
    this.fetchRegistry,
    this.findProgram,
    this.registry = officialRegistry,
  });

  final CanoaConfig config;

  /// Where overlays install and catalogs are fetched; none in a web build.
  final ProfileHome? home;
  final SaveConfig onSave;

  /// The agents an MCP server is allowed or turned off for.
  final List<String> agents;

  /// The programs the host's Store keeps, reached from the page's menu; none where the host has no Store.
  final ProgramStores? programs;

  /// Reads a registry's answer; null where the app reads no registry.
  final RegistryFetch? fetchRegistry;
  final FindProgram? findProgram;
  final String registry;

  @override
  State<LoadoutPage> createState() => LoadoutPageState();
}

class LoadoutPageState extends State<LoadoutPage> with SingleTickerProviderStateMixin {
  late final LoadoutController controller = LoadoutController(
    config: widget.config,
    home: widget.home,
    save: widget.onSave,
    agents: widget.agents,
    registry: widget.registry,
    fetch: widget.fetchRegistry,
    findProgram: widget.findProgram,
  )..addListener(_changed);
  late final TabController tabs = TabController(length: 3, vsync: this);

  @override
  void initState() {
    super.initState();
    unawaited(controller.load());
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    controller
      ..removeListener(_changed)
      ..dispose();
    tabs.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final skillsCount = controller.skills.length;
    final mcpCount = controller.installedMcp().length;
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(controller.config);
      },
      child: Scaffold(
        appBar: AppBar(
          title: const Text('Loadout'),
          actions: [
            IconButton(
              key: const Key('loadout-refresh'),
              tooltip: 'Fetch the catalogs again',
              onPressed: controller.busy != null || widget.home == null ? null : controller.load,
              icon: const Icon(Icons.refresh),
            ),
            if (widget.programs case final programs?)
              IconButton(
                key: const Key('loadout-programs'),
                tooltip: 'Agent programs',
                icon: const Icon(Icons.apps_outlined),
                onPressed: () => Navigator.of(context).push<void>(selectablePage(
                  builder: (_) => Scaffold(
                    appBar: AppBar(title: const Text('Agent programs')),
                    body: ListView(padding: const EdgeInsets.all(16), children: [ProgramsSection(programs: programs)]),
                  ),
                )),
              ),
          ],
          bottom: TabBar(
            key: const Key('loadout-tabs'),
            controller: tabs,
            tabs: [
              Tab(key: const Key('loadout-tab-overlays'), text: 'Overlays ${controller.config.overlays.length}'),
              Tab(key: const Key('loadout-tab-skills'), text: 'Skills $skillsCount'),
              Tab(key: const Key('loadout-tab-mcps'), text: 'MCPs $mcpCount'),
            ],
          ),
        ),
        body: Column(children: [
          if (controller.busy != null) const LinearProgressIndicator(),
          Expanded(
            child: TabBarView(controller: tabs, children: [
              OverlaysSection(controller: controller, page: this),
              SkillsSection(controller: controller),
              McpsSection(controller: controller),
            ]),
          ),
        ]),
      ),
    );
  }
}

/// Installed and Discover, and the counts of each.
class InstalledOrDiscover extends StatelessWidget {
  const InstalledOrDiscover({super.key, required this.section, required this.discover, required this.installedCount,
      required this.discoverCount, required this.onChanged});

  /// The section's name, for the keys.
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

/// A card of the three sections: an icon, a title, lines under it, and what the person can do.
class LoadoutCard extends StatelessWidget {
  const LoadoutCard({super.key, required this.leading, required this.title, this.lines = const [], this.provenance,
      this.trailing, this.footer, this.onTap});

  final Widget leading;
  final String title;
  final List<String> lines;

  /// Where it comes from, in one line, under the others.
  final String? provenance;
  final Widget? trailing;
  final Widget? footer;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
              leading,
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(oneLine(title), style: text.titleSmall),
                  for (final line in lines) Text(oneLine(line), style: text.bodyMedium),
                  if (provenance != null)
                    Text(oneLine(provenance!), key: const Key('provenance'),
                        style: text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant)),
                ]),
              ),
              if (trailing != null) ...[const SizedBox(width: 8), trailing!],
            ]),
            if (footer != null) ...[const SizedBox(height: 8), footer!],
          ]),
        ),
      ),
    );
  }
}

/// A letter on a disc, for a card with no icon of its own.
class LoadoutMark extends StatelessWidget {
  const LoadoutMark({super.key, required this.name, this.icon});
  final String name;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => icon != null
      ? CircleAvatar(radius: 16, child: Icon(icon, size: 18))
      : OverlayPlaceholder(name: name, size: 32);
}

/// The Overlays section.
class OverlaysSection extends StatefulWidget {
  const OverlaysSection({super.key, required this.controller, required this.page});
  final LoadoutController controller;
  final LoadoutPageState page;

  @override
  State<OverlaysSection> createState() => _OverlaysSectionState();
}

class _OverlaysSectionState extends State<OverlaysSection> with AutomaticKeepAliveClientMixin {
  bool discover = false;
  final query = TextEditingController();
  final tags = <String>{};
  final catalogsShown = <String>{};
  final label = TextEditingController();
  final spec = TextEditingController();

  LoadoutController get c => widget.controller;

  @override
  bool get wantKeepAlive => true;

  @override
  void dispose() {
    query.dispose();
    label.dispose();
    spec.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final offered = c.listing?.entries.length;
    return ListView(padding: const EdgeInsets.all(16), children: [
      Align(
        alignment: Alignment.centerLeft,
        child: InstalledOrDiscover(
          section: 'overlays',
          discover: discover,
          installedCount: c.config.overlays.length,
          discoverCount: offered,
          onChanged: (value) => setState(() => discover = value),
        ),
      ),
      const SizedBox(height: 12),
      if (c.problem != null)
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Text(visible(c.problem!), key: const Key('loadout-problem'),
              style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
      if (discover) ..._discover(context) else ..._installed(context),
    ]);
  }

  // Installed.

  List<Widget> _installed(BuildContext context) {
    if (c.config.overlays.isEmpty) {
      return [
        const Text('No overlays yet. An overlay is a directory of rules, skills, MCP servers and settings that an agent starts '
            'with. Find some in Discover.', key: Key('overlays-none')),
        const SizedBox(height: 8),
        FilledButton.tonal(
          key: const Key('overlays-to-discover'),
          onPressed: () => setState(() => discover = true),
          child: const Text('Discover overlays'),
        ),
      ];
    }
    return [
      const Text('Each overlay a new session starts with, unless you turn it off here. A session already started keeps what it started with.'),
      const SizedBox(height: 8),
      for (final MapEntry(key: name, value: source) in c.config.overlays.entries) _installedCard(context, name, source),
    ];
  }

  Widget _installedCard(BuildContext context, String name, OverlaySource source) {
    final record = c.home?.overlays.installed(name);
    final layer = c.layerOf(name);
    final aosLayer = layer?.aos;
    final report = c.reportOf(name);
    final icon = aosLayer?.about['icon'];
    final iconBytes = icon == null ? null : aosLayer?.icon[icon];
    final provenance = switch (source) {
      DirOverlay(:final dir, :final pin) =>
        'the directory $dir, read in place · ${pin == null ? 'not pinned' : 'pinned ${_short(pin)}'}',
      GitOverlay() => [
        if (record?.catalog case final catalog?) 'from the catalog $catalog' else 'named in canoa.cfg by hand',
        source.spec,
        if (record != null) 'commit ${record.commit.substring(0, record.commit.length < 8 ? record.commit.length : 8)}',
        if (source.pin != null) 'pinned ${_short(source.pin!)}' else 'not pinned',
      ].join(' · '),
    };
    final profile = layer?.profile;
    // The variables its own manifest sets, not those AOS adds to every overlay's environment.
    final envSet = aosLayer?.manifest['env'] is Map ? (aosLayer!.manifest['env'] as Map).length : 0;
    final applies = <String>[
      if (profile != null) ...[
        if (envSet > 0) '$envSet environment ${envSet == 1 ? 'variable' : 'variables'}',
        if (profile.skills.isNotEmpty) '${profile.skills.length} ${profile.skills.length == 1 ? 'skill' : 'skills'}',
        if (profile.mcp.isNotEmpty) '${profile.mcp.length} MCP ${profile.mcp.length == 1 ? 'server' : 'servers'} (off until allowed)',
        if (profile.contexts.isNotEmpty) 'context',
      ],
    ];
    final notApplied = report?.notApplied ?? const <String>[];
    final left = layer == null ? c.leftOutBecause(name) : null;
    final fromStore = record?.catalog != null;
    final on = c.isDefaultOn(name);
    return LoadoutCard(
      key: Key('overlay-installed-$name'),
      leading: icon != null && iconBytes != null
          ? OverlayIcon(name: name, path: icon, bytes: iconBytes, limits: c.config.store.iconLimits)
          : OverlayPlaceholder(name: name, size: 32),
      title: name,
      lines: [
        ?aosLayer?.about['summary'],
        if (left != null) 'Left out: $left',
        if (applies.isNotEmpty) 'Applies: ${applies.join(' · ')}',
        if (left == null && layer != null && applies.isEmpty) 'Applies nothing of its own',
        if (notApplied.isNotEmpty) 'Not applied by Canoa yet: ${notApplied.join(', ')}',
      ],
      provenance: provenance,
      trailing: Column(crossAxisAlignment: CrossAxisAlignment.end, children: [
        Switch(
          key: Key('overlay-default-$name'),
          value: on,
          onChanged: (value) => unawaited(c.setDefault(name, value)),
        ),
        Text(on ? 'On for new sessions' : 'Off for new sessions',
            key: Key('overlay-default-label-$name'), style: Theme.of(context).textTheme.labelSmall),
      ]),
      footer: Wrap(spacing: 8, children: [
        if (source is GitOverlay && c.home != null)
          TextButton(
            key: Key('overlay-update-$name'),
            onPressed: c.busy != null ? null : () => unawaited(_update(name, source)),
            child: const Text('Check for updates…'),
          ),
        if (source is GitOverlay && fromStore && c.home != null)
          TextButton(
            key: Key('overlay-remove-$name'),
            onPressed: c.busy != null ? null : () => unawaited(_remove(name, source)),
            child: const Text('Remove…'),
          ),
      ]),
    );
  }

  static String _short(String digest) => digest.length > 19 ? '${digest.substring(0, 19)}…' : digest;

  Future<void> _update(String name, GitOverlay source) async {
    final home = c.home!;
    final update = await c.run('Checking $name', () => home.overlays.check(name, source));
    if (update == null || !mounted) return;
    final apply = await showSelectableDialog<bool>(context: context, builder: (_) => OverlayUpdateDialog(update: update));
    if (apply != true) return;
    await c.run('Updating $name', () async {
      final record = await home.overlays.apply(update);
      await c.apply(c.config.withOverlayPin(name, record.digest!));
    });
  }

  Future<void> _remove(String name, GitOverlay source) async {
    final yes = await showSelectableDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Remove $name?'),
        content: const Text('Its files, its fetched history and its entry in canoa.cfg go; agents started after this start without it.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(key: const Key('overlay-remove-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
        ],
      ),
    );
    if (yes != true) return;
    await c.run('Removing $name', () async {
      final changed = await removeInstalled(c.home!.overlays, c.config, name, source);
      await c.apply(changed);
    });
  }

  // Discover.

  List<Widget> _discover(BuildContext context) {
    final text = Theme.of(context).textTheme;
    if (c.home == null) return const [Text('The desktop app fetches catalogs and installs overlays.')];
    if (c.config.catalogs.isEmpty) return [_emptyState(context)];
    final listing = c.listing;
    final shown = [
      for (final listed in listing?.search(query.text, tags: tags) ?? const <ListedEntry>[])
        if (catalogsShown.isEmpty || catalogsShown.contains(listed.label)) listed,
    ];
    return [
      TextField(
        key: const Key('store-search'),
        controller: query,
        decoration: const InputDecoration(prefixIcon: Icon(Icons.search), labelText: 'Search overlays'),
        onChanged: (_) => setState(() {}),
      ),
      const SizedBox(height: 8),
      Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [
        for (final catalog in c.loaded)
          FilterChip(
            key: Key('catalog-filter-${catalog.label}'),
            label: Text(catalog.label),
            selected: catalogsShown.contains(catalog.label),
            onSelected: (on) => setState(() => on ? catalogsShown.add(catalog.label) : catalogsShown.remove(catalog.label)),
          ),
        if (listing case final listing? when listing.tags.isNotEmpty)
          for (final tag in {...listing.topTags(shownTags), ...tags})
            FilterChip(
              key: Key('store-tag-$tag'),
              label: Text(tag),
              selected: tags.contains(tag),
              onSelected: (on) => setState(() => on ? tags.add(tag) : tags.remove(tag)),
            ),
      ]),
      if (c.loadingCatalogs) ...[const SizedBox(height: 8), const LinearProgressIndicator(), const Text('Fetching catalogs…')],
      const SizedBox(height: 8),
      if (!c.loadingCatalogs && shown.isEmpty) const Text('Nothing listed matches.', key: Key('overlays-nothing')),
      for (final listed in shown) _discoverCard(context, listed),
      const SizedBox(height: 8),
      ExpansionTile(
        key: const Key('catalogs-manage'),
        tilePadding: EdgeInsets.zero,
        title: Text('Catalogs ${c.config.catalogs.length}', style: text.titleSmall),
        children: [
          const Align(
            alignment: Alignment.centerLeft,
            child: Text('A catalog lists overlays, and skills, by name and source. What it says of an overlay is its author\'s '
                'claim; an overlay\'s page shows what the overlay itself does once fetched.'),
          ),
          for (final catalog in c.loaded) _catalogTile(catalog),
          const SizedBox(height: 8),
          _addForm(context),
          for (final nonDefault in c.config.store.nonDefaults) Text('[store] $nonDefault', style: text.bodySmall),
        ],
      ),
    ];
  }

  Widget _discoverCard(BuildContext context, ListedEntry listed) {
    final entry = listed.entry;
    final catalog = c.listing!.catalogOf(listed);
    final icon = catalog.icons[entry.name];
    final tagLine = [for (final tag in (entry.about['tags'] as List?) ?? const []) '$tag'];
    final status = _stateOf(entry);
    return LoadoutCard(
      key: Key('store-entry-${listed.label}-${entry.name}'),
      leading: StillIcon(name: entry.name, path: entry.about['icon'] as String?, bytes: icon?.bytes, limits: c.config.store.iconLimits),
      title: listed.shown,
      lines: [
        if (entry.about['summary'] case final String summary) summary,
        if (tagLine.isNotEmpty) tagLine.join(' · '),
        ?status,
      ],
      provenance: 'from the catalog ${listed.label} · ${displaySource(entry.source)}',
      onTap: () => unawaited(_open(listed)),
    );
  }

  String? _stateOf(CatalogEntry entry) {
    try {
      return switch (entryState(c.config, installName(entry), installSource(entry), overlays: c.home?.overlays)) {
        EntryState.installed => 'Installed',
        EntryState.taken => 'Another overlay of this name is in canoa.cfg',
        EntryState.byHand => 'Named in canoa.cfg by hand',
        EntryState.absent => null,
      };
    } on StoreRefusal {
      return null;
    }
  }

  Future<void> _open(ListedEntry listed) async {
    final saved = await Navigator.of(context).push<CanoaConfig>(selectablePage(
      builder: (_) => OverlayPage(
        listed: listed,
        catalog: c.listing!.catalogOf(listed),
        config: c.config,
        home: c.home,
        onSave: c.apply,
        catalogs: skillCatalogs(c.loaded),
      ),
    ));
    if (saved != null) await c.adopt(saved);
  }

  Widget _catalogTile(LoadedCatalog catalog) {
    final read = catalog.catalog;
    final lines = [
      displaySource(aos.Source('git', catalog.source.repo, catalog.source.reference, catalog.source.path)),
      if (catalog.commit case final commit?) 'At commit $commit',
      ?catalog.problem,
      if (read != null) ...[
        if (read.name case final name?) 'It calls itself $name',
        '${read.entries.length} overlays and ${read.skills.length} skills listed${read.refused.isEmpty ? '' : ', ${read.refused.length} refused'}',
        if (catalog.leftOut > 0) '${catalog.leftOut} more left out, past [store] catalog_max_entries = ${c.config.store.catalogMaxEntries}',
        for (final (name, why) in read.refused) '  refused $name: $why',
        for (final pointer in read.ignored) '  left out $pointer',
        for (final MapEntry(key: name, value: icon) in catalog.icons.entries)
          if (icon.refused case final why?) '  icon of $name refused: $why',
      ],
    ];
    return ListTile(
      key: Key('store-catalog-${catalog.label}'),
      contentPadding: EdgeInsets.zero,
      title: Text(catalog.label),
      subtitle: Text(lines.map(oneLine).join('\n')),
      trailing: TextButton(
        key: Key('store-remove-catalog-${catalog.label}'),
        onPressed: c.busy != null ? null : () => unawaited(c.removeCatalog(catalog.label)),
        child: const Text('Remove'),
      ),
    );
  }

  Widget _addForm(BuildContext context) => Wrap(spacing: 8, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.end, children: [
    SizedBox(
      width: 160,
      child: TextField(key: const Key('store-catalog-label'), controller: label,
          decoration: const InputDecoration(labelText: 'Label, optional')),
    ),
    SizedBox(
      width: 420,
      child: TextField(key: const Key('store-catalog-source'), controller: spec,
          decoration: const InputDecoration(labelText: 'Source', hintText: 'https://github.com/ana/overlays.git@main#catalog.toml')),
    ),
    FilledButton(
      key: const Key('store-add-catalog'),
      onPressed: c.busy != null ? null : () => unawaited(_add(spec.text, label.text)),
      child: const Text('Add catalog'),
    ),
  ]);

  Future<void> _add(String source, String name) async {
    await c.addCatalog(name, source);
    label.clear();
    spec.clear();
  }

  /// What a person with no catalog sees: where to find some, and how to add one, never a bare repository field.
  Widget _emptyState(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Column(key: const Key('catalogs-empty'), crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Find overlays and skills', style: text.titleMedium),
      const SizedBox(height: 4),
      const Text('A catalog is a list of overlays and skills kept in a git repository. Add one to browse what it lists, '
          'read what each would do, and install it.'),
      const SizedBox(height: 12),
      Text('Catalogs to start from', style: text.titleSmall),
      for (final known in knownCatalogs)
        Card(
          key: Key('known-catalog-${known.label}'),
          child: ListTile(
            title: Text(known.label),
            subtitle: Text('${known.about}\n${known.spec.split('@').first.replaceAll('.git', '')}'),
            isThreeLine: true,
            trailing: FilledButton.tonal(
              key: Key('known-catalog-add-${known.label}'),
              onPressed: c.busy != null ? null : () => unawaited(_add(known.spec, known.label)),
              child: const Text('Add'),
            ),
          ),
        ),
      const SizedBox(height: 12),
      Text('Or paste a catalog', style: text.titleSmall),
      const SizedBox(height: 4),
      const Text('A git repository, a branch or tag, and the catalog file in it: https://github.com/ana/overlays.git@main#catalog.toml'),
      const SizedBox(height: 8),
      _addForm(context),
    ]);
  }
}

/// Opens Loadout over [context], and gives the configuration as it left it.
Future<CanoaConfig?> showLoadout(BuildContext context, {
  required CanoaConfig config,
  required ProfileHome? home,
  required SaveConfig onSave,
  List<String> agents = const [],
  ProgramStores? programs,
  RegistryFetch? fetchRegistry,
  FindProgram? findProgram,
  String registry = officialRegistry,
}) => Navigator.of(context).push<CanoaConfig>(selectablePage(
  builder: (_) => LoadoutPage(config: config, home: home, onSave: onSave, agents: agents, programs: programs,
      fetchRegistry: fetchRegistry, findProgram: findProgram, registry: registry),
));
