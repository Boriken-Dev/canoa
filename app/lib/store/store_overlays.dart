import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../loadout/loadout_controller.dart';
import '../loadout/overlay_knowledge.dart';
import '../loadout/overlay_options.dart';
import '../profile/profile.dart';
import '../profile/profiles_panel.dart' show OverlayIcon, OverlayUpdateDialog, visible;
import '../selectable.dart';
import 'overlay_needs.dart' show NeedLine;
import 'overlay_page.dart' show OverlayPage, oneLine;
import 'store.dart';
import 'store_banner.dart';
import 'store_cards.dart';
import 'store_catalogs.dart';
import 'store_icon.dart';
import 'store_runtimes.dart' show runtimesTypeId;
import 'store_view.dart';

/// The Overlays type: the overlays canoa.cfg names, and the overlays the catalogs list.
class OverlaysBody extends StatefulWidget {
  const OverlaysBody({super.key, required this.controller, required this.view, this.showType});

  final LoadoutController controller;
  final StoreView view;

  /// Shows another type of the Store by its id, for an overlay that needs a runtime the Runtimes type installs.
  final void Function(String id)? showType;

  @override
  State<OverlaysBody> createState() => _OverlaysBodyState();
}

class _OverlaysBodyState extends State<OverlaysBody> with AutomaticKeepAliveClientMixin {
  LoadoutController get c => widget.controller;
  StoreView get view => widget.view;

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final problem = c.problem == null
        ? const <Widget>[]
        : [
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(visible(c.problem!), key: const Key('loadout-problem'),
                  style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          ];
    return view.discover ? _discover(context, problem) : _installed(context, problem);
  }

  // Installed.

  Widget _installed(BuildContext context, List<Widget> head) {
    if (c.config.overlays.isEmpty) {
      return CardList(header: [
        ...head,
        const Text('No overlays yet. An overlay is a directory of rules, skills, MCP servers and settings that an agent starts '
            'with. Find some in Discover.', key: Key('overlays-none')),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: FilledButton.tonal(
            key: const Key('overlays-to-discover'),
            onPressed: () => view.discover = true,
            child: const Text('Discover overlays'),
          ),
        ),
      ]);
    }
    final shown = [
      for (final MapEntry(key: name, value: source) in c.config.overlays.entries)
        if (view.matches('$name\n${c.layerOf(name)?.aos?.about['summary'] ?? ''}') &&
            (view.catalogs.isEmpty || view.catalogs.contains(c.home?.overlays.installed(name)?.catalog)))
          (name, source),
    ];
    return CardList(
      header: [
        ...head,
        const Text('Each overlay a new session starts with, unless you turn it off here. A session already started keeps what it started with.'),
        const SizedBox(height: 8),
        if (shown.isEmpty) const Text('No installed overlay matches.', key: Key('overlays-installed-nothing')),
      ],
      cards: [for (final (name, source) in shown) _installedCard(context, name, source)],
    );
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
    final options = aosLayer == null ? const <DeclaredOption>[] : declaredOptions(aosLayer.manifest);
    final notApplied = report?.notApplied ?? const <String>[];
    final needs = c.needs[name] ?? const <NeedLine>[];
    final unmet = [for (final line in needs) if (!line.met) line];
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
        if (needs.any((line) => line.met)) 'Found on this computer: ${[for (final line in needs) if (line.met) line.key.replaceFirst('runtime.', '')].join(', ')}',
        for (final line in unmet) 'Needs ${line.key.replaceFirst('runtime.', '')}: ${line.how}',
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
      footer: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        if (layer != null) ...[
          OverlayKnowledge(
            overlay: name,
            choice: c.config.overlayKb(name),
            hinted: (aosLayer?.manifest['knowledge'] as Map?)?['serve'] == true,
            busy: c.busy != null,
            onChanged: (choice) => unawaited(c.apply(c.config.withOverlayKb(name, choice))),
          ),
          const SizedBox(height: 8),
        ],
        if (options.isNotEmpty)
          Theme(
            data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
            child: ExpansionTile(
              key: Key('overlay-options-$name'),
              tilePadding: EdgeInsets.zero,
              childrenPadding: const EdgeInsets.only(top: 8, bottom: 8),
              expandedCrossAxisAlignment: CrossAxisAlignment.start,
              title: Text('Options (${options.length})', style: Theme.of(context).textTheme.labelLarge),
              children: [
                OverlayOptionsForm(
                  overlay: name,
                  options: options,
                  saved: c.config.overlayOptions(name),
                  busy: c.busy != null,
                  onSave: (values) async {
                    await c.apply(c.config.withOverlayOptions(name, values));
                  },
                ),
              ],
            ),
          ),
        Wrap(spacing: 8, children: [
          for (final offer in {for (final line in unmet) ?line.offer})
            TextButton(
              key: Key('overlay-needs-install-$name-$offer'),
              onPressed: widget.showType == null ? null : () => widget.showType!(runtimesTypeId),
              child: const Text('Install from the Store'),
            ),
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
      ]),
    );
  }

  static String _short(String digest) => digest.length > 19 ? '${digest.substring(0, 19)}…' : digest;

  Future<void> _update(String name, GitOverlay source) async {
    final home = c.home!;
    final update = await c.run('Checking $name', () => home.overlays.check(name, source, catalog: home.overlays.installed(name)?.catalog));
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

  Widget _discover(BuildContext context, List<Widget> head) {
    if (c.home == null) return CardList(header: [...head, const Text('The desktop app fetches catalogs and installs overlays.')]);
    if (c.config.catalogs.isEmpty) return CardList(header: [...head, NoCatalogs(controller: c)]);
    final listing = c.listing;
    final inScope = [
      for (final listed in listing?.entries ?? const <ListedEntry>[])
        if (view.catalogs.isEmpty || view.catalogs.contains(listed.label)) listed,
    ];
    final shown = [
      for (final listed in listing?.search(view.query.text, tags: view.tags) ?? const <ListedEntry>[])
        if (view.catalogs.isEmpty || view.catalogs.contains(listed.label)) listed,
    ];
    final scope = [for (final loaded in c.loaded) if (loaded.catalog != null && (view.catalogs.isEmpty || view.catalogs.contains(loaded.label))) loaded];
    final text = Theme.of(context).textTheme;
    final filtered = view.tags.isNotEmpty || view.query.text.isNotEmpty;
    return CardList(
      header: [
        ...head,
        ...sourceNotes(c, only: view.catalogs),
        if (c.loadingCatalogs) const LinearProgressIndicator(),
        if (scope.isNotEmpty)
          StoreBanner(
            specs: bannersFor(
              noun: 'overlays',
              title: 'Overlays',
              scope: scope,
              items: [for (final listed in inScope) _bannerItem(listed)],
              showTag: (tag) {
                view.tags
                  ..clear()
                  ..add(tag);
                view.changed();
              },
            ),
            iconLimits: c.config.store.iconLimits,
            mark: StoreType.overlays.icon,
          ),
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Row(children: [
            Text(filtered ? 'Matching overlays' : 'Top overlays', style: text.titleMedium),
            const SizedBox(width: 8),
            Chip(key: const Key('store-count'), label: Text('${shown.length}'), visualDensity: VisualDensity.compact),
            const Spacer(),
            if (filtered)
              TextButton(
                key: const Key('store-show-all'),
                onPressed: () {
                  view.tags.clear();
                  view.query.clear();
                  view.changed();
                },
                child: const Text('Show all →'),
              ),
          ]),
        ),
        if (!c.loadingCatalogs && shown.isEmpty)
          Text(
            filtered
                ? 'Nothing listed matches.'
                : view.catalogs.isEmpty
                    ? 'No catalog lists an overlay.'
                    : '${view.catalogs.first} lists no overlays.',
            key: const Key('overlays-nothing'),
          ),
      ],
      cards: [for (final listed in shown) _tile(context, listed, c.listing!.catalogOf(listed))],
    );
  }

  BannerItem _bannerItem(ListedEntry listed) => BannerItem(
    name: listed.entry.name,
    catalog: listed.label,
    iconPath: listed.entry.about['icon'] as String?,
    iconBytes: c.listing!.catalogOf(listed).icons[listed.entry.name]?.bytes,
    tags: [for (final tag in (listed.entry.about['tags'] as List?) ?? const []) '$tag'],
    open: () => unawaited(_open(listed)),
  );

  final _installing = <String>{};

  Widget _tile(BuildContext context, ListedEntry listed, LoadedCatalog catalog) {
    final entry = listed.entry;
    final icon = catalog.icons[entry.name];
    final key = '${listed.label}/${entry.name}';
    return StoreTile(
      key: Key('store-entry-${listed.label}-${entry.name}'),
      leading: StillIcon(name: entry.name, path: entry.about['icon'] as String?, bytes: icon?.bytes, size: 48,
          limits: c.config.store.iconLimits),
      title: listed.shown,
      summary: entry.about['summary'] as String?,
      by: '${publisherOf(listed)} · ${listed.label}',
      needs: needsOf(catalog.facts['overlay:${entry.name}']),
      installed: isInstalled(c.config, entry),
      busy: _installing.contains(key),
      installKey: Key('store-entry-install-${listed.label}-${entry.name}'),
      onOpen: () => unawaited(_open(listed)),
      onInstall: c.home == null || c.busy != null ? null : () => unawaited(_quickInstall(listed)),
    );
  }

  /// Installs from the tile, through the path the page takes. What needs a look first (a pin that moved or does not match,
  /// skills to fetch, an overlay Canoa would leave out) opens the page instead, and nothing is installed.
  Future<void> _quickInstall(ListedEntry listed) async {
    final home = c.home!;
    final entry = listed.entry;
    final key = '${listed.label}/${entry.name}';
    final String name;
    final GitOverlay source;
    try {
      name = installName(entry);
      source = installSource(entry);
    } on StoreRefusal {
      return _open(listed);
    }
    setState(() => _installing.add(key));
    var needsLook = false;
    try {
      await c.run('Installing $name', () async {
        final named = namedAs(c.config, name) ?? name;
        final preview = await previewOverlay(home.overlays, named, source,
            os: home.os,
            catalogPin: entry.pin, catalog: listed.label, iconLimits: c.config.store.iconLimits, catalogs: skillCatalogs(c.loaded),
            others: await installedManifests(home, c.config, except: named), skillPins: c.config.skillPins);
        if (!preview.installable || pinMoved(c.config, preview) != null || (preview.skills?.isEmpty == false)) {
          needsLook = true;
          await discardPreview(home.overlays, preview.name);
          return;
        }
        await c.apply(await installPreview(home.overlays, c.config, preview, catalog: listed.label));
      });
    } finally {
      if (mounted) setState(() => _installing.remove(key));
    }
    if (needsLook && mounted) await _open(listed);
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
        contents: c.contents,
      ),
    ));
    if (saved != null) await c.adopt(saved);
  }
}

/// One line for each catalog the host could not read now: a catalog whose last copy is shown, with why it is old, and one with
/// no copy, with why.
List<Widget> sourceNotes(LoadoutController c, {Set<String> only = const {}}) => [
  for (final catalog in c.loaded)
    if (only.isEmpty || only.contains(catalog.label))
    if (catalog.problem ?? catalog.staleBecause case final why?)
      Padding(
        key: Key('store-source-note-${catalog.label}'),
        padding: const EdgeInsets.only(bottom: 4),
        child: Text(
          catalog.catalog == null
              ? oneLine('${catalog.label}: $why')
              : oneLine('${catalog.label}: showing the last copy, since it could not be refreshed: $why'),
        ),
      ),
];
