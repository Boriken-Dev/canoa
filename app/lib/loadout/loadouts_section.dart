/// The Loadouts section's main view: the loadout chosen in its index, shown with what it holds and how to change it, or the editor
/// of one. A loadout is a named collection of whole overlays, chosen skills and servers a session starts with; the Store is where
/// those are found and installed, and the editor's picker leads there without losing what has been typed.
library;

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../brand.dart';
import '../config/canoa_config.dart';
import '../profile/profile_home.dart';
import '../shell/shell.dart' show ShellScope;
import '../store/overlay_page.dart' show SaveConfig;
import 'loadout_editor.dart';
import 'loadout_page.dart';
import 'loadouts_index.dart';

class LoadoutsSection extends StatefulWidget {
  const LoadoutsSection({
    super.key,
    required this.config,
    required this.home,
    required this.onSave,
    required this.selection,
    this.onStore,
    this.onUse,
    this.agents = const [],
  });

  final CanoaConfig config;

  /// Where fetched skills are read; none in a web build, which lists none.
  final ProfileHome? home;
  final SaveConfig onSave;

  /// Opens the Store; the editor stays as it is.
  final VoidCallback? onStore;

  /// Starts a new session with the loadout of that name.
  final void Function(String name)? onUse;
  final List<String> agents;

  /// The loadout chosen in the index and the one being edited; the owner keeps it so that both outlive this page.
  final LoadoutsSelection selection;

  @override
  State<LoadoutsSection> createState() => LoadoutsSectionState();
}

class LoadoutsSectionState extends State<LoadoutsSection> {
  ValueNotifier<LoadoutDraft?> get _editing => widget.selection.draft;

  Set<String> _skills = const {};

  /// Whether the skills here have been read; until then, and where none can be read, a skill is not called missing.
  bool _skillsKnown = false;
  String? _problem;

  @override
  void initState() {
    super.initState();
    unawaited(_readSkills());
  }

  @override
  void didUpdateWidget(LoadoutsSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    // What the Store installed shows when the person comes back.
    if (oldWidget.config != widget.config || oldWidget.home != widget.home) unawaited(_readSkills());
  }

  Future<void> _readSkills() async {
    final home = widget.home;
    if (home == null) return;
    final copies = await home.overlays.skillCopies();
    if (!mounted) return;
    setState(() {
      _skills = {for (final copy in copies.values) if (copy.chosen) copy.name};
      _skillsKnown = true;
    });
  }

  CanoaConfig get _config => widget.config;

  Set<String> get _servers => {for (final MapEntry(:key, :value) in _config.profile.mcp.entries) if (value != null) key};

  Future<void> _save() async {
    final draft = _editing.value;
    if (draft == null) return;
    if (draft.problemWith(_config.loadouts.keys) case final why?) {
      setState(() => _problem = why);
      return;
    }
    final name = draft.name;
    final all = <String, Loadout>{};
    var placed = false;
    for (final MapEntry(:key, :value) in _config.loadouts.entries) {
      if (key == draft.original) {
        all[name] = draft.toLoadout();
        placed = true;
      } else {
        all[key] = value;
      }
    }
    if (!placed) all[name] = draft.toLoadout();
    final renamedDefault = draft.original != null && draft.original == _config.defaultLoadout ? name : null;
    final CanoaConfig changed;
    try {
      changed = _config.withLoadouts(
        all,
        defaultName: renamedDefault,
        renamed: draft.original == null || draft.original == name ? const {} : {draft.original!: name},
      );
    } on FormatException catch (error) {
      setState(() => _problem = error.message);
      return;
    }
    final saved = await widget.onSave(changed);
    if (!mounted) return;
    setState(() => _problem = saved == null ? '${Brand.configFile} could not be written' : null);
    if (saved != null) {
      _editing.value = null;
      widget.selection.select(name);
    }
  }

  Future<void> _delete(String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete $name?'),
        content: const Text('New sessions stop offering it. Sessions already running keep what they started with.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('loadout-delete-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Delete')),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    // The next in the list takes its place, else the one before it.
    final names = _config.loadouts.keys.toList();
    final at = names.indexOf(name);
    final next = at + 1 < names.length ? names[at + 1] : (at > 0 ? names[at - 1] : null);
    final wasShown = widget.selection.selectedIn(_config) == name;
    final saved = await widget.onSave(_config.withLoadouts({..._config.loadouts}..remove(name)));
    if (!mounted) return;
    if (saved != null && wasShown && next != null) widget.selection.select(next);
    setState(() => _problem = saved == null ? '${Brand.configFile} could not be written' : null);
  }

  Future<void> _makeDefault(String name) async {
    final saved = await widget.onSave(_config.withLoadouts(_config.loadouts, defaultName: name));
    if (mounted) setState(() => _problem = saved == null ? '${Brand.configFile} could not be written' : null);
  }

  void _open(LoadoutDraft draft) {
    setState(() => _problem = null);
    _editing.value = draft;
  }

  Future<void> _add(LoadoutGroup group) async {
    final draft = _editing.value;
    if (draft == null) return;
    final (title, held, offered) = switch (group) {
      LoadoutGroup.overlays => (
        'Add an overlay',
        draft.overlays,
        {for (final name in _config.overlays.keys) name: 'Applies whole'},
      ),
      LoadoutGroup.skills => ('Add a skill', draft.skills, {for (final name in _skills) name: 'Chosen skill'}),
      LoadoutGroup.mcp => ('Add a server', draft.mcp, {for (final name in _servers) name: 'In your profile'}),
    };
    final answer = await pickInstalled(
      context,
      title: title,
      items: {for (final MapEntry(:key, :value) in offered.entries) if (!held.contains(key)) key: value},
    );
    if (answer == null || !mounted) return;
    held.addAll(answer.chosen);
    draft.changed();
    if (answer.toStore) widget.onStore?.call();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.selection,
    builder: (context, _) => ValueListenableBuilder<LoadoutDraft?>(
      valueListenable: _editing,
      builder: (context, draft, _) {
        // On a narrow window the index is in the drawer, which this button opens.
        final menu = ShellScope.menuButton(context);
        return Column(children: [
          if (menu != null) Align(alignment: Alignment.centerLeft, child: menu),
          Expanded(
            child: Align(
              alignment: Alignment.topCenter,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 760),
                child: draft != null ? _editor(draft) : _shown(context),
              ),
            ),
          ),
        ]);
      },
    ),
  );

  Widget _editor(LoadoutDraft draft) => LoadoutEditor(
    draft: draft,
    config: _config,
    skills: _skillsKnown ? _skills : draft.skills.toSet(),
    agents: widget.agents,
    problem: _problem,
    onAdd: (group) => unawaited(_add(group)),
    onSave: () => unawaited(_save()),
    onCancel: () => _editing.value = null,
  );

  /// The loadout chosen in the index, or what to do when there is none.
  Widget _shown(BuildContext context) {
    final name = widget.selection.selectedIn(_config);
    if (name == null) {
      return Padding(
        padding: const EdgeInsets.all(16),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('Loadouts', style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 4),
          Text(
            'A loadout is what a session starts with: whole overlays, skills you chose and MCP servers. '
            'Editing one reaches new sessions only.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
          const SizedBox(height: 12),
          const Text('No loadouts yet. New sessions start with every overlay and every server.', key: Key('loadouts-empty-page')),
          const SizedBox(height: 12),
          FilledButton.icon(
            key: const Key('loadout-new-page'),
            onPressed: widget.selection.startNew,
            icon: const Icon(Icons.add),
            label: const Text('New loadout'),
          ),
        ]),
      );
    }
    final loadout = _config.loadouts[name]!;
    return LoadoutPage(
      name: name,
      config: _config,
      skills: _skills,
      skillsKnown: _skillsKnown,
      agents: widget.agents,
      problem: _problem,
      onEdit: () => _open(LoadoutDraft.of(name, loadout)),
      onDuplicate: () => _open(LoadoutDraft.of(name, loadout, copy: true)),
      onUse: widget.onUse == null ? null : () => widget.onUse!(name),
      onMakeDefault: () => unawaited(_makeDefault(name)),
      onDelete: () => unawaited(_delete(name)),
    );
  }
}
