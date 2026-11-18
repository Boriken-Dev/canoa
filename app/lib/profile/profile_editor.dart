/// The editor of one profile of canoa.cfg's `[profiles]`: its name and the values a new session starts with when it is chosen: the
/// source it runs on, its model, effort and mode, its starting instructions (text added to the session's initial context after
/// every other layer's), the prompt cache lifetime and the loadout. The account and the permissions are the session's own and
/// are not part of one.
library;

import 'package:flutter/material.dart';

import '../chat/cache_lifetime.dart';
import '../config/canoa_config.dart';
import 'profile.dart' show ProfileAccess;
import 'profile_items.dart';

/// The items the editor draws a row of its own for, which the host also lists: the rest of the host's list is drawn from the forms
/// the providers describe.
const _ownRows = {'model', 'effort', 'mode', 'context'};

class ProfileEditor extends StatefulWidget {
  const ProfileEditor({
    super.key,
    required this.original,
    required this.preset,
    required this.taken,
    required this.sources,
    required this.loadouts,
    required this.onSave,
    required this.onCancel,
    this.catalog,
  });

  /// The items the host lists, which the editor draws a form for; [profileItems] when none is given.
  final ProfileItemCatalog? catalog;

  /// The name the profile has in canoa.cfg; null for a new one.
  final String? original;

  /// What it holds now.
  final Preset preset;

  /// The names the other profiles have.
  final Set<String> taken;

  /// The sources a profile may run on, by label.
  final List<String> sources;

  /// The loadouts canoa.cfg names.
  final List<String> loadouts;

  /// Writes the profile; a refusal is the answer, shown beside Save.
  final Future<String?> Function(String name, Preset preset) onSave;
  final VoidCallback onCancel;

  @override
  State<ProfileEditor> createState() => _ProfileEditorState();
}

class _ProfileEditorState extends State<ProfileEditor> {
  late final _name = TextEditingController(text: widget.original ?? '');
  late final _model = TextEditingController(text: widget.preset.model ?? '');
  late final _effort = TextEditingController(text: widget.preset.effort ?? '');
  late final _mode = TextEditingController(text: widget.preset.mode ?? '');
  late final _context = TextEditingController(text: widget.preset.context ?? '');
  late String? _source = widget.preset.source;
  late String? _cache = widget.preset.cacheLifetime;
  late String? _loadout = widget.preset.loadout;
  late ProfileAccess _access = widget.preset.access;
  late Map<String, Map<String, Object?>> _items = {for (final MapEntry(:key, :value) in widget.preset.items.entries) key: {...value}};
  String? _problem;
  bool _saving = false;

  @override
  void dispose() {
    for (final controller in [_name, _model, _effort, _mode, _context]) {
      controller.dispose();
    }
    super.dispose();
  }

  String? get _nameProblem {
    final name = _name.text;
    if (name.isEmpty) return null;
    if (!isPresetName(name)) return presetNameRule;
    if (widget.taken.any((other) => other.toLowerCase() == name.toLowerCase() && other != widget.original)) {
      return 'Another profile has that name';
    }
    return null;
  }

  static String? _text(TextEditingController controller) {
    final text = controller.text.trim();
    return text.isEmpty ? null : text;
  }

  Preset _made() => Preset(
    source: _source,
    model: _text(_model),
    effort: _text(_effort),
    mode: _text(_mode),
    // Instructions are kept as written, but a field of only blanks is none.
    context: _context.text.trim().isEmpty ? null : _context.text,
    cacheLifetime: _cache,
    loadout: _loadout,
    access: _access,
    items: _items,
  );

  ProfileItemCatalog get _catalog => widget.catalog ?? profileItems;

  /// What the form of [item] holds now.
  Map<String, Object?> _valuesOf(ProfileItemInfo item) => item.id == 'access'
      ? {'sandbox': ?_access.sandbox, 'network': ?_access.network, 'approvals': ?_access.approvals}
      : _items[item.id] ?? const {};

  /// Takes what the form of [item] holds: the access is the profile's own setting, any other item's values are kept under its id,
  /// and an item with no value left is no longer set.
  void _take(ProfileItemInfo item, Map<String, Object?> values) {
    setState(() {
      if (item.id == 'access') {
        _access = ProfileAccess(
          sandbox: values['sandbox'] as String?,
          network: values['network'] as bool?,
          approvals: values['approvals'] as String?,
        );
      } else if (values.isEmpty) {
        _items = {..._items}..remove(item.id);
      } else {
        _items = {..._items, item.id: values};
      }
    });
  }

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _problem = null;
    });
    final refused = await widget.onSave(_name.text, _made());
    if (mounted) {
      setState(() {
        _saving = false;
        _problem = refused;
      });
    }
  }

  DropdownMenuItem<String?> _none(String words) => DropdownMenuItem<String?>(value: null, child: Text(words));

  @override
  Widget build(BuildContext context) {
    final ready = _name.text.isNotEmpty && _nameProblem == null && !_saving;
    // A value the file holds that the list no longer offers stays shown, so saving does not drop it unseen.
    final sources = {...widget.sources, ?_source}.toList();
    final loadouts = {...widget.loadouts, ?_loadout}.toList();
    return Column(
      key: const Key('profile-editor'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(widget.original == null ? 'New profile' : 'Edit ${widget.original}', style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-name'),
          controller: _name,
          // The default profile keeps its name: sessions that name none start with it.
          enabled: widget.original != presetDefault,
          decoration: InputDecoration(labelText: 'Name', errorText: _nameProblem),
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 8),
        DropdownButtonFormField<String?>(
          key: const Key('profile-edit-source'),
          isExpanded: true,
          initialValue: _source,
          decoration: const InputDecoration(labelText: 'Source'),
          items: [
            _none('The session\'s own'),
            for (final label in sources) DropdownMenuItem<String?>(value: label, child: Text(label)),
          ],
          onChanged: (value) => setState(() => _source = value),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: TextField(
              key: const Key('profile-edit-model'),
              controller: _model,
              decoration: const InputDecoration(labelText: 'Model', hintText: 'The agent\'s own'),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              key: const Key('profile-edit-effort'),
              controller: _effort,
              decoration: const InputDecoration(labelText: 'Effort', hintText: 'The agent\'s own'),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              key: const Key('profile-edit-mode'),
              controller: _mode,
              decoration: const InputDecoration(labelText: 'Mode', hintText: 'The agent\'s own'),
            ),
          ),
        ]),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-context'),
          controller: _context,
          minLines: 3,
          maxLines: 10,
          decoration: const InputDecoration(
            labelText: 'Starting instructions',
            alignLabelWithHint: true,
            helperText: 'Added to the session\'s initial context after every other layer\'s',
          ),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-cache'),
              isExpanded: true,
              initialValue: _cache,
              decoration: const InputDecoration(labelText: 'Prompt cache'),
              items: [
                _none('Not set'),
                for (final lifetime in CacheLifetime.values)
                  DropdownMenuItem<String?>(
                    value: lifetime.id,
                    child: Text('${lifetime.label}: ${lifetime.about}', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _cache = value),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-loadout'),
              isExpanded: true,
              initialValue: _loadout,
              decoration: const InputDecoration(labelText: 'Loadout'),
              items: [
                _none('The default'),
                for (final name in loadouts)
                  DropdownMenuItem<String?>(
                    value: name,
                    child: Text(widget.loadouts.contains(name) ? name : '$name (not in canoa.cfg)', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _loadout = value),
            ),
          ),
        ]),
        ListenableBuilder(
          listenable: _catalog,
          builder: (context, _) {
            final listed = [for (final item in _catalog.items) if (!_ownRows.contains(item.id)) item];
            // An item the profile holds values for that no provider offers now (its extension was removed): the values stay, and say so.
            final gone = [for (final id in _items.keys) if (_catalog.answered && _catalog.byId(id) == null) id];
            return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final item in listed)
                _ItemSection(
                  key: ValueKey('item-${item.id}'),
                  item: item,
                  values: _valuesOf(item),
                  onChanged: (values) => _take(item, values),
                ),
              for (final id in gone)
                ListTile(
                  key: Key('item-gone-$id'),
                  dense: true,
                  leading: Icon(Icons.warning_amber_outlined, color: Theme.of(context).colorScheme.error),
                  title: Text(id),
                  subtitle: const Text('No extension offers this item now: its values are kept and not applied.'),
                  trailing: TextButton(
                    key: Key('item-gone-remove-$id'),
                    onPressed: () => setState(() => _items = {..._items}..remove(id)),
                    child: const Text('Remove its values'),
                  ),
                ),
            ]);
          },
        ),
        if (_problem != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(_problem!, key: const Key('profile-edit-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        const SizedBox(height: 8),
        Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          TextButton(key: const Key('profile-edit-cancel'), onPressed: widget.onCancel, child: const Text('Cancel')),
          const SizedBox(width: 8),
          FilledButton(key: const Key('profile-edit-save'), onPressed: ready ? _save : null, child: const Text('Save')),
        ]),
      ],
    );
  }
}

/// The form one item's provider describes: a row for each of its fields, which holds the values the person gave and nothing else.
class _ItemSection extends StatefulWidget {
  const _ItemSection({super.key, required this.item, required this.values, required this.onChanged});

  final ProfileItemInfo item;
  final Map<String, Object?> values;
  final ValueChanged<Map<String, Object?>> onChanged;

  @override
  State<_ItemSection> createState() => _ItemSectionState();
}

class _ItemSectionState extends State<_ItemSection> {
  late final Map<String, TextEditingController> _controllers = {
    for (final field in widget.item.fields)
      if (const {'text', 'path', 'number', 'list'}.contains(field.shape)) field.name: TextEditingController(text: _shown(widget.values[field.name])),
  };

  static String _shown(Object? value) => switch (value) {
    null => '',
    List<Object?> list => list.join('\n'),
    _ => '$value',
  };

  @override
  void dispose() {
    for (final controller in _controllers.values) {
      controller.dispose();
    }
    super.dispose();
  }

  /// The values the form holds: a field left empty is absent. [chosen] is what a menu has just been given for its field.
  Map<String, Object?> _read({String? menu, Object? chosen}) {
    final out = <String, Object?>{};
    for (final field in widget.item.fields) {
      final controller = _controllers[field.name];
      final Object? value;
      if (controller == null) {
        value = field.name == menu ? chosen : widget.values[field.name];
      } else {
        final text = controller.text.trim();
        value = text.isEmpty
            ? null
            : switch (field.shape) {
                'list' => [for (final line in controller.text.split('\n')) if (line.trim().isNotEmpty) line.trim()],
                'number' => int.tryParse(text) ?? text,
                _ => text,
              };
      }
      if (value != null) out[field.name] = value;
    }
    return out;
  }

  Widget _field(ItemField field) {
    final key = Key('item-field-${widget.item.id}-${field.name}');
    final label = field.required ? '${field.label} *' : field.label;
    final hint = field.defaultValue == null ? null : 'Default: ${field.defaultValue}';
    switch (field.shape) {
      case 'bool':
        return DropdownButtonFormField<bool?>(
          key: key,
          isExpanded: true,
          initialValue: widget.values[field.name] as bool?,
          decoration: InputDecoration(labelText: label, helperText: field.help, hintText: hint),
          items: const [
            DropdownMenuItem<bool?>(value: null, child: Text('Not set')),
            DropdownMenuItem<bool?>(value: true, child: Text('Yes')),
            DropdownMenuItem<bool?>(value: false, child: Text('No')),
          ],
          onChanged: (value) => widget.onChanged(_read(menu: field.name, chosen: value)),
        );
      case 'choice':
        return DropdownButtonFormField<String?>(
          key: key,
          isExpanded: true,
          initialValue: widget.values[field.name] as String?,
          decoration: InputDecoration(labelText: label, helperText: field.help, hintText: hint),
          items: [
            DropdownMenuItem<String?>(value: null, child: Text(field.defaultValue == null ? 'Not set' : 'Default (${field.defaultValue})')),
            for (final choice in field.choices) DropdownMenuItem<String?>(value: choice, child: Text(choice)),
          ],
          onChanged: (value) => widget.onChanged(_read(menu: field.name, chosen: value)),
        );
      default:
        final list = field.shape == 'list';
        return TextField(
          key: key,
          controller: _controllers[field.name],
          minLines: list ? 2 : 1,
          maxLines: list ? 6 : 1,
          keyboardType: field.shape == 'number' ? TextInputType.number : null,
          decoration: InputDecoration(labelText: label, helperText: field.help, hintText: list ? 'One a line' : hint),
          onChanged: (_) => widget.onChanged(_read()),
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    final item = widget.item;
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        key: Key('item-section-${item.id}'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(item.title, style: Theme.of(context).textTheme.titleSmall),
          if (item.help != null) Text(item.help!, style: Theme.of(context).textTheme.bodySmall),
          if (item.harnesses.isNotEmpty) Text('For ${item.harnesses.join(', ')} only', style: Theme.of(context).textTheme.bodySmall),
          for (final field in item.fields) ...[const SizedBox(height: 8), _field(field)],
        ],
      ),
    );
  }
}
