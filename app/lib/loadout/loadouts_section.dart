/// The Loadouts section: the loadouts canoa.cfg names, each with what it holds and whether it can start, and the editor of one.
/// A loadout is a named collection of whole overlays, chosen skills and servers a session starts with; the Store is where those
/// are found and installed, and the editor's picker leads there without losing what has been typed.
library;

import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../profile/profile_home.dart';
import '../store/overlay_page.dart' show SaveConfig;
import 'loadout_editor.dart';

class LoadoutsSection extends StatefulWidget {
  const LoadoutsSection({
    super.key,
    required this.config,
    required this.home,
    required this.onSave,
    this.onStore,
    this.onUse,
    this.agents = const [],
    this.draft,
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

  /// The loadout being edited, kept by the owner so that it outlives this page; null opens the list.
  final ValueNotifier<LoadoutDraft?>? draft;

  @override
  State<LoadoutsSection> createState() => LoadoutsSectionState();
}

class LoadoutsSectionState extends State<LoadoutsSection> {
  late final ValueNotifier<LoadoutDraft?> _own = ValueNotifier(null);
  ValueNotifier<LoadoutDraft?> get _editing => widget.draft ?? _own;

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

  @override
  void dispose() {
    _own.dispose();
    super.dispose();
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

  /// What is missing from [loadout]: each as `name (overlay)`.
  List<String> _missing(Loadout loadout) => [
    for (final name in loadout.overlays) if (!_config.overlays.containsKey(name)) '$name (overlay)',
    if (_skillsKnown)
      for (final name in loadout.skills) if (!_skills.contains(name)) '$name (skill)',
    for (final name in loadout.mcp) if (!_servers.contains(name)) '$name (MCP server)',
  ];

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
      changed = _config.withLoadouts(all, defaultName: renamedDefault);
    } on FormatException catch (error) {
      setState(() => _problem = error.message);
      return;
    }
    final saved = await widget.onSave(changed);
    if (!mounted) return;
    setState(() => _problem = saved == null ? 'canoa.cfg could not be written' : null);
    if (saved != null) _editing.value = null;
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
    final saved = await widget.onSave(_config.withLoadouts({..._config.loadouts}..remove(name)));
    if (mounted) setState(() => _problem = saved == null ? 'canoa.cfg could not be written' : null);
  }

  Future<void> _makeDefault(String name) async {
    final saved = await widget.onSave(_config.withLoadouts(_config.loadouts, defaultName: name));
    if (mounted) setState(() => _problem = saved == null ? 'canoa.cfg could not be written' : null);
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
  Widget build(BuildContext context) => ValueListenableBuilder<LoadoutDraft?>(
    valueListenable: _editing,
    builder: (context, draft, _) => Align(
      alignment: Alignment.topCenter,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 760),
        child: draft != null ? _editor(draft) : _list(context),
      ),
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

  Widget _list(BuildContext context) {
    final loadouts = _config.loadouts;
    final scheme = Theme.of(context).colorScheme;
    return ListView(
      key: const Key('loadouts-list'),
      padding: const EdgeInsets.all(16),
      children: [
        Row(children: [
          Expanded(child: Text('Loadouts', style: Theme.of(context).textTheme.titleLarge)),
          FilledButton.icon(
            key: const Key('loadout-new'),
            onPressed: () => _open(LoadoutDraft()),
            icon: const Icon(Icons.add),
            label: const Text('New loadout'),
          ),
        ]),
        const SizedBox(height: 4),
        Text(
          'A loadout is what a session starts with: whole overlays, skills you chose and MCP servers. '
          'Editing one reaches new sessions only.',
          style: Theme.of(context).textTheme.bodySmall,
        ),
        if (_problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(_problem!, style: TextStyle(color: scheme.error))),
        const SizedBox(height: 12),
        if (loadouts.isEmpty)
          const Padding(
            padding: EdgeInsets.symmetric(vertical: 24),
            child: Text('No loadouts yet. New sessions start with every overlay and every server.', key: Key('loadouts-empty')),
          ),
        for (final MapEntry(:key, :value) in loadouts.entries) _card(context, key, value),
      ],
    );
  }

  Widget _card(BuildContext context, String name, Loadout loadout) {
    final scheme = Theme.of(context).colorScheme;
    final missing = _missing(loadout);
    final isDefault = name == _config.defaultLoadout;
    String count(int n, String one, String many) => '$n ${n == 1 ? one : many}';
    return Card(
      key: Key('loadout-card-$name'),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Row(children: [
            Flexible(child: Text(name, style: Theme.of(context).textTheme.titleMedium)),
            if (isDefault) Padding(padding: const EdgeInsets.only(left: 8), child: Chip(key: Key('loadout-default-$name'), label: const Text('Default'), visualDensity: VisualDensity.compact)),
          ]),
          if (loadout.description.isNotEmpty) Text(loadout.description),
          Text(
            '${count(loadout.overlays.length, 'overlay', 'overlays')} · ${count(loadout.skills.length, 'skill', 'skills')} · '
            '${count(loadout.mcp.length, 'MCP server', 'MCP servers')}',
            key: Key('loadout-count-$name'),
          ),
          Text(
            missing.isEmpty ? 'Ready' : 'Missing: ${missing.join(', ')}',
            key: Key('loadout-ready-$name'),
            style: TextStyle(color: missing.isEmpty ? null : scheme.error),
          ),
          Wrap(spacing: 4, children: [
            TextButton(key: Key('loadout-edit-$name'), onPressed: () => _open(LoadoutDraft.of(name, loadout)), child: const Text('Edit')),
            TextButton(key: Key('loadout-duplicate-$name'), onPressed: () => _open(LoadoutDraft.of(name, loadout, copy: true)), child: const Text('Duplicate')),
            if (widget.onUse != null)
              TextButton(key: Key('loadout-use-$name'), onPressed: () => widget.onUse!(name), child: const Text('Use in new session')),
            if (!isDefault) TextButton(key: Key('loadout-default-set-$name'), onPressed: () => unawaited(_makeDefault(name)), child: const Text('Make default')),
            TextButton(key: Key('loadout-delete-$name'), onPressed: () => unawaited(_delete(name)), child: const Text('Delete')),
          ]),
        ]),
      ),
    );
  }
}
