import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../profile/aos.dart';
import '../profile/aos_catalog.dart';
import '../profile/overlay_layer.dart';
import '../profile/profile.dart';
import '../profile/profile_home.dart';
import '../profile/profiles_dialog.dart' show EffectiveTable, OverlayReportView, visible;
import 'store.dart';
import 'store_icon.dart';

/// Saves a changed Canoa.cfg and gives the configuration saved, or null when it was not.
typedef SaveConfig = Future<CanoaConfig?> Function(CanoaConfig changed);

/// [text] for a place that holds one line: as [visible] shows it, and a line break or tab named too, so text from a
/// catalog or a manifest cannot add a line that reads as Canoa's own.
String oneLine(String text) => visible(text).replaceAll('\n', '<U+000A>').replaceAll('\t', '<U+0009>');

/// How many tags the listing offers as filters: the most used.
const shownTags = 40;

/// The overlay store: the catalogs of `[catalogs]`, each fetched as overlays are, their entries searched by text and
/// tag, an entry two catalogs offer shown once per catalog qualified by its label, and each entry's page.
class StorePage extends StatefulWidget {
  const StorePage({super.key, required this.config, required this.home, required this.onSave});

  final CanoaConfig config;

  /// Where overlays install and catalogs are fetched; none in a web build.
  final ProfileHome? home;
  final SaveConfig onSave;

  @override
  State<StorePage> createState() => StorePageState();
}

class StorePageState extends State<StorePage> {
  late CanoaConfig config = widget.config;
  final query = TextEditingController();
  final label = TextEditingController();
  final source = TextEditingController();
  final tags = <String>{};
  List<LoadedCatalog> loaded = const [];
  StoreListing? listing;
  String? busy, problem;

  @override
  void initState() {
    super.initState();
    _loadAll();
  }

  @override
  void dispose() {
    query.dispose();
    label.dispose();
    source.dispose();
    super.dispose();
  }

  Future<void> _loadAll() async {
    final home = widget.home;
    if (home == null) return;
    setState(() => busy = 'Fetching catalogs');
    final found = <LoadedCatalog>[];
    for (final MapEntry(key: name, value: from) in config.catalogs.entries) {
      found.add(await loadCatalog(home.overlays, name, from, config.store));
    }
    if (!mounted) return;
    setState(() {
      loaded = found;
      listing = StoreListing(found);
      tags.retainAll(listing!.tags);
      busy = null;
    });
  }

  Future<void> _save(CanoaConfig changed) async {
    final saved = await widget.onSave(changed);
    if (saved != null && mounted) setState(() => config = saved);
  }

  Future<void> _addCatalog() async {
    final name = label.text.trim(), spec = source.text.trim();
    final CanoaConfig changed;
    try {
      if (config.catalogs.keys.any((key) => normalizeName(key) == normalizeName(name))) {
        throw const FormatException('Another catalog has that label');
      }
      changed = config.withCatalog(name, spec);
    } on FormatException catch (error) {
      setState(() => problem = error.message);
      return;
    }
    setState(() => problem = null);
    await _save(changed);
    label.clear();
    source.clear();
    await _loadAll();
  }

  Future<void> _removeCatalog(String name) async {
    await widget.home?.overlays.removeCatalog(name);
    await _save(config.withoutCatalog(name));
    await _loadAll();
  }

  Future<void> _open(ListedEntry listed) async {
    final saved = await Navigator.of(context).push<CanoaConfig>(MaterialPageRoute(
      builder: (_) => OverlayPage(listed: listed, catalog: listing!.catalogOf(listed), config: config, home: widget.home,
          onSave: widget.onSave),
    ));
    if (saved != null && mounted) setState(() => config = saved);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final shown = listing?.search(query.text, tags: tags) ?? const <ListedEntry>[];
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(config);
      },
      child: Scaffold(
        appBar: AppBar(
          title: const Text('Overlay store'),
          actions: [
            IconButton(
              key: const Key('store-refresh'),
              tooltip: 'Fetch the catalogs again',
              onPressed: busy != null || widget.home == null ? null : _loadAll,
              icon: const Icon(Icons.refresh),
            ),
          ],
        ),
        body: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            if (widget.home == null)
              const Text('The desktop app fetches catalogs and installs overlays.')
            else ...[
              TextField(
                key: const Key('store-search'),
                controller: query,
                decoration: const InputDecoration(prefixIcon: Icon(Icons.search), labelText: 'Search overlays'),
                onChanged: (_) => setState(() {}),
              ),
              if (listing case final listing? when listing.tags.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Wrap(spacing: 6, runSpacing: 6, children: [
                    for (final tag in {...listing.topTags(shownTags), ...tags})
                      FilterChip(
                        key: Key('store-tag-$tag'),
                        label: Text(tag),
                        selected: tags.contains(tag),
                        onSelected: (on) => setState(() => on ? tags.add(tag) : tags.remove(tag)),
                      ),
                  ]),
                ),
              if (busy != null) ...[const SizedBox(height: 8), const LinearProgressIndicator(), Text('$busy…')],
              const SizedBox(height: 8),
              if (config.catalogs.isEmpty)
                const Text('No catalogs yet: add one below, a git repository and the catalog file in it.')
              else if (busy == null && shown.isEmpty)
                const Text('Nothing listed matches.'),
              for (final listed in shown) _entryTile(listed),
              const SizedBox(height: 16),
              Text('Catalogs', style: text.titleMedium),
              const Text('A catalog lists overlays by name and source. What it says of an overlay is its author\'s claim; '
                  'an overlay\'s page shows what the overlay itself does once fetched.'),
              for (final catalog in loaded) _catalogTile(catalog),
              const SizedBox(height: 8),
              _addForm(),
              if (problem != null) Text(visible(problem!), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              for (final weakening in config.store.weakenings)
                Text('[store] $weakening: looser than the default', style: text.bodySmall),
            ],
          ],
        ),
      ),
    );
  }

  Widget _entryTile(ListedEntry listed) {
    final entry = listed.entry;
    final catalog = listing!.catalogOf(listed);
    final icon = catalog.icons[entry.name];
    final summary = entry.about['summary'];
    final tagLine = [for (final tag in (entry.about['tags'] as List?) ?? const []) '$tag'];
    final status = switch (_state(entry)) {
      null => null,
      EntryState.installed => 'Installed',
      EntryState.taken => 'Another overlay of this name is in Canoa.cfg',
      EntryState.absent => null,
    };
    return Card(
      key: Key('store-entry-${listed.label}-${entry.name}'),
      child: ListTile(
        leading: StillIcon(name: entry.name, path: entry.about['icon'] as String?, bytes: icon?.bytes,
            limits: config.store.iconLimits),
        title: Text(oneLine(listed.shown)),
        subtitle: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          if (summary is String) Text(oneLine(summary)),
          Text(oneLine('${listed.label} · ${displaySource(entry.source)}'),
              style: Theme.of(context).textTheme.bodySmall),
          if (tagLine.isNotEmpty) Text(oneLine(tagLine.join(' · ')), style: Theme.of(context).textTheme.bodySmall),
          if (status != null) Text(status, style: Theme.of(context).textTheme.labelMedium),
        ]),
        isThreeLine: true,
        onTap: () => _open(listed),
      ),
    );
  }

  EntryState? _state(CatalogEntry entry) {
    try {
      return entryState(config, installName(entry), installSource(entry));
    } on StoreRefusal {
      return null;
    }
  }

  Widget _catalogTile(LoadedCatalog catalog) {
    final read = catalog.catalog;
    final lines = [
      displaySource(AosSource(SourceKind.git, catalog.source.repo, catalog.source.reference, catalog.source.path)),
      if (catalog.commit case final commit?) 'At commit $commit',
      ?catalog.problem,
      if (read != null) ...[
        if (read.name case final name?) 'It calls itself $name',
        '${read.entries.length} listed${read.refused.isEmpty ? '' : ', ${read.refused.length} refused'}',
        if (catalog.leftOut > 0)
          '${catalog.leftOut} more left out, past [store] catalog_max_entries = ${config.store.catalogMaxEntries}',
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
        onPressed: busy != null ? null : () => _removeCatalog(catalog.label),
        child: const Text('Remove'),
      ),
    );
  }

  Widget _addForm() => Wrap(spacing: 8, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.end, children: [
    SizedBox(
      width: 160,
      child: TextField(key: const Key('store-catalog-label'), controller: label,
          decoration: const InputDecoration(labelText: 'Label')),
    ),
    SizedBox(
      width: 420,
      child: TextField(key: const Key('store-catalog-source'), controller: source,
          decoration: const InputDecoration(labelText: 'Source, <repository>@<ref>#<catalog file>')),
    ),
    FilledButton(key: const Key('store-add-catalog'), onPressed: busy != null ? null : _addCatalog,
        child: const Text('Add catalog')),
  ]);
}

/// One entry's page: first what its catalog claims, then, once fetched and before anything is installed, what the
/// overlay itself says and would do, from its own manifest and files; then install, update or remove.
class OverlayPage extends StatefulWidget {
  const OverlayPage({super.key, required this.listed, required this.catalog, required this.config,
      required this.home, required this.onSave});

  final ListedEntry listed;
  final LoadedCatalog catalog;
  final CanoaConfig config;
  final ProfileHome? home;
  final SaveConfig onSave;

  @override
  State<OverlayPage> createState() => OverlayPageState();
}

class OverlayPageState extends State<OverlayPage> {
  late CanoaConfig config = widget.config;
  OverlayPreview? preview;
  String? busy, problem;

  CatalogEntry get entry => widget.listed.entry;

  (String, GitOverlay)? get _target {
    try {
      return (installName(entry), installSource(entry));
    } on StoreRefusal {
      return null;
    }
  }

  String? get _refusal {
    try {
      installName(entry);
      installSource(entry);
      return null;
    } on StoreRefusal catch (error) {
      return error.message;
    }
  }

  Future<void> _run(String what, Future<void> Function() action) async {
    setState(() {
      busy = what;
      problem = null;
    });
    try {
      await action();
    } on Object catch (error) {
      if (mounted) setState(() => problem = '$what: $error');
    } finally {
      if (mounted) setState(() => busy = null);
    }
  }

  @override
  void dispose() {
    _discard();
    super.dispose();
  }

  /// What a preview fetched goes when the page closes without installing it.
  void _discard() {
    final shown = preview, home = widget.home;
    if (shown != null && home != null) unawaited(discardPreview(home.overlays, shown.name));
  }

  Future<void> _fetch() => _run('Fetching', () async {
    final home = widget.home!;
    final (name, source) = _target!;
    final found = await previewOverlay(home.overlays, namedAs(config, name) ?? name, source,
        os: home.os, shared: mergeProfiles([Layer(const LayerId(LayerKind.user), config.profile)]).shareEnv,
        catalogPin: entry.pin, iconLimits: config.store.iconLimits);
    if (mounted) setState(() => preview = found);
  });

  Future<void> _install() async {
    final shown = preview!;
    final moved = pinMoved(config, shown);
    if (moved != null) {
      final yes = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('Move the pin?'),
          content: Text('Canoa.cfg pins ${shown.name} to $moved. What was fetched is ${shown.update.digest}, '
              'shown on this page. Installing it moves the pin there.'),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            FilledButton(key: const Key('store-move-pin'), onPressed: () => Navigator.pop(context, true),
                child: const Text('Move the pin and install')),
          ],
        ),
      );
      if (yes != true) return;
    }
    await _installing(movePin: moved != null);
  }

  Future<void> _installing({required bool movePin}) => _run('Installing', () async {
    final changed = await installPreview(widget.home!.overlays, config, preview!, movePin: movePin);
    final saved = await widget.onSave(changed);
    if (saved != null && mounted) {
      setState(() {
        config = saved;
        preview = null;
      });
    }
  });

  Future<void> _remove() async {
    final (name, source) = _target!;
    final yes = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Remove ${namedAs(config, name) ?? name}?'),
        content: const Text('Its files, its fetched history and its entry in Canoa.cfg go; agents started after this '
            'start without it.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(key: const Key('store-remove-confirm'), onPressed: () => Navigator.pop(context, true),
              child: const Text('Remove')),
        ],
      ),
    );
    if (yes != true) return;
    await _run('Removing', () async {
      final changed = await removeInstalled(widget.home!.overlays, config, name, source);
      final saved = await widget.onSave(changed);
      if (saved != null && mounted) {
        setState(() {
          config = saved;
          preview = null;
        });
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final target = _target;
    final state = target == null ? null : entryState(config, target.$1, target.$2);
    final found = preview;
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(config);
      },
      child: Scaffold(
        appBar: AppBar(title: Text(oneLine(widget.listed.shown))),
        body: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            _claims(text),
            const Divider(height: 32),
            if (_refusal case final why?)
              Text(visible(why), key: const Key('store-refusal'), style: TextStyle(color: Theme.of(context).colorScheme.error))
            else if (state == EntryState.taken)
              Text('Canoa.cfg names ${namedAs(config, target!.$1)} from another source. The store neither installs over '
                  'it nor removes it.', key: const Key('store-taken'))
            else if (widget.home == null)
              const Text('The desktop app fetches and installs overlays.')
            else if (found == null)
              Align(
                alignment: Alignment.centerLeft,
                child: FilledButton.tonal(
                  key: const Key('store-fetch'),
                  onPressed: busy != null ? null : _fetch,
                  child: Text(state == EntryState.installed ? 'Fetch and show what an update changes' : 'Fetch and show what it would do'),
                ),
              )
            else ...[
              _fetched(found, text),
              const SizedBox(height: 12),
              _actions(found, state!),
            ],
            if (state == EntryState.installed && found == null && widget.home != null) ...[
              const SizedBox(height: 8),
              Align(
                alignment: Alignment.centerLeft,
                child: TextButton(key: const Key('store-remove'), onPressed: busy != null ? null : _remove,
                    child: const Text('Remove…')),
              ),
            ],
            if (busy != null) ...[const LinearProgressIndicator(), Text('$busy…')],
            if (problem != null)
              Text(visible(problem!), key: const Key('store-problem'),
                  style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ],
        ),
      ),
    );
  }

  /// The catalog's claims, labelled as such.
  Widget _claims(TextTheme text) {
    final about = entry.about;
    final icon = widget.catalog.icons[entry.name];
    return Column(
      key: const Key('store-claims'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(children: [
          StillIcon(name: entry.name, path: about['icon'] as String?, bytes: icon?.bytes, size: 56,
              limits: config.store.iconLimits),
          const SizedBox(width: 12),
          Expanded(child: Text(oneLine(widget.listed.shown), style: text.titleLarge)),
        ]),
        const SizedBox(height: 8),
        Text('What the catalog ${widget.listed.label} says', style: text.titleMedium),
        _fields({
          'Summary': about['summary'],
          'Homepage': about['homepage'],
          'Repository': about['repository'],
          'License': about['license'],
          'Authors': (about['authors'] as List?)?.join(', '),
          'Tags': (about['tags'] as List?)?.join(', '),
        }),
        _field('Source', displaySource(entry.source), key: const Key('store-source')),
        _field('Pin', entry.pin ?? 'none: the catalog pins nothing (unpinned), and installing pins what is fetched'),
        if (icon?.refused case final why?) _field('Icon', 'refused ($why): a placeholder shows instead'),
        if (entry.description case final description?)
          _quoted('Its description, as the catalog writes it', description, const Key('store-claim-description')),
      ],
    );
  }

  /// Free text of another's, after the fields Canoa vouches for and set apart from them, its lines kept.
  Widget _quoted(String label, String value, Key key) => Container(
    key: key,
    margin: const EdgeInsets.only(top: 8),
    padding: const EdgeInsets.only(left: 12),
    decoration: BoxDecoration(border: Border(left: BorderSide(color: Theme.of(context).colorScheme.outline, width: 3))),
    child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text(label, style: Theme.of(context).textTheme.labelMedium),
      SelectableText(visible(value), style: const TextStyle(fontStyle: FontStyle.italic)),
    ]),
  );

  Widget _fields(Map<String, Object?> fields) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final MapEntry(:key, :value) in fields.entries)
        if (value != null) _field(key, '$value'),
    ],
  );

  Widget _field(String label, String value, {Key? key}) => Padding(
    padding: const EdgeInsets.only(top: 4),
    child: SelectableText.rich(key: key, TextSpan(children: [
      TextSpan(text: '$label: ', style: const TextStyle(fontWeight: FontWeight.w600)),
      TextSpan(text: oneLine(value)),
    ])),
  );

  /// What the overlay itself says and would do, from its own manifest and files.
  Widget _fetched(OverlayPreview found, TextTheme text) {
    final update = found.update;
    final about = found.about;
    final error = TextStyle(color: Theme.of(context).colorScheme.error);
    final manifestName = found.manifest['name'];
    final description = found.manifest['description'];
    final named = config.overlays.keys.map(normalizeName).toSet();
    final layer = found.layer;
    final moved = pinMoved(config, found);
    Widget files(String label, List<String> paths) =>
        paths.isEmpty ? const SizedBox.shrink() : _field(label, paths.join(', '));
    return Column(
      key: const Key('store-fetched'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(children: [
          StillIcon(key: const Key('store-own-icon'), name: found.name, path: about['icon'] as String?,
              bytes: found.icon, size: 56, limits: config.store.iconLimits),
          const SizedBox(width: 12),
          Expanded(child: Text(oneLine(manifestName is String ? manifestName : found.name), style: text.titleLarge)),
        ]),
        const SizedBox(height: 8),
        Text('What the overlay itself says', style: text.titleMedium),
        _fields({
          'Summary': about['summary'],
          'Homepage': about['homepage'],
          'Repository': about['repository'],
          'License': about['license'],
          'Authors': (about['authors'] as List?)?.join(', '),
          'Tags': (about['tags'] as List?)?.join(', '),
        }),
        _field('Source', displaySource(parseSource(update.source.spec))),
        _field('Commit', update.commit),
        _field('Tree digest', update.digest, key: const Key('store-digest')),
        if (found.pinRefusal case final why?)
          Text(oneLine(why), key: const Key('store-pin-refused'), style: error)
        else if (entry.pin != null)
          const Text('The catalog\'s pin matches what was fetched.', key: Key('store-pin-ok')),
        if (moved != null)
          Text('Canoa.cfg pins it to $moved: installing this moves the pin, which it asks first.',
              key: const Key('store-pin-moves'), style: error),
        if (found.iconRefused case final why?) _field('Icon', 'refused ($why): a placeholder shows instead'),
        if (description is String)
          _quoted('Its description, as its overlay.cfg writes it', description, const Key('store-own-description')),
        if (update.from != null) ...[
          const SizedBox(height: 8),
          Text('What an update changes', style: text.titleMedium),
          _field('From', update.from!),
          if (update.changesNothing) const Text('Nothing changes.'),
          files('Added', update.added),
          files('Changed', update.changed),
          files('Removed', update.removed),
        ],
        files('Left out: no part of an overlay, never installed', update.skipped),
        const SizedBox(height: 8),
        Text('What it would do', style: text.titleMedium),
        const Text('From its own overlay.cfg and files, as Settings, Profiles would apply it. Nothing of it has run, '
            'and Canoa runs no setup, env script, dependency install or command of an overlay.'),
        if (found.refusal case final why?)
          Text(visible('Canoa would leave it out: $why'), key: const Key('store-would-refuse'), style: error)
        else if (layer != null) ...[
          EffectiveTable(key: const Key('store-behaviour'), effective: mergeProfiles([layer])),
          if (found.report case final report?)
            OverlayReportView(report: OverlayReport(report.layer, report.digest, [
              for (final line in report.notApplied)
                if (!line.endsWith('hints for another harness')) line,
            ], report.ungranted, report.unresolved, report.weakenings)),
        ],
        for (final id in found.harnesses) Text(oneLine('harness.$id: hints for $id agents only'), style: text.bodySmall),
        for (final (path, os, network) in found.setup)
          Text(oneLine('setup $path${os == null ? '' : ' on ${os.join(', ')}'}: '
              '${network ? 'says it needs the network' : 'declares no network'}; Canoa does not run it'),
              style: text.bodySmall),
        for (final name in found.requires)
          Text(oneLine(named.contains(normalizeName(name))
              ? 'requires $name, which Canoa.cfg names'
              : 'requires $name, which Canoa.cfg does not name: install it too, or this applies without it'),
              style: text.bodySmall),
      ],
    );
  }

  Widget _actions(OverlayPreview found, EntryState state) {
    final installed = state == EntryState.installed && widget.home!.overlays.installed(namedAs(config, found.name)!) != null;
    final canApply = found.installable && !(installed && found.update.changesNothing) && busy == null;
    return Wrap(spacing: 8, children: [
      FilledButton(
        key: const Key('store-install'),
        onPressed: canApply ? _install : null,
        child: Text(pinMoved(config, found) != null
            ? 'Update and move the pin…'
            : '${installed ? 'Update' : 'Install'} and pin ${_short(found.update.digest)}'),
      ),
      if (state == EntryState.installed)
        TextButton(key: const Key('store-remove'), onPressed: busy != null ? null : _remove, child: const Text('Remove…')),
      TextButton(
        onPressed: busy != null
            ? null
            : () {
                _discard();
                setState(() => preview = null);
              },
        child: const Text('Close'),
      ),
    ]);
  }

  static String _short(String digest) => digest.length > 19 ? '${digest.substring(0, 19)}…' : digest;
}

/// Opens the store over [context], and gives the configuration as the store left it.
Future<CanoaConfig?> showStore(BuildContext context,
        {required CanoaConfig config, required ProfileHome? home, required SaveConfig onSave}) =>
    Navigator.of(context).push<CanoaConfig>(MaterialPageRoute(
      builder: (_) => StorePage(config: config, home: home, onSave: onSave),
    ));
