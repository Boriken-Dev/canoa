import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../loadout/loadout_controller.dart';
import '../profile/profile.dart';
import '../profile/profiles_panel.dart' show visible;
import '../profile/skill_copies.dart';
import '../selectable.dart';
import 'entry_details.dart';
import 'overlay_page.dart' show SkillRows, oneLine;
import 'skills.dart';
import 'store.dart' show LoadedCatalog;
import 'store_banner.dart';
import 'store_cards.dart';
import 'store_icon.dart';
import 'store_overlays.dart' show sourceNotes;
import 'store_view.dart';

/// The Skills type: the skills fetched on this computer, and the skills the catalogs offer.
///
/// A skill is an Agent Skills directory (agentskills.io): a `SKILL.md` with YAML front matter naming it, and whatever it
/// refers to. One is fetched from its git source only after its source and pin are shown and the person says yes, its files
/// are checked against the pin before any is read, and nothing it holds runs on being installed.
class SkillsBody extends StatefulWidget {
  const SkillsBody({super.key, required this.controller, required this.view});

  final LoadoutController controller;
  final StoreView view;

  @override
  State<SkillsBody> createState() => _SkillsBodyState();
}

class _SkillsBodyState extends State<SkillsBody> with AutomaticKeepAliveClientMixin {
  final spec = TextEditingController();

  LoadoutController get c => widget.controller;
  StoreView get view => widget.view;

  @override
  bool get wantKeepAlive => true;

  @override
  void dispose() {
    spec.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final problem = c.problem == null
        ? const <Widget>[]
        : [
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(visible(c.problem!), key: const Key('skills-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          ];
    if (c.home == null) return CardList(header: [...problem, const Text('The desktop app fetches skills.')]);
    return view.discover ? _discover(context, problem) : _installed(context, problem);
  }

  Widget _installed(BuildContext context, List<Widget> head) {
    if (c.skills.isEmpty) {
      return CardList(header: [
        ...head,
        const Text('No skills fetched yet. A skill an overlay names is fetched when you install the overlay; others you choose '
            'yourself in Discover.', key: Key('skills-none')),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: FilledButton.tonal(key: const Key('skills-to-discover'), onPressed: () => view.discover = true, child: const Text('Discover skills')),
        ),
      ]);
    }
    final shown = [for (final copy in c.skills.values) if (view.matches(copy.name)) copy];
    return CardList(
      header: [...head, if (shown.isEmpty) const Text('No installed skill matches.', key: Key('skills-installed-nothing'))],
      cards: [for (final copy in shown) _installedCard(context, copy)],
    );
  }

  Widget _installedCard(BuildContext context, SkillCopy copy) {
    final shown = _sourceShown(copy.source);
    final removable = copy.chosen || copy.overlays.isEmpty;
    final text = Theme.of(context).textTheme;
    final on = copy.chosen ? c.skillIsOn(copy.name) : copy.overlays.isNotEmpty;
    return LoadoutCard(
      key: Key('skill-installed-${skillKey(copy.name)}'),
      leading: const LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined),
      title: copy.name,
      lines: [
        if (copy.chosen) 'Chosen by you' else if (copy.overlays.isNotEmpty) 'Comes with ${copy.overlays.join(', ')}' else 'Named by no overlay now',
      ],
      provenance: [
        if (copy.catalog != null) 'from the catalog ${copy.catalog}' else 'pasted source',
        shown,
        copy.pin == null ? 'unpinned, fetched as ${copy.digest}' : 'pinned ${copy.pin}',
      ].join(' · '),
      trailing: copy.chosen
          ? Column(crossAxisAlignment: CrossAxisAlignment.end, children: [
              Switch(key: Key('skill-default-${skillKey(copy.name)}'), value: on, onChanged: (value) => unawaited(c.setSkillOn(copy.name, value))),
              Text(on ? 'On for new sessions' : 'Off for new sessions', style: text.labelSmall),
            ])
          : null,
      footer: removable
          ? Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                key: Key('skill-remove-${skillKey(copy.name)}'),
                onPressed: c.busy != null ? null : () => unawaited(c.removeSkill(copy)),
                child: const Text('Remove'),
              ),
            )
          : null,
    );
  }

  static String _sourceShown(String key) {
    try {
      return aos.Source.parse(key).shown();
    } on aos.AosError {
      return 'a source Canoa cannot show';
    }
  }

  Widget _discover(BuildContext context, List<Widget> head) {
    final offered = c.skillsOffered();
    final inScope = [for (final item in offered) if (view.catalogs.isEmpty || view.catalogs.contains(item.$1)) item];
    final shown = [
      for (final item in inScope)
        if (view.matches('${item.$2}\n${item.$1}\n${item.$3.description ?? ''}\n${item.$3.source.shown()}') &&
            view.tags.every((tag) => _tagsOf(item.$3).contains(tag)))
          item,
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
              noun: 'skills',
              title: 'Skills',
              scope: scope,
              items: [for (final (label, name, offer) in inScope) _bannerItem(label, name, offer)],
              showTag: (tag) {
                view.tags
                  ..clear()
                  ..add(tag);
                view.changed();
              },
            ),
            iconLimits: c.config.store.iconLimits,
            mark: StoreType.skills.icon,
          ),
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Row(children: [
            Text(filtered ? 'Matching skills' : 'Top skills', style: text.titleMedium),
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
        if (offered.isEmpty && !c.loadingCatalogs)
          Text(c.config.catalogs.isEmpty
              ? 'No catalogs yet, so nothing is offered. Add one in Store settings, or paste a skill below.'
              : 'None of your catalogs offers a skill that is not here already. Paste one below.',
              key: const Key('skills-nothing'))
        else if (shown.isEmpty && !c.loadingCatalogs)
          const Text('Nothing listed matches.', key: Key('skills-nothing-matches')),
      ],
      cards: [for (final (label, name, offer) in shown) _offerTile(label, name, offer)],
      footer: [
        const SizedBox(height: 16),
        Text('Paste a skill', style: Theme.of(context).textTheme.titleSmall),
        const Text('A git repository, a branch or tag, and the skill\'s directory: https://github.com/ana/skills.git@main#skills/review. '
            'Its SKILL.md names it, and the name has to match the directory.'),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: TextField(key: const Key('skills-paste'), controller: spec,
                decoration: const InputDecoration(labelText: 'Source', hintText: 'https://github.com/ana/skills.git@main#skills/review')),
          ),
          const SizedBox(width: 8),
          FilledButton(key: const Key('skills-paste-review'), onPressed: c.busy != null ? null : () => unawaited(_pasted()), child: const Text('Review…')),
        ]),
      ],
    );
  }

  static List<String> _tagsOf(aos.CatalogEntry offer) => [for (final tag in (offer.about?['tags'] as List?) ?? const []) '$tag'];

  LoadedCatalog? _catalogOf(String label) => c.loaded.where((catalog) => catalog.label == label).firstOrNull;

  BannerItem _bannerItem(String label, String name, aos.CatalogEntry offer) => BannerItem(
    name: name,
    catalog: label,
    iconPath: offer.about?['icon'] as String?,
    iconBytes: _catalogOf(label)?.icons[name]?.bytes,
    tags: _tagsOf(offer),
    open: () => unawaited(_details(label, name, offer)),
  );

  /// A skill the catalogs offer, as a tile: the + reviews it and installs it on a yes.
  Widget _offerTile(String label, String name, aos.CatalogEntry offer) => StoreTile(
    key: Key('store-offered-skill-$label-$name'),
    leading: StillIcon(name: name, path: offer.about?['icon'] as String?, bytes: _catalogOf(label)?.icons[name]?.bytes, size: 48,
        limits: c.config.store.iconLimits),
    title: name,
    summary: offer.description ?? offer.about?['summary'] as String?,
    by: '${publisherFrom(offer.about ?? const {}, offer.source.shown())} · $label',
    needs: needsOf(_catalogOf(label)?.facts['skill:$name']),
    installed: false,
    installKey: Key('store-skill-install-$label-$name'),
    onOpen: () => unawaited(_details(label, name, offer)),
    onInstall: c.busy != null ? null : () => unawaited(_review(label, name, offer)),
  );

  Future<void> _details(String label, String name, aos.CatalogEntry offer) async {
    final catalog = c.loaded.where((each) => each.label == label).firstOrNull;
    final about = offer.about ?? const <String, Object>{};
    await Navigator.of(context).push<void>(selectablePage(
      builder: (_) => EntryDetails(
        key: Key('skill-details-$label-$name'),
        title: name,
        leading: const LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined),
        from: 'the catalog $label',
        summary: offer.description ?? about['summary'] as String?,
        overview: {
          for (final field in const ['homepage', 'repository', 'license'])
            if (about[field] case final String value) '${field[0].toUpperCase()}${field.substring(1)}': value,
          if (about['authors'] case final List<Object?> authors when authors.isNotEmpty) 'Authors': authors.join(', '),
          if (about['tags'] case final List<Object?> tags when tags.isNotEmpty) 'Tags': tags.join(', '),
        },
        source: {
          'Source': offer.source.shown(),
          'Pin': offer.pin ?? 'none: the catalog pins nothing, and installing pins what is fetched',
          'Catalog': label,
        },
        facts: catalog?.facts['skill:$name'],
        technical: {
          if (offer.pin != null) 'Pin digest': offer.pin!,
          if (catalog?.commit != null) 'Catalog commit': catalog!.commit!,
        },
        action: FilledButton.tonal(
          key: const Key('entry-details-action'),
          onPressed: c.busy != null ? null : () => unawaited(_review(label, name, offer, fromDetails: true)),
          child: const Text('Review and install…'),
        ),
      ),
    ));
  }

  Future<void> _pasted() async {
    final GitOverlay source;
    try {
      source = OverlaySource.parseGit(spec.text.trim());
    } on ProfileException {
      c.problem = 'That is not <repository>@<ref>#<skill directory> of a source git may fetch';
      c.notifyListeners();
      return;
    }
    final name = source.path.split('/').where((part) => part.isNotEmpty).lastOrNull;
    if (name == null || !validSkillName(name)) {
      c.problem = 'Name the skill\'s directory after #, such as #skills/review: the skill\'s name is its directory\'s';
      c.notifyListeners();
      return;
    }
    c.problem = null;
    await _review(null, name, aos.CatalogEntry(source: aos.Source('git', source.repo, source.reference, source.path), pin: null, description: null, about: null));
  }

  /// Shows where the skill comes from and what it is pinned to, and fetches it only on a yes.
  Future<void> _review(String? label, String name, aos.CatalogEntry offer, {bool fromDetails = false}) async {
    final item = reviewChosenSkill(c.home!.overlays, c.skills, label, name, offer, pins: c.config.skillPins);
    final yes = await showSelectableDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Install the skill ${oneLine(name)}?'),
        content: SingleChildScrollView(child: SkillRows(items: [item], key: const Key('store-skill-review'))),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(
            key: const Key('store-skill-install-confirm'),
            onPressed: item.fetch == null ? null : () => Navigator.pop(context, true),
            child: Text(item.pin == null ? 'Fetch it unpinned and install' : 'Fetch it at its pin and install'),
          ),
        ],
      ),
    );
    if (yes != true) return;
    await c.installSkill(item);
    if (!mounted) return;
    // From a details page, back to the list first.
    if (fromDetails) Navigator.of(context).pop();
    view.discover = false;
  }
}
