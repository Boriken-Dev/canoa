import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../profile/profiles_panel.dart' show visible;
import '../surfaces/a2ui/overlay_views.dart';
import 'links.dart';
import 'contents_store.dart';
import 'overlay_contents.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart' show EntryFacts;

/// What an entry's index facts say it holds, shown before anything is fetched: its contents, what it requires, the hooks and
/// environment scripts it carries, and the technical details with their digests. All of it is what the entry's catalog or
/// manifest says; nothing here ran.
class EntryFactsView extends StatelessWidget {
  const EntryFactsView({
    super.key,
    required this.facts,
    this.requirements = const [],
    this.technical = const {},
    this.read,
    this.reading = false,
    this.overlay,
    this.onOpenView,
  });

  /// The overlay's name, which its views are drawn under.
  final String? overlay;

  /// Draws one of the overlay's views in the open conversation; null where there is none to draw in.
  final OpenOverlayView? onOpenView;

  /// What reading the entry ahead found of what it provides; null before anything has been read.
  final EntryContents? read;

  /// Whether the entry is being read now.
  final bool reading;

  /// The index's facts of the entry, or null when the host has none (an entry listed by hand).
  final EntryFacts? facts;

  /// Requirements from another source than the manifest, such as the runtime a registry package needs.
  final List<String> requirements;

  /// Details for someone who checks: label to value, each shown whole.
  final Map<String, String> technical;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final facts = this.facts;
    final provided = read?.contents;
    final needs = [...?facts?.requirements, ...requirements, ...?provided?.requirements];
    if (provided != null) return _provided(context, provided, needs);
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Contents', style: text.titleSmall),
      if (read?.problem case final problem?)
        Text('It could not be read: ${oneLine(problem)}', key: const Key('details-contents-failed'),
            style: TextStyle(color: Theme.of(context).colorScheme.error))
      else if (reading)
        const Text('Reading what it provides…', key: Key('details-contents-reading')),
      if (read?.problem != null || reading)
        const SizedBox.shrink()
      else if (facts == null || !facts.hasContents)
        const Text(
          'Not read yet. It is read with the store\'s catalog, and the refresh reads it again.',
          key: Key('details-contents-unread'),
        )
      else ...[
        _list('Skills', facts.skills, const Key('details-skills')),
        _list('MCP servers (off until allowed)', facts.mcpServers, const Key('details-mcp-servers')),
        if (facts.skills.isEmpty && facts.mcpServers.isEmpty) const Text('No skills and no MCP servers.'),
      ],
      const SizedBox(height: 12),
      Text('Requirements', style: text.titleSmall),
      if (needs.isEmpty)
        const Text('None listed.', key: Key('details-requirements-none'))
      else
        for (final need in needs) Text(oneLine(need), key: const Key('details-requirement')),
      const SizedBox(height: 12),
      Text('Hooks and environment scripts it carries', style: text.titleSmall),
      if (facts == null || !facts.hasContents)
        const Text('Not read yet.', key: Key('details-carried-unread'))
      else if (facts.hooks.isEmpty && facts.envScripts.isEmpty)
        const Text('None.', key: Key('details-carried-none'))
      else ...[
        _list('Hooks, by event', facts.hooks, const Key('details-hooks')),
        _list('Environment scripts', facts.envScripts, const Key('details-env-scripts')),
        const Text('They stay off until you allow them at their digest.', key: Key('details-carried-off')),
      ],
      if (facts?.problem case final problem?) ...[
        const SizedBox(height: 12),
        Text(oneLine(problem), key: const Key('details-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
      ],
      if (technical.isNotEmpty || facts != null) ...[
        const SizedBox(height: 4),
        Theme(
          data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            key: const Key('details-technical'),
            tilePadding: EdgeInsets.zero,
            childrenPadding: const EdgeInsets.only(bottom: 8),
            expandedCrossAxisAlignment: CrossAxisAlignment.start,
            title: Text('Technical details', style: text.titleSmall),
            children: [
              if (facts != null) _field('Index id', facts.id),
              if (facts?.version case final version?) _field('Version', version),
              for (final MapEntry(:key, :value) in technical.entries) _field(key, value),
            ],
          ),
        ),
      ],
    ]);
  }

  Widget _provided(BuildContext context, OverlayContents provided, List<String> needs) {
    final text = Theme.of(context).textTheme;
    Widget section(String title, List<Widget> rows, Key key) => rows.isEmpty
        ? const SizedBox.shrink()
        : Padding(
            key: key,
            padding: const EdgeInsets.only(top: 12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [Text(title, style: text.titleSmall), ...rows]),
          );
    Widget line(String value, {Key? key}) =>
        Padding(padding: const EdgeInsets.only(top: 2), child: SelectableText(oneLine(value), key: key));
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('What it provides', style: text.titleSmall),
      Text('From its own ${provided.generated ? 'files, with an overlay.cfg Canoa generated' : 'overlay.cfg and files'}. '
          'Nothing of it has run.', key: const Key('details-provided-note')),
      if (provided.isEmpty) const Text('It provides nothing Canoa lists.', key: Key('details-provided-nothing')),
      section('Skills', [
        for (final skill in provided.skills)
          line(skill.description == null ? skill.name : '${skill.name}: ${skill.description}', key: Key('details-skill-${skill.name}')),
      ], const Key('details-skills')),
      section('Rules', [for (final path in provided.rules) line(path)], const Key('details-rules')),
      section('Context', [for (final path in provided.context) line(path)], const Key('details-context')),
      section('Knowledge', [
        for (final file in provided.knowledge) line('${file.path}: ${file.served ? 'served on request' : 'put in the context'}'),
      ], const Key('details-knowledge')),
      section('Hooks (off until you allow them at their digest)', [for (final hook in provided.hooks) line(hook)], const Key('details-hooks')),
      section('Environment scripts and setup', [
        for (final script in provided.envScripts) line('env script $script'),
        for (final script in provided.setup) line('setup $script'),
      ], const Key('details-env-scripts')),
      section('MCP servers (off until allowed)', [for (final server in provided.mcpServers) line(server)], const Key('details-mcp-servers')),
      section('Commands on its PATH', [for (final command in provided.bin) line(command)], const Key('details-bin')),
      _views(context, provided),
      const SizedBox(height: 12),
      Text('Requirements', style: text.titleSmall),
      if (needs.isEmpty)
        const Text('None listed.', key: Key('details-requirements-none'))
      else
        for (final need in needs) Text(oneLine(need), key: const Key('details-requirement')),
      if (provided.issues.isNotEmpty)
        section('Fields of its manifest that are not valid, and left out', [for (final issue in provided.issues) line(issue)],
            const Key('details-issues')),
      if (provided.generated && provided.manifest != null)
        Theme(
          data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            key: const Key('details-generated'),
            tilePadding: EdgeInsets.zero,
            expandedCrossAxisAlignment: CrossAxisAlignment.start,
            title: Text('The overlay.cfg generated for it', style: text.titleSmall),
            children: [SelectableText(provided.manifest!, style: const TextStyle(fontFamily: 'monospace'))],
          ),
        ),
      if (provided.digest != null)
        Theme(
          data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            key: const Key('details-technical'),
            tilePadding: EdgeInsets.zero,
            childrenPadding: const EdgeInsets.only(bottom: 8),
            expandedCrossAxisAlignment: CrossAxisAlignment.start,
            title: Text('Technical details', style: text.titleSmall),
            children: [
              if (facts != null) _field('Index id', facts!.id),
              _field('Tree digest read', provided.digest!),
              for (final MapEntry(:key, :value) in technical.entries) _field(key, value),
            ],
          ),
        ),
    ]);
  }

  /// The views the overlay ships: each with why it cannot be drawn, or a button that draws it in the open conversation.
  Widget _views(BuildContext context, OverlayContents provided) {
    final shipped = readOverlayViews(provided.views);
    if (shipped.isEmpty) return const SizedBox.shrink();
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      key: const Key('details-views'),
      padding: const EdgeInsets.only(top: 12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('store.views'), style: text.titleSmall),
        for (final problem in shipped.problems) Text(oneLine(problem), style: TextStyle(color: scheme.error)),
        for (final view in shipped.views)
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Row(children: [
              Expanded(
                child: view.drawable
                    ? SelectableText(oneLine(view.title))
                    : SelectableText(oneLine('${view.title}: ${view.problems.take(3).join('; ')}'),
                        key: Key('details-view-problem-${view.name}'), style: TextStyle(color: scheme.error)),
              ),
              if (view.drawable && onOpenView != null && overlay != null)
                TextButton(
                  key: Key('details-view-open-${view.name}'),
                  onPressed: () {
                    final why = onOpenView!(overlay!, view, shipped);
                    if (why.isNotEmpty) {
                      ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(oneLine(why.first))));
                    }
                  },
                  child: Text(context.t('store.view_open')),
                ),
            ]),
          ),
      ]),
    );
  }

  static Widget _list(String label, List<String> names, Key key) => names.isEmpty
      ? const SizedBox.shrink()
      : Padding(
          padding: const EdgeInsets.only(top: 2),
          child: SelectableText.rich(key: key, TextSpan(children: [
            TextSpan(text: '$label: ', style: const TextStyle(fontWeight: FontWeight.w600)),
            TextSpan(text: names.map(oneLine).join(', ')),
          ])),
        );

  static Widget _field(String label, String value) => Padding(
    padding: const EdgeInsets.only(top: 4),
    child: SelectableText.rich(TextSpan(children: [
      TextSpan(text: '$label: ', style: const TextStyle(fontWeight: FontWeight.w600)),
      TextSpan(text: oneLine(value)),
    ])),
  );
}

/// One entry's page, for a skill or an MCP server: what its catalog or registry says, where it comes from, what it holds, and
/// the one labelled action that goes on to the review before anything is installed. An overlay's own page is
/// `OverlayPage`, which shows the same facts and goes on to its dry run.
class EntryDetails extends StatelessWidget {
  const EntryDetails({
    super.key,
    required this.title,
    required this.leading,
    required this.from,
    this.summary,
    this.state,
    this.overview = const {},
    this.source = const {},
    this.facts,
    this.requirements = const [],
    this.technical = const {},
    this.contents = true,
    this.sections = const [],
    this.action,
  });

  final String title;
  final Widget leading;

  /// Whose claim this is, such as `the catalog main` or `the registry registry.example`.
  final String from;
  final String? summary;

  /// `Installed`, or null.
  final String? state;

  /// Overview fields: label to value. A Homepage or Repository that is a plain web address opens when tapped.
  final Map<String, String> overview;

  /// Source and version fields: label to value.
  final Map<String, String> source;
  final EntryFacts? facts;
  final List<String> requirements;
  final Map<String, String> technical;

  /// Whether the page lists what the entry provides and requires, as a skill's or a server's does. An entry that provides
  /// nothing of the kind (a runtime) turns it off and says what it has in [sections].
  final bool contents;

  /// Further sections, after the source and version, each laid out as the page's own: a title in `titleSmall` and its lines.
  final List<Widget> sections;

  /// The labelled button that goes on, such as `Review and install…`; null when nothing can be done.
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Scaffold(
      appBar: AppBar(title: Text(oneLine(title))),
      body: ListView(
        key: const Key('entry-details'),
        padding: const EdgeInsets.all(16),
        children: [
          Row(children: [
            leading,
            const SizedBox(width: 12),
            Expanded(child: Text(oneLine(title), style: text.titleLarge)),
            if (state != null) Chip(key: const Key('details-state'), label: Text(state!)),
          ]),
          const SizedBox(height: 12),
          Text('Overview', style: text.titleSmall),
          Text('What $from says', style: text.bodySmall),
          if (summary != null) Padding(padding: const EdgeInsets.only(top: 4), child: SelectableText(visible(summary!))),
          for (final MapEntry(:key, :value) in overview.entries) DetailField(label: key, value: value),
          const SizedBox(height: 12),
          Text('Source and version', style: text.titleSmall),
          for (final MapEntry(:key, :value) in source.entries) DetailField(label: key, value: value),
          const SizedBox(height: 12),
          if (contents) EntryFactsView(facts: facts, requirements: requirements, technical: technical),
          ...sections,
          if (action != null) ...[const SizedBox(height: 16), Align(alignment: Alignment.centerLeft, child: action)],
        ],
      ),
    );
  }
}

/// A labelled line of an entry's page: the label in bold and the value as one line of text, or, for a Homepage or Repository that is
/// a plain web address, a link that opens when tapped.
class DetailField extends StatelessWidget {
  const DetailField({super.key, required this.label, required this.value});

  final String label, value;

  @override
  Widget build(BuildContext context) {
    final uri = label == 'Homepage' || label == 'Repository' ? webLink(value) : null;
    if (uri != null) return LinkField(key: Key('details-link-${label.toLowerCase()}'), label: label, uri: uri, shown: oneLine(value));
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: SelectableText.rich(TextSpan(children: [
        TextSpan(text: '$label: ', style: const TextStyle(fontWeight: FontWeight.w600)),
        TextSpan(text: oneLine(value)),
      ])),
    );
  }
}

/// A titled section of an entry's page, in the page's own spacing: [title] as the other sections write theirs, then [children].
class DetailSection extends StatelessWidget {
  const DetailSection({super.key, required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 12),
    child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text(title, style: Theme.of(context).textTheme.titleSmall),
      ...children,
    ]),
  );
}
