/// The editor of one loadout: its name and description, and three groups of what it holds, which are whole overlays (in the order
/// `[overlays]` composes them), standalone skills and MCP servers. Each item says whether it is installed, that the loadout
/// includes it, and, for a server, which agents their own settings allow it; whether a session turns it on is the composer's
/// choice, and no control here reaches an item inside an overlay: an overlay's options are set on the overlay.
library;

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import '../profile/profile.dart' show isEntryName;

/// A loadout being edited: what the editor shows before Save writes it. It is kept by whoever opened the editor, so it outlives
/// the page: a visit to the Store to install something leaves it as it was.
class LoadoutDraft extends ChangeNotifier {
  LoadoutDraft({
    this.original,
    this.name = '',
    this.description = '',
    List<String> overlays = const [],
    List<String> skills = const [],
    List<String> mcp = const [],
  }) : overlays = [...overlays],
       skills = [...skills],
       mcp = [...mcp];

  /// A draft of the loadout [name] as [loadout] holds it now; with [copy], a new loadout that starts as it is.
  factory LoadoutDraft.of(String name, Loadout loadout, {bool copy = false}) =>
      LoadoutDraft(
        original: copy ? null : name,
        name: copy ? '$name copy' : name,
        description: loadout.description,
        overlays: loadout.overlays,
        skills: loadout.skills,
        mcp: loadout.mcp,
      );

  /// The name the loadout has in canoa.cfg; null for a new one.
  final String? original;
  String name, description;
  final List<String> overlays, skills, mcp;

  void changed() => notifyListeners();

  /// Why [name] cannot be written, given the names [others] hold; null when it can.
  String? problemWith(Iterable<String> others) {
    if (!isEntryName(name)) {
      return 'A loadout is named with 1 to 64 letters, digits, - or _';
    }
    if (others.any((other) => other.toLowerCase() == name.toLowerCase() && other != original)) {
      return 'Another loadout has that name';
    }
    return null;
  }

  Loadout toLoadout() => Loadout(
    description: description.trim(),
    overlays: overlays,
    skills: skills,
    mcp: mcp,
  );
}

/// What the picker adds to: one of the three groups.
enum LoadoutGroup { overlays, skills, mcp }

class LoadoutEditor extends StatelessWidget {
  const LoadoutEditor({
    super.key,
    required this.draft,
    required this.config,
    required this.skills,
    required this.agents,
    required this.onAdd,
    required this.onSave,
    required this.onCancel,
    this.problem,
  });

  final LoadoutDraft draft;
  final CanoaConfig config;

  /// The skills the person chose that are installed, by name.
  final Set<String> skills;

  /// The agents a server is allowed for or turned off for.
  final List<String> agents;
  final void Function(LoadoutGroup group) onAdd;
  final VoidCallback onSave, onCancel;
  final String? problem;

  /// The overlays in the order `[overlays]` composes them, then any it no longer lists.
  List<String> get _overlayRows => [
    for (final name in config.overlays.keys)
      if (draft.overlays.contains(name)) name,
    for (final name in draft.overlays)
      if (!config.overlays.containsKey(name)) name,
  ];

  Set<String> get _servers => {
    for (final MapEntry(:key, :value) in config.profile.mcp.entries)
      if (value != null) key,
  };

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: draft,
      builder: (context, _) => _page(context),
    );
  }

  Widget _page(BuildContext context) {
    final wrong = draft.problemWith(config.loadouts.keys);
    return ListView(
      key: const Key('loadout-editor'),
      padding: const EdgeInsets.all(16),
      children: [
        Text(
          draft.original == null ? 'New loadout' : 'Edit ${draft.original}',
          style: Theme.of(context).textTheme.titleLarge,
        ),
        const SizedBox(height: 12),
        TextFormField(
          key: const Key('loadout-name'),
          initialValue: draft.name,
          decoration: InputDecoration(
            labelText: 'Name',
            errorText: draft.name.isEmpty
                ? null
                : draft.problemWith(config.loadouts.keys),
          ),
          onChanged: (value) {
            draft.name = value;
            draft.changed();
          },
        ),
        const SizedBox(height: 8),
        TextFormField(
          key: const Key('loadout-description'),
          initialValue: draft.description,
          decoration: const InputDecoration(labelText: 'What it is for'),
          onChanged: (value) => draft.description = value,
        ),
        const SizedBox(height: 16),
        _Group(
          title: 'Whole overlays',
          about:
              'Each applies whole, in the order the overlays compose. Its options are set on the overlay.',
          add: 'Add an overlay',
          group: LoadoutGroup.overlays,
          onAdd: onAdd,
          rows: [
            for (final name in _overlayRows)
              _Row(
                keyName: 'overlay-$name',
                name: name,
                installed: config.overlays.containsKey(name),
                onRemove: () {
                  draft.overlays.remove(name);
                  draft.changed();
                },
              ),
          ],
        ),
        _Group(
          title: 'Standalone skills',
          about: 'Skills you chose, without an overlay.',
          add: 'Add a skill',
          group: LoadoutGroup.skills,
          onAdd: onAdd,
          rows: [
            for (final name in draft.skills)
              _Row(
                keyName: 'skill-$name',
                name: name,
                installed: skills.contains(name),
                onRemove: () {
                  draft.skills.remove(name);
                  draft.changed();
                },
              ),
          ],
        ),
        _Group(
          title: 'MCP servers',
          about:
              'Servers of your own profile. What each agent may start is set in Settings and stays as it is.',
          add: 'Add a server',
          group: LoadoutGroup.mcp,
          onAdd: onAdd,
          rows: [
            for (final name in draft.mcp)
              _Row(
                keyName: 'mcp-$name',
                name: name,
                installed: _servers.contains(name),
                allowed: agents.isEmpty
                    ? null
                    : 'Allowed for ${agents.where((agent) => config.agentMcp[agent]?[name] != false).length} of ${agents.length} agents',
                onRemove: () {
                  draft.mcp.remove(name);
                  draft.changed();
                },
              ),
          ],
        ),
        if (problem != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              problem!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
        const SizedBox(height: 16),
        Wrap(
          spacing: 8,
          children: [
            FilledButton(
              key: const Key('loadout-save'),
              onPressed: wrong == null ? onSave : null,
              child: const Text('Save'),
            ),
            TextButton(
              key: const Key('loadout-cancel'),
              onPressed: onCancel,
              child: const Text('Cancel'),
            ),
          ],
        ),
      ],
    );
  }
}

class _Row {
  const _Row({
    required this.keyName,
    required this.name,
    required this.installed,
    required this.onRemove,
    this.allowed,
  });
  final String keyName, name;
  final bool installed;
  final String? allowed;
  final VoidCallback onRemove;
}

class _Group extends StatelessWidget {
  const _Group({
    required this.title,
    required this.about,
    required this.add,
    required this.group,
    required this.onAdd,
    required this.rows,
  });

  final String title, about, add;
  final LoadoutGroup group;
  final void Function(LoadoutGroup group) onAdd;
  final List<_Row> rows;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(title, style: Theme.of(context).textTheme.titleMedium),
          Text(about, style: Theme.of(context).textTheme.bodySmall),
          const SizedBox(height: 4),
          if (rows.isEmpty)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 8),
              child: Text('None yet'),
            ),
          for (final row in rows)
            ListTile(
              key: Key('loadout-row-${row.keyName}'),
              dense: true,
              contentPadding: EdgeInsets.zero,
              title: Text(row.name),
              subtitle: Wrap(
                spacing: 8,
                children: [
                  Text(
                    row.installed ? 'Installed' : 'Not installed',
                    style: TextStyle(
                      color: row.installed ? null : scheme.error,
                    ),
                  ),
                  const Text('Included'),
                  if (row.allowed != null) Text(row.allowed!),
                ],
              ),
              trailing: IconButton(
                key: Key('loadout-remove-${row.keyName}'),
                tooltip: 'Take ${row.name} out',
                icon: const Icon(Icons.remove_circle_outline),
                onPressed: row.onRemove,
              ),
            ),
          TextButton.icon(
            key: Key('loadout-add-${group.name}'),
            onPressed: () => onAdd(group),
            icon: const Icon(Icons.add),
            label: Text(add),
          ),
        ],
      ),
    );
  }
}

/// What the picker answers: the names chosen, or a wish to look in the Store, which keeps the draft.
typedef PickerAnswer = ({List<String> chosen, bool toStore});

/// A searchable list of what is installed and not yet in the loadout, with a way on to the Store for what is not.
Future<PickerAnswer?> pickInstalled(
  BuildContext context, {
  required String title,
  required Map<String, String> items,
}) {
  return showDialog<PickerAnswer>(
    context: context,
    builder: (_) => _Picker(title: title, items: items),
  );
}

class _Picker extends StatefulWidget {
  const _Picker({required this.title, required this.items});
  final String title;
  final Map<String, String> items;

  @override
  State<_Picker> createState() => _PickerState();
}

class _PickerState extends State<_Picker> {
  final _chosen = <String>{};
  var _query = '';

  @override
  Widget build(BuildContext context) {
    final words = _query
        .toLowerCase()
        .split(RegExp(r'\s+'))
        .where((word) => word.isNotEmpty);
    final shown = [
      for (final MapEntry(:key, :value) in widget.items.entries)
        if (words.every('$key $value'.toLowerCase().contains)) key,
    ];
    return AlertDialog(
      title: Text(widget.title),
      content: SizedBox(
        width: 480,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              key: const Key('loadout-picker-search'),
              decoration: const InputDecoration(
                prefixIcon: Icon(Icons.search),
                hintText: 'Search what is installed',
              ),
              onChanged: (value) => setState(() => _query = value),
            ),
            Flexible(
              child: shown.isEmpty
                  ? const Padding(
                      padding: EdgeInsets.all(16),
                      child: Text(
                        'Nothing installed matches. The Store has more.',
                      ),
                    )
                  : ListView(
                      shrinkWrap: true,
                      children: [
                        for (final name in shown)
                          CheckboxListTile(
                            key: Key('loadout-pick-$name'),
                            dense: true,
                            value: _chosen.contains(name),
                            title: Text(name),
                            subtitle: widget.items[name]!.isEmpty
                                ? null
                                : Text(widget.items[name]!),
                            onChanged: (on) => setState(
                              () => on == true
                                  ? _chosen.add(name)
                                  : _chosen.remove(name),
                            ),
                          ),
                      ],
                    ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          key: const Key('loadout-picker-store'),
          onPressed: () =>
              Navigator.pop(context, (chosen: _chosen.toList(), toStore: true)),
          child: const Text('Find more in the Store'),
        ),
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('loadout-picker-add'),
          onPressed: _chosen.isEmpty
              ? null
              : () => Navigator.pop(context, (
                  chosen: _chosen.toList(),
                  toStore: false,
                )),
          child: const Text('Add'),
        ),
      ],
    );
  }
}
