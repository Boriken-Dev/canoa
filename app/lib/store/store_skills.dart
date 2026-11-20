import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../loadout/loadout_controller.dart';
import '../profile/overlays.dart' show ScannedSkill, SkillHit, SkillScan;
import '../profile/profile.dart';
import '../profile/profiles_panel.dart' show visible;
import '../profile/skill_copies.dart';
import '../selectable.dart';
import 'overlay_page.dart' show SkillRows, oneLine;
import 'skill_sources.dart';
import 'skills.dart';
import 'store_banner.dart';
import 'store_cards.dart';
import 'store_catalogs.dart' show confirmRemoval;
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
  final sourceText = TextEditingController();
  final searchText = TextEditingController();
  String? sourceProblem;

  LoadoutController get c => widget.controller;
  StoreView get view => widget.view;

  @override
  bool get wantKeepAlive => true;

  @override
  void dispose() {
    spec.dispose();
    sourceText.dispose();
    searchText.dispose();
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
    final inOverlays = c.overlaySkills();
    if (c.skills.isEmpty && inOverlays.isEmpty) {
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
    final shownInOverlays = [for (final each in inOverlays) if (view.matches(each.name)) each];
    return CardList(
      header: [
        ...head,
        if (shown.isEmpty && shownInOverlays.isEmpty) const Text('No installed skill matches.', key: Key('skills-installed-nothing')),
      ],
      cards: [
        for (final copy in shown) _installedCard(context, copy),
        for (final each in shownInOverlays) _overlayCard(each.name, each.overlay),
      ],
    );
  }

  /// A skill an installed overlay provides: it has no Remove of its own, since removing the overlay removes it.
  Widget _overlayCard(String name, String overlay) => LoadoutCard(
    key: Key('skill-installed-${skillKey(name)}'),
    leading: const LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined),
    title: name,
    lines: ['Comes with $overlay'],
    provenance: 'comes with the overlay $overlay; remove the overlay to remove it',
  );

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
        if (copy.catalog != null) 'from the catalog ${copy.catalog}' else 'from a source you named',
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

  /// Which place Discover reads: one of the sources that hold skills in git, or a search, and a field to add a source. Choosing
  /// a source is what fetches it.
  Widget _sourceRow(BuildContext context) {
    final sources = c.skillSources;
    final chosen = view.source;
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Wrap(spacing: 8, runSpacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [
        for (final label in c.generatedSkillSources)
          ChoiceChip(
            key: Key('skills-source-$label'),
            label: Text(label),
            tooltip: 'Read the skills ${_generatedBy(label)} lists',
            selected: chosen == label,
            onSelected: c.scanningSource != null ? null : (_) => unawaited(_choose(label)),
          ),
        for (final MapEntry(:key, :value) in sources.entries)
          ChoiceChip(
            key: Key('skills-source-$key'),
            label: Text(value.spec),
            tooltip: 'Read the skills in ${value.shown}',
            selected: chosen == key,
            onSelected: c.scanningSource != null ? null : (_) => unawaited(_choose(key)),
          ),
        if (c.skillsSearch case final address?)
          ChoiceChip(
            key: const Key('skills-from-search'),
            label: Text('Search ${Uri.tryParse(address)?.host ?? address}'),
            tooltip: 'Search the skills ${Uri.tryParse(address)?.host ?? address} lists',
            selected: chosen == skillsSearchSource,
            onSelected: (_) {
              view.source = skillsSearchSource;
              view.changed();
            },
          ),
        if (chosen != null && (sources.containsKey(chosen) || c.generatedSkillSources.contains(chosen)))
          TextButton(
            key: const Key('skills-source-remove'),
            onPressed: c.busy != null ? null : () => unawaited(_removeSource(chosen)),
            child: const Text('Remove this source'),
          ),
      ]),
      const SizedBox(height: 8),
      Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Expanded(
          child: TextField(
            key: const Key('skills-source-address'),
            controller: sourceText,
            decoration: InputDecoration(
              labelText: 'Add a source',
              hintText: 'owner/repo  or  https://github.com/owner/repo  or  a folder',
              errorText: sourceProblem,
            ),
            onSubmitted: (_) => unawaited(_addSource()),
          ),
        ),
        const SizedBox(width: 8),
        FilledButton.tonal(
          key: const Key('skills-source-add'),
          onPressed: c.busy != null ? null : () => unawaited(_addSource()),
          child: const Text('Add'),
        ),
      ]),
      const SizedBox(height: 12),
    ]);
  }

  Future<void> _choose(String label) async {
    view.source = label;
    view.changed();
    if (!c.skillScans.containsKey(label)) await c.scanSkillSource(label);
  }

  Future<void> _addSource() async {
    final problem = await c.addSkillSource(sourceText.text);
    if (!mounted) return;
    setState(() {
      sourceProblem = problem;
      if (problem == null) sourceText.clear();
    });
  }

  Future<void> _removeSource(String label) async {
    if (!await confirmRemoval(
      context,
      title: 'Stop reading $label?',
      body: 'It is taken out of canoa.cfg. The skills installed from it stay installed, and adding it again reads it once more.',
    )) {
      return;
    }
    if (!mounted) return;
    await c.removeSkillSource(label);
    if (view.source == label) view.source = null;
    view.changed();
  }

  /// The skills one source holds, read from its `SKILL.md` files at the commit named, each installable on a yes.
  Widget _fromSource(BuildContext context, List<Widget> head, String label) {
    final source = c.skillSources[label]!;
    final scan = c.skillScans[label];
    final text = Theme.of(context).textTheme;
    final own = c.skillSourceIsOwn(label);
    final problem = c.skillScanProblems[label];
    final held = [
      if (scan != null)
        for (final skill in scan.skills)
          if (source.only == null || skillKey(skill.name) == skillKey(source.only!)) skill,
    ];
    final shown = [for (final skill in held) if (view.matches('${skill.name}\n${skill.description}')) skill];
    return CardList(
      header: [
        ...head,
        _sourceRow(context),
        Text(source.shown, key: const Key('skills-source-shown'), style: text.titleSmall),
        Text(
          own
              ? 'You typed this source, so it is fetched with your own git settings, credentials and keys. Reading it fetches its '
                  'repository; nothing is installed until you say yes to a skill.'
              : 'This source is fetched with none of your git settings, credentials or keys. Reading it fetches its repository; '
                  'nothing is installed until you say yes to a skill.',
          key: const Key('skills-source-note'),
          style: text.bodySmall,
        ),
        const SizedBox(height: 8),
        if (c.scanningSource == label) const LinearProgressIndicator(),
        if (problem != null)
          Text(visible(problem), key: const Key('skills-source-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        if (scan != null) ...[
          Row(children: [
            Text(view.query.text.isEmpty ? 'Skills in this source' : 'Matching skills', style: text.titleMedium),
            const SizedBox(width: 8),
            Chip(key: const Key('store-count'), label: Text('${shown.length}'), visualDensity: VisualDensity.compact),
            const Spacer(),
            TextButton(
              key: const Key('skills-source-reread'),
              onPressed: c.scanningSource != null ? null : () => unawaited(c.scanSkillSource(label)),
              child: const Text('Read again'),
            ),
          ]),
          Text('Read at commit ${scan.commit}', key: const Key('skills-source-commit'), style: text.bodySmall),
          if (scan.skipped > 0 || scan.truncated)
            Text(
              [
                if (scan.skipped > 0)
                  '${scan.skipped} SKILL.md left out: no front matter naming and describing it, marked internal, or a name already listed.',
                if (scan.truncated) 'The source holds more skills than are listed.',
              ].join(' '),
              key: const Key('skills-source-left-out'),
              style: text.bodySmall,
            ),
          const SizedBox(height: 8),
          if (held.isEmpty)
            const Text('This source holds no skill: Canoa looks for SKILL.md files the way npx skills does.', key: Key('skills-source-none'))
          else if (shown.isEmpty)
            const Text('Nothing listed matches.', key: Key('skills-source-nothing-matches')),
        ] else if (problem == null && c.scanningSource != label)
          const Text('Choose the source to read which skills it holds.', key: Key('skills-source-unread')),
      ],
      cards: [for (final skill in shown) _scannedTile(label, source, scan!, skill, own)],
    );
  }

  /// What reads the generated catalog [label], in the person's words: the program, or the extension's source.
  String _generatedBy(String label) {
    final spec = c.config.catalogSpecs('skills')[label];
    final titled = spec == null ? null : c.typeOf('skills')?.sources.where((each) => each.id == spec.provider).firstOrNull?.title;
    return 'read by ${titled ?? spec?.provider ?? 'a source'} lists';
  }

  /// The skills a program or an extension lists for [label]: each names where it is, and installing one reads that source, shows
  /// what was found and asks first.
  Widget _fromGenerated(BuildContext context, List<Widget> head, String label) {
    final scan = c.skillScans[label];
    final text = Theme.of(context).textTheme;
    final problem = c.skillScanProblems[label];
    final shown = [if (scan != null) for (final skill in scan.skills) if (view.matches('${skill.name}\n${skill.description}')) skill];
    return CardList(
      header: [
        ...head,
        _sourceRow(context),
        Text(
          'This list is made by ${_generatedBy(label).replaceFirst(' lists', '')} each time you read it. It only says where each skill is: '
          'installing one reads that source with none of your git settings, credentials or keys, shows what it holds and asks first.',
          key: const Key('skills-source-note'),
          style: text.bodySmall,
        ),
        const SizedBox(height: 8),
        if (c.scanningSource == label) const LinearProgressIndicator(),
        if (problem != null)
          Text(visible(problem), key: const Key('skills-source-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        if (scan != null) ...[
          Row(children: [
            Text(view.query.text.isEmpty ? 'Skills in this list' : 'Matching skills', style: text.titleMedium),
            const SizedBox(width: 8),
            Chip(key: const Key('store-count'), label: Text('${shown.length}'), visualDensity: VisualDensity.compact),
            const Spacer(),
            TextButton(
              key: const Key('skills-source-reread'),
              onPressed: c.scanningSource != null ? null : () => unawaited(c.scanSkillSource(label)),
              child: const Text('Read again'),
            ),
          ]),
          if (scan.skipped > 0 || scan.truncated)
            Text(
              [
                if (scan.skipped > 0) '${scan.skipped} entries left out: no name or source, or a name already listed.',
                if (scan.truncated) 'The list names more skills than are shown.',
              ].join(' '),
              key: const Key('skills-source-left-out'),
              style: text.bodySmall,
            ),
          const SizedBox(height: 8),
          if (scan.skills.isEmpty) const Text('This list names no skill.', key: Key('skills-source-none')),
        ] else if (problem == null && c.scanningSource != label)
          const Text('Choose the source to read which skills it lists.', key: Key('skills-source-unread')),
      ],
      cards: [for (final skill in shown) _listedTile(label, skill)],
    );
  }

  Widget _listedTile(String label, ScannedSkill skill) {
    final here = c.skills[skillKey(skill.name)];
    return StoreTile(
      key: Key('store-source-skill-$label-${skill.name}'),
      leading: const SizedBox.square(dimension: 48, child: Center(child: LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined))),
      title: skill.name,
      summary: skill.problem == null ? skill.description : '${skill.description}\nCannot be installed: ${skill.problem}',
      by: '${skill.source ?? 'no source'}${skill.path.isEmpty ? '' : ' · ${skill.path}'}',
      installed: here != null,
      installKey: Key('store-source-skill-install-$label-${skill.name}'),
      onOpen: () => unawaited(_installListed(skill)),
      onInstall: skill.problem != null || c.busy != null || c.resolvingHit != null ? null : () => unawaited(_installListed(skill)),
    );
  }

  /// Reads the source a listing names for [skill], finds the skill in it and reviews it for install.
  Future<void> _installListed(ScannedSkill skill) async {
    if (skill.problem != null) return;
    final found = await c.resolveListedSkill(skill);
    if (found == null || !mounted) return;
    if (found.skill.problem != null) {
      c.problem = '${skill.name} cannot be installed: ${found.skill.problem}';
      c.notifyListeners();
      return;
    }
    await _reviewScanned(found.source, found.scan, found.skill, false);
  }

  Widget _scannedTile(String label, SkillSource source, SkillScan scan, ScannedSkill skill, bool own) {
    final here = c.skills[skillKey(skill.name)];
    return StoreTile(
      key: Key('store-source-skill-$label-${skill.name}'),
      leading: const SizedBox.square(dimension: 48, child: Center(child: LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined))),
      title: skill.name,
      summary: skill.problem == null ? skill.description : '${skill.description}\nCannot be installed: ${skill.problem}',
      by: '${skill.path.isEmpty ? 'the whole source' : skill.path} · ${source.spec}',
      installed: here != null,
      installKey: Key('store-source-skill-install-$label-${skill.name}'),
      onOpen: () => unawaited(_reviewScanned(source, scan, skill, own)),
      onInstall: skill.problem != null || c.busy != null ? null : () => unawaited(_reviewScanned(source, scan, skill, own)),
    );
  }

  /// A skill of a source is installed from the commit that was read, so that what is fetched is what was listed.
  Future<void> _reviewScanned(SkillSource source, SkillScan scan, ScannedSkill skill, bool own) async {
    if (skill.problem != null) return;
    await _review(
      null,
      skill.name,
      aos.CatalogEntry(
        source: aos.Source('git', source.repo, scan.commit, skill.path.isEmpty ? null : skill.path),
        pin: null,
        description: skill.description,
        about: null,
      ),
      own: own,
    );
  }

  /// The skills a directory lists for what the person types. The directory is a third party: what it returns is only which
  /// repository holds a skill, and installing one reads the repository itself, as for a source, and asks first.
  Widget _fromSearch(BuildContext context, List<Widget> head) {
    final text = Theme.of(context).textTheme;
    final host = Uri.tryParse(c.skillsSearch ?? '')?.host ?? 'the directory';
    final problem = c.skillSearchProblem;
    final searched = c.skillSearchedFor;
    return CardList(
      header: [
        ...head,
        _sourceRow(context),
        Text(
          'Only the words you search for are sent to $host, a third party. A result names a repository; nothing is installed until '
          'you say yes to the skill, after Canoa has read the repository itself.',
          key: const Key('skills-search-note'),
          style: text.bodySmall,
        ),
        const SizedBox(height: 8),
        Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Expanded(
            child: TextField(
              key: const Key('skills-search-query'),
              controller: searchText,
              decoration: InputDecoration(labelText: 'Search $host', hintText: 'pdf, web performance, ...'),
              onSubmitted: (_) => unawaited(c.searchSkills(searchText.text)),
            ),
          ),
          const SizedBox(width: 8),
          FilledButton.tonal(
            key: const Key('skills-search-go'),
            onPressed: c.searchingSkills ? null : () => unawaited(c.searchSkills(searchText.text)),
            child: const Text('Search'),
          ),
        ]),
        const SizedBox(height: 8),
        if (c.searchingSkills || c.resolvingHit != null) const LinearProgressIndicator(),
        if (problem != null)
          Text(visible(problem), key: const Key('skills-search-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        if (searched != null) ...[
          Row(children: [
            Text('Skills for "${oneLine(searched)}"', style: text.titleMedium),
            const SizedBox(width: 8),
            Chip(key: const Key('store-count'), label: Text('${c.skillHits.length}'), visualDensity: VisualDensity.compact),
          ]),
          if (c.skillHits.isEmpty) const Text('Nothing was found.', key: Key('skills-search-nothing')),
        ] else if (problem == null)
          const Text('Type what the skill is for, then Search.', key: Key('skills-search-unsearched')),
      ],
      cards: [for (final hit in c.skillHits) _hitTile(hit)],
    );
  }

  Widget _hitTile(SkillHit hit) => StoreTile(
    key: Key('store-search-skill-${hit.source}/${hit.skillId}'),
    leading: const SizedBox.square(dimension: 48, child: Center(child: LoadoutMark(name: 'skill', icon: Icons.auto_stories_outlined))),
    title: hit.name,
    summary: hit.id,
    by: '${_installs(hit.installs)} · ${hit.source}',
    installed: c.skills.containsKey(skillKey(hit.skillId)),
    installKey: Key('store-search-skill-install-${hit.source}/${hit.skillId}'),
    onOpen: () => unawaited(_installHit(hit)),
    onInstall: c.busy != null || c.resolvingHit != null ? null : () => unawaited(_installHit(hit)),
  );

  static String _installs(int installs) => installs == 1 ? '1 install' : '$installs installs';

  /// Reads the hit's repository as any source is read, finds the skill in it, and reviews it for install.
  Future<void> _installHit(SkillHit hit) async {
    final found = await c.resolveSkillHit(hit);
    if (found == null || !mounted) return;
    if (found.skill.problem != null) {
      c.skillSearchProblem = '${hit.id} cannot be installed: ${found.skill.problem}';
      c.notifyListeners();
      return;
    }
    await _reviewScanned(found.source, found.scan, found.skill, false);
  }

  Widget _discover(BuildContext context, List<Widget> head) {
    final chosenSource = view.source;
    if (chosenSource == skillsSearchSource && c.skillsSearch != null) return _fromSearch(context, head);
    if (chosenSource != null && c.skillSources.containsKey(chosenSource)) return _fromSource(context, head, chosenSource);
    if (chosenSource != null && c.generatedSkillSources.contains(chosenSource)) return _fromGenerated(context, head, chosenSource);
    return CardList(
      header: [
        ...head,
        _sourceRow(context),
        // A skill an overlay names is installed with its overlay, which knows what it needs: it is not offered alone here.
        const Text(
          'Choose a source to read which skills it holds. A skill that comes in an overlay is installed with the overlay, from Overlays.',
          key: Key('skills-choose-source'),
        ),
      ],
      cards: const [],
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
  Future<void> _review(String? label, String name, aos.CatalogEntry offer, {bool own = false}) async {
    final item = reviewChosenSkill(c.home!.overlays, c.skills, label, name, offer, pins: c.config.skillPins, own: own);
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
    view.discover = false;
  }
}
