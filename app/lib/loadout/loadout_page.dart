/// One loadout as the Loadouts section's main view shows it: what it is for, whether a session can start with it, the overlays,
/// skills and MCP servers it holds, and the setup a new session gets from it; with edit, copy, use, make default and delete.
library;

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'loadouts_index.dart' show loadoutCounts;

class LoadoutPage extends StatelessWidget {
  const LoadoutPage({
    super.key,
    required this.name,
    required this.config,
    required this.skills,
    required this.skillsKnown,
    required this.agents,
    required this.onEdit,
    required this.onDuplicate,
    required this.onMakeDefault,
    required this.onDelete,
    this.onUse,
    this.problem,
  });

  final String name;
  final CanoaConfig config;

  /// The skills the person chose that are installed; [skillsKnown] says they have been read, since until then a skill is not
  /// called missing.
  final Set<String> skills;
  final bool skillsKnown;
  final List<String> agents;
  final VoidCallback onEdit, onDuplicate, onMakeDefault, onDelete;

  /// Starts a new session with this loadout; null where sessions cannot be started from here.
  final VoidCallback? onUse;
  final String? problem;

  Loadout get _loadout => config.loadouts[name]!;

  Set<String> get _servers => {for (final MapEntry(:key, :value) in config.profile.mcp.entries) if (value != null) key};

  /// What is missing, each as `name (overlay)`.
  List<String> get _missing => [
    for (final item in _loadout.overlays) if (!config.overlays.containsKey(item)) '$item (overlay)',
    if (skillsKnown) for (final item in _loadout.skills) if (!skills.contains(item)) '$item (skill)',
    for (final item in _loadout.mcp) if (!_servers.contains(item)) '$item (MCP server)',
  ];

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final loadout = _loadout;
    final missing = _missing;
    final isDefault = name == config.defaultLoadout;
    // Overlays apply in the order `[overlays]` composes them; one it no longer lists comes last.
    final overlays = [
      for (final held in config.overlays.keys) if (loadout.overlays.contains(held)) held,
      for (final held in loadout.overlays) if (!config.overlays.containsKey(held)) held,
    ];
    return ListView(
      key: Key('loadout-page-$name'),
      padding: const EdgeInsets.all(16),
      children: [
        Row(children: [
          Flexible(child: Text(name, style: text.titleLarge)),
          if (isDefault)
            Padding(
              padding: const EdgeInsets.only(left: 8),
              child: Chip(key: Key('loadout-default-$name'), label: const Text('Default'), visualDensity: VisualDensity.compact),
            ),
        ]),
        if (loadout.description.isNotEmpty) Padding(padding: const EdgeInsets.only(top: 4), child: Text(loadout.description)),
        const SizedBox(height: 4),
        Text(loadoutCounts(loadout), key: Key('loadout-count-$name'), style: text.bodySmall),
        Text(
          missing.isEmpty ? 'Ready' : 'Missing: ${missing.join(', ')}',
          key: Key('loadout-ready-$name'),
          style: TextStyle(color: missing.isEmpty ? null : scheme.error),
        ),
        if (problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(problem!, style: TextStyle(color: scheme.error))),
        const SizedBox(height: 8),
        Wrap(spacing: 4, children: [
          FilledButton.tonal(key: Key('loadout-edit-$name'), onPressed: onEdit, child: const Text('Edit')),
          TextButton(key: Key('loadout-duplicate-$name'), onPressed: onDuplicate, child: const Text('Duplicate')),
          if (onUse != null) TextButton(key: Key('loadout-use-$name'), onPressed: onUse, child: const Text('Use in new session')),
          if (!isDefault) TextButton(key: Key('loadout-default-set-$name'), onPressed: onMakeDefault, child: const Text('Make default')),
          TextButton(key: Key('loadout-delete-$name'), onPressed: onDelete, child: const Text('Delete')),
        ]),
        const SizedBox(height: 16),
        _Group(
          title: 'Whole overlays',
          about: 'Each applies whole, in the order the overlays compose.',
          rows: [for (final item in overlays) _Item('overlay-$item', item, config.overlays.containsKey(item) ? 'Installed' : 'Not installed', ok: config.overlays.containsKey(item))],
        ),
        _Group(
          title: 'Standalone skills',
          about: 'Skills you chose, without an overlay.',
          rows: [
            for (final item in loadout.skills)
              _Item('skill-$item', item, !skillsKnown ? 'Chosen' : skills.contains(item) ? 'Installed' : 'Not installed', ok: !skillsKnown || skills.contains(item)),
          ],
        ),
        _Group(
          title: 'MCP servers',
          about: 'Servers of your own profile. What each agent may start is set in Settings.',
          rows: [
            for (final item in loadout.mcp)
              _Item(
                'mcp-$item',
                item,
                _servers.contains(item) ? 'In your profile' : 'Not in your profile',
                ok: _servers.contains(item),
                extra: agents.isEmpty ? null : 'Allowed for ${agents.where((agent) => config.agentMcp[agent]?[item] != false).length} of ${agents.length} agents',
              ),
          ],
        ),
        Text('Effective setup', style: text.titleMedium),
        const SizedBox(height: 4),
        Container(
          key: Key('loadout-effective-$name'),
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(8)),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(
              missing.isEmpty
                  ? 'What a new session with this loadout starts with, overlays in the order they apply.'
                  : 'A new session with this loadout starts without what is missing above.',
              style: text.bodySmall,
            ),
            const SizedBox(height: 8),
            _Line('Overlays', [for (final item in overlays) if (config.overlays.containsKey(item)) item]),
            _Line('Skills', [for (final item in loadout.skills) if (!skillsKnown || skills.contains(item)) item]),
            _Line('MCP servers', [for (final item in loadout.mcp) if (_servers.contains(item)) item]),
          ]),
        ),
      ],
    );
  }
}

class _Item {
  const _Item(this.keyName, this.name, this.status, {required this.ok, this.extra});
  final String keyName, name, status;
  final bool ok;
  final String? extra;
}

class _Group extends StatelessWidget {
  const _Group({required this.title, required this.about, required this.rows});

  final String title, about;
  final List<_Item> rows;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(title, style: Theme.of(context).textTheme.titleMedium),
        Text(about, style: Theme.of(context).textTheme.bodySmall),
        const SizedBox(height: 4),
        if (rows.isEmpty) const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('None')),
        for (final row in rows)
          ListTile(
            key: Key('loadout-item-${row.keyName}'),
            dense: true,
            contentPadding: EdgeInsets.zero,
            title: Text(row.name),
            subtitle: Wrap(spacing: 8, children: [
              Text(row.status, style: TextStyle(color: row.ok ? null : scheme.error)),
              if (row.extra != null) Text(row.extra!),
            ]),
          ),
      ]),
    );
  }
}

class _Line extends StatelessWidget {
  const _Line(this.label, this.names);

  final String label;
  final List<String> names;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 2),
    child: Text.rich(TextSpan(children: [
      TextSpan(text: '$label: ', style: const TextStyle(fontWeight: FontWeight.w600)),
      TextSpan(text: names.isEmpty ? 'none' : names.join(', ')),
    ])),
  );
}
