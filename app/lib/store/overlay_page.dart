import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../profile/overlay_layer.dart';
import '../profile/profile.dart';
import '../profile/profile_home.dart';
import '../profile/skill_copies.dart';
import '../profile/profiles_panel.dart' show EffectiveTable, OverlayReportView, visible;
import 'links.dart';
import 'skills.dart';
import 'store.dart';
import 'store_icon.dart';
import '../selectable.dart';

/// Saves a changed canoa.cfg and gives the configuration saved, or null when it was not.
typedef SaveConfig = Future<CanoaConfig?> Function(CanoaConfig changed);

/// [text] for a place that holds one line: as [visible] shows it, and a line break or tab named too, so text from a
/// catalog or a manifest cannot add a line that reads as Canoa's own.
String oneLine(String text) => visible(text).replaceAll('\n', '<U+000A>').replaceAll('\t', '<U+0009>');

/// How many tags the listing offers as filters: the most used.
const shownTags = 40;

/// One entry's page: first what its catalog claims, then, once fetched and before anything is installed, what the
/// overlay itself says and would do, from its own manifest and files; then install, update or remove.
class OverlayPage extends StatefulWidget {
  const OverlayPage({super.key, required this.listed, required this.catalog, required this.config,
      required this.home, required this.onSave, this.catalogs = const []});

  final ListedEntry listed;
  final LoadedCatalog catalog;
  final CanoaConfig config;
  final ProfileHome? home;
  final SaveConfig onSave;

  /// The catalogs read, through which a skill named without a source is resolved.
  final List<(String, aos.Catalog)> catalogs;

  @override
  State<OverlayPage> createState() => OverlayPageState();
}

class OverlayPageState extends State<OverlayPage> {
  late CanoaConfig config = widget.config;
  OverlayPreview? preview;
  String? busy, problem;

  /// What became of the skills the last install fetched.
  SkillInstallReport? skillReport;

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
        catalogPin: entry.pin, iconLimits: config.store.iconLimits, catalogs: widget.catalogs,
        others: await installedManifests(home, config, except: namedAs(config, name) ?? name), skillPins: config.skillPins);
    if (mounted) setState(() => preview = found);
  });

  Future<void> _install() async {
    final shown = preview!;
    final moved = pinMoved(config, shown);
    if (moved != null) {
      final yes = await showSelectableDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('Move the pin?'),
          content: Text('canoa.cfg pins ${shown.name} to $moved. What was fetched is ${shown.update.digest}, '
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
    final report = SkillInstallReport();
    final changed = await installPreview(widget.home!.overlays, config, preview!,
        catalog: widget.listed.label, movePin: movePin, report: report);
    if (mounted) setState(() => skillReport = report);
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
    final yes = await showSelectableDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Remove ${namedAs(config, name) ?? name}?'),
        content: const Text('Its files, its fetched history and its entry in canoa.cfg go; agents started after this '
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
      final changed = await removeInstalled(widget.home!.overlays, config, name, source, offer: _offerOrphans);
      final saved = await widget.onSave(changed);
      if (saved != null && mounted) {
        setState(() {
          config = saved;
          preview = null;
        });
      }
    });
  }

  /// Offers to remove the skills that only this overlay named and that nothing names now.
  Future<bool> _offerOrphans(List<SkillCopy> orphans) async {
    final yes = await showSelectableDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Remove its skills too?'),
        content: Text('${orphans.map((copy) => oneLine(copy.name)).join(', ')} '
            '${orphans.length == 1 ? 'was' : 'were'} fetched for this overlay, and no other overlay names '
            '${orphans.length == 1 ? 'it' : 'them'}. Skills you chose yourself are never offered.'),
        actions: [
          TextButton(key: const Key('store-orphans-keep'), onPressed: () => Navigator.pop(context, false),
              child: const Text('Keep them')),
          FilledButton(key: const Key('store-orphans-remove'), onPressed: () => Navigator.pop(context, true),
              child: const Text('Remove them')),
        ],
      ),
    );
    return yes == true;
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final target = _target;
    final state = target == null ? null : entryState(config, target.$1, target.$2, overlays: widget.home?.overlays);
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
              Text('canoa.cfg names ${namedAs(config, target!.$1)} from another source. The store neither installs over '
                  'it nor removes it.', key: const Key('store-taken'))
            else if (state == EntryState.byHand)
              Text('canoa.cfg names ${namedAs(config, target!.$1)} from this source, by hand. The store neither updates '
                  'nor removes it; take the entry out of canoa.cfg to let the store manage it.',
                  key: const Key('store-by-hand'))
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
            if (skillReport case final report? when report.outcomes.isNotEmpty) ...[
              const SizedBox(height: 8),
              Text('Its skills', style: text.titleMedium),
              for (final outcome in report.outcomes)
                Text(oneLine('${outcome.name}: ${outcome.message}'), key: Key('store-skill-outcome-${skillKey(outcome.name)}')),
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

  /// The address a Homepage or Repository field names, when it is a plain web address.
  static Uri? _linkOf(String key, String value) => key == 'Homepage' || key == 'Repository' ? webLink(value) : null;

  Widget _fields(Map<String, Object?> fields) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final MapEntry(:key, :value) in fields.entries)
        if (value != null)
          if (_linkOf(key, '$value') case final uri?)
            LinkField(key: Key('store-link-${key.toLowerCase()}'), label: key, uri: uri, shown: oneLine('$value'))
          else
            _field(key, '$value'),
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
    final named = config.overlays.keys.map(aos.normalizeName).toSet();
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
        _field('Source', displaySource(aos.Source.parse(update.source.spec))),
        _field('Commit', update.commit),
        _field('Tree digest', update.digest, key: const Key('store-digest')),
        if (found.pinRefusal case final why?)
          Text(oneLine(why), key: const Key('store-pin-refused'), style: error)
        else if (entry.pin != null)
          const Text('The catalog\'s pin matches what was fetched.', key: Key('store-pin-ok')),
        if (moved != null)
          Text('canoa.cfg pins it to $moved: installing this moves the pin, which it asks first.',
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
        if (found.skills case final review? when !review.isEmpty) ...[
          const SizedBox(height: 8),
          Text('Skills from elsewhere', key: const Key('store-skills-review'), style: text.titleMedium),
          Text(review.fetches
              ? 'Installing fetches these, each from its source and checked against its pin, as one yes with the overlay. '
                  'Nothing has been fetched yet, and fetching runs nothing.'
              : 'Installing fetches none of these.'),
          SkillRows(items: review.items),
          if (review.orphans.isNotEmpty)
            Text(oneLine('No longer named by this overlay: ${review.orphans.map((copy) => copy.name).join(', ')}; '
                'you are offered their removal'), style: text.bodySmall),
        ],
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
            ], report.ungranted, report.unresolved, report.nonDefaults)),
        ],
        for (final id in found.harnesses) Text(oneLine('harness.$id: hints for $id agents only'), style: text.bodySmall),
        for (final (path, os, network) in found.setup)
          Text(oneLine('setup $path${os == null ? '' : ' on ${os.join(', ')}'}: '
              '${network ? 'says it needs the network' : 'declares no network'}; Canoa does not run it'),
              style: text.bodySmall),
        for (final name in found.requires)
          Text(oneLine(named.contains(aos.normalizeName(name))
              ? 'requires $name, which canoa.cfg names'
              : 'requires $name, which canoa.cfg does not name: install it too, or this applies without it'),
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

/// The skills an overlay names from elsewhere, one row each: its source as shown, its pin or that it has none, the
/// catalog that gave the source, and what installing does with it.
class SkillRows extends StatelessWidget {
  const SkillRows({super.key, required this.items});
  final List<SkillItem> items;

  static String _what(SkillItem item) => switch (item.status) {
    SkillStatus.fetch => 'fetched',
    SkillStatus.update => item.why ?? 'fetched again: the source, pin or catalog changed from the one accepted',
    SkillStatus.unchanged => 'kept as accepted',
    SkillStatus.missing => 'not delivered: named, and no catalog offers it',
    SkillStatus.ambiguous => 'not delivered: ${item.catalogs?.join(' and ')} both offer it, and Canoa never picks one',
    SkillStatus.shadowed => 'not delivered: ${item.why}',
    SkillStatus.refused => 'not delivered: ${item.why}',
  };

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final item in items)
        Padding(
          key: Key('store-skill-row-${skillKey(item.name)}'),
          padding: const EdgeInsets.only(top: 6),
          child: SelectableText.rich(TextSpan(children: [
            TextSpan(text: '${oneLine(item.name)}: ', style: const TextStyle(fontWeight: FontWeight.w600)),
            TextSpan(text: oneLine([
              _what(item),
              if (item.source != null) item.source!,
              if (item.fetch != null || item.status == SkillStatus.unchanged)
                item.pin == null ? 'unpinned: nothing says what it should hold' : 'pinned ${item.pin}',
              if (item.configuredPin != null) 'pin required by canoa.cfg [skill_pins]',
              for (final warning in item.warnings)
                switch (warning) {
                  'insecure-transport' => 'over a transport that is not encrypted',
                  'private-source' => 'from a private network address',
                  _ => warning,
                },
              if (item.catalog != null) 'from the catalog ${item.catalog}',
            ].join(' · '))),
          ])),
        ),
    ],
  );
}
