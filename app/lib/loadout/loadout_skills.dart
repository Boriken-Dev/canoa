import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../profile/profile.dart';
import '../profile/profiles_panel.dart' show visible;
import '../profile/skill_copies.dart';
import '../selectable.dart';
import '../store/skills.dart';
import '../store/overlay_page.dart' show SkillRows, oneLine;
import 'loadout_controller.dart';
import 'loadout_page.dart';

/// The Skills section: the skills fetched on this computer, and the skills the catalogs offer.
///
/// A skill is an Agent Skills directory (agentskills.io): a `SKILL.md` with YAML front matter naming it, and whatever it
/// refers to. One is fetched from its git source only after its source and pin are shown and the person says yes, its files
/// are checked against the pin before any is read, and nothing it holds runs on being installed.
class SkillsSection extends StatefulWidget {
  const SkillsSection({super.key, required this.controller});
  final LoadoutController controller;

  @override
  State<SkillsSection> createState() => _SkillsSectionState();
}

class _SkillsSectionState extends State<SkillsSection> with AutomaticKeepAliveClientMixin {
  bool discover = false;
  final query = TextEditingController();
  final spec = TextEditingController();

  LoadoutController get c => widget.controller;

  @override
  bool get wantKeepAlive => true;

  @override
  void dispose() {
    query.dispose();
    spec.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) {
        final offered = c.skillsOffered();
        return ListView(padding: const EdgeInsets.all(16), children: [
          Align(
            alignment: Alignment.centerLeft,
            child: InstalledOrDiscover(
              section: 'skills',
              discover: discover,
              installedCount: c.skills.length,
              discoverCount: c.home == null ? null : offered.length,
              onChanged: (value) => setState(() => discover = value),
            ),
          ),
          const SizedBox(height: 12),
          if (c.problem != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(visible(c.problem!), key: const Key('skills-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          if (c.home == null)
            const Text('The desktop app fetches skills.')
          else if (discover)
            ..._discover(context, offered)
          else
            ..._installed(context),
        ]);
      },
    );
  }

  List<Widget> _installed(BuildContext context) {
    if (c.skills.isEmpty) {
      return [
        const Text('No skills fetched yet. A skill an overlay names is fetched when you install the overlay; others you choose '
            'yourself in Discover.', key: Key('skills-none')),
        const SizedBox(height: 8),
        FilledButton.tonal(key: const Key('skills-to-discover'), onPressed: () => setState(() => discover = true), child: const Text('Discover skills')),
      ];
    }
    return [for (final copy in c.skills.values) _installedCard(context, copy)];
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

  List<Widget> _discover(BuildContext context, List<(String, String, aos.CatalogEntry)> offered) {
    final words = query.text.toLowerCase().split(RegExp(r'\s+')).where((word) => word.isNotEmpty);
    final shown = [
      for (final item in offered)
        if (words.every('${item.$2}\n${item.$1}\n${item.$3.description ?? ''}\n${item.$3.source.shown()}'.toLowerCase().contains)) item,
    ];
    return [
      TextField(
        key: const Key('skills-search'),
        controller: query,
        decoration: const InputDecoration(prefixIcon: Icon(Icons.search), labelText: 'Search skills'),
        onChanged: (_) => setState(() {}),
      ),
      const SizedBox(height: 8),
      if (c.loadingCatalogs) const LinearProgressIndicator(),
      if (offered.isEmpty && !c.loadingCatalogs)
        Text(c.config.catalogs.isEmpty
            ? 'No catalogs yet, so nothing is offered. Add one in Overlays, Discover, or paste a skill below.'
            : 'None of your catalogs offers a skill that is not here already. Paste one below.',
            key: const Key('skills-nothing')),
      for (final (label, name, offer) in shown)
        LoadoutCard(
          key: Key('store-offered-skill-$label-$name'),
          leading: const LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined),
          title: name,
          lines: [?offer.description],
          provenance: 'from the catalog $label · ${offer.source.shown()} · ${offer.pin ?? 'unpinned'}',
          trailing: FilledButton.tonal(
            key: Key('store-skill-install-$label-$name'),
            onPressed: c.busy != null ? null : () => unawaited(_review(label, name, offer)),
            child: const Text('Install…'),
          ),
        ),
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
    ];
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
  Future<void> _review(String? label, String name, aos.CatalogEntry offer) async {
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
    if (mounted) setState(() => discover = false);
  }
}
