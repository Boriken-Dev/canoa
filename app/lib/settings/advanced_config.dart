import 'dart:async';
import 'dart:convert';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import '../shell/confirm_remove.dart';
import 'settings_row.dart';

/// What the app asks the host with: `config/advanced` and `config/advanced_set`.
typedef AdvancedCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// How a table holds what it holds: one table of settings, a table of entries each named by its key, a list of tables, or a table of
/// names each holding one text.
enum AdvancedShape { flat, entries, list, pairs }

/// A value a lower layer gives that the person's own file does not: shown with where it comes from, never taken out from here.
final class Inherited {
  const Inherited({required this.layer, required this.value});
  final String layer;
  final Object? value;
}

/// One table of canoa.cfg no dedicated page edits: its guatiao schema, what the person's own file holds and what a lower layer gives.
final class AdvancedTable {
  const AdvancedTable({required this.path, required this.title, required this.shape, required this.schema, required this.own, required this.inherited});

  factory AdvancedTable.read(Map<Object?, Object?> raw) => AdvancedTable(
    path: [for (final key in raw['path'] as List<Object?>) '$key'],
    title: '${raw['title']}',
    shape: switch (raw['shape']) {
      'entries' => AdvancedShape.entries,
      'list' => AdvancedShape.list,
      'pairs' => AdvancedShape.pairs,
      _ => AdvancedShape.flat,
    },
    schema: {for (final MapEntry(:key, :value) in (raw['schema'] as Map<Object?, Object?>? ?? const {}).entries) '$key': value},
    own: raw['own'],
    inherited: {
      for (final MapEntry(:key, :value) in (raw['inherited'] as Map<Object?, Object?>? ?? const {}).entries)
        '$key': Inherited(layer: '${(value as Map)['layer']}', value: value['value']),
    },
  );

  final List<String> path;
  final String title;
  final AdvancedShape shape;
  final Map<String, Object?> schema;
  final Object? own;
  final Map<String, Inherited> inherited;

  String get id => path.join('.');

  /// The members of an entry the schema says hold free-form JSON, which a form shows as text.
  Set<String> get jsonMembers => {
    for (final MapEntry(:key, :value) in schemaProperties(schema).entries)
      if (value is Map && value['x-json'] == true) key,
  };

  /// The person's own entries by name (a flat table is one entry named '').
  Map<String, Map<String, Object?>> get ownEntries => switch (own) {
    final Map<Object?, Object?> map when shape == AdvancedShape.pairs => {for (final MapEntry(:key, :value) in map.entries) '$key': {'value': value}},
    final Map<Object?, Object?> map when shape == AdvancedShape.entries => {
      for (final MapEntry(:key, :value) in map.entries)
        if (value is Map) '$key': {for (final MapEntry(key: member, value: held) in value.entries) '$member': held},
    },
    final Map<Object?, Object?> map => {'': {for (final MapEntry(:key, :value) in map.entries) '$key': value}},
    _ => const {},
  };

  /// The person's own rows of a list.
  List<Map<String, Object?>> get ownRows => [
    if (own case final List<Object?> rows)
      for (final row in rows)
        if (row is Map) {for (final MapEntry(:key, :value) in row.entries) '$key': value},
  ];
}

/// The tables of canoa.cfg with no page of their own, and the writes that add, change and remove them, through the host.
class AdvancedConfig extends ChangeNotifier {
  AdvancedConfig(this._call);

  final AdvancedCall _call;
  bool _disposed = false, _loaded = false;
  List<AdvancedTable> _tables = const [];
  String? _revision, _problem;

  List<AdvancedTable> get tables => _tables;
  bool get loaded => _loaded;

  /// Why the tables could not be listed, in the host's words.
  String? get problem => _problem;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  Future<void> load() async {
    try {
      final answer = await _call('config/advanced', const {});
      final raw = answer is Map ? answer['tables'] : null;
      _tables = [if (raw is List) for (final each in raw) if (each is Map) AdvancedTable.read(each)];
      _revision = answer is Map ? answer['revision'] as String? : null;
      _problem = null;
    } on ControlFailed catch (error) {
      _problem = error.message;
    } finally {
      _loaded = true;
      notifyListeners();
    }
  }

  /// Makes the table or entry at [path] hold [value], or takes it out for null. Returns the host's words when it refuses, else null.
  Future<String?> set(List<String> path, Object? value) async {
    try {
      await _call('config/advanced_set', {'path': path, 'value': value, 'revision': ?_revision});
    } on ControlFailed catch (error) {
      await load();
      return error.message;
    }
    await load();
    return null;
  }
}

/// The values of a form as the host takes them: a member that holds free-form JSON is parsed from its text. Null names the member whose
/// text is not JSON.
({Map<String, Object?>? values, String? bad}) fromForm(Map<String, Object?> values, Set<String> json) {
  final out = <String, Object?>{};
  for (final MapEntry(:key, :value) in values.entries) {
    if (value == null || (value is String && value.trim().isEmpty) || (value is List && value.isEmpty)) continue;
    if (json.contains(key)) {
      try {
        out[key] = jsonDecode('$value');
      } on FormatException {
        return (values: null, bad: key);
      }
    } else {
      out[key] = value;
    }
  }
  return (values: out, bad: null);
}

/// The values the host holds as a form shows them.
Map<String, Object?> toForm(Map<String, Object?> held, Set<String> json) => {
  for (final MapEntry(:key, :value) in held.entries) key: json.contains(key) ? jsonEncode(value) : value,
};

/// Settings > Advanced: the tables of canoa.cfg no other page edits, each as a form made from its schema, with add, change and remove.
class AdvancedSection extends StatefulWidget {
  const AdvancedSection({super.key, required this.config});

  final AdvancedConfig config;

  @override
  State<AdvancedSection> createState() => _AdvancedSectionState();
}

class _AdvancedSectionState extends State<AdvancedSection> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.config.load());
  }

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    return ListenableBuilder(
      listenable: widget.config,
      builder: (context, _) {
        final config = widget.config;
        if (!config.loaded) return const Center(child: CircularProgressIndicator());
        return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(text('cfg.advanced_help'))),
          if (config.problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(problem, key: const Key('advanced-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          for (final table in config.tables)
            SettingsGroup(key: Key('advanced-table-${table.id}'), title: table.title, children: [_TableEditor(config: config, table: table)]),
        ]);
      },
    );
  }
}

class _TableEditor extends StatefulWidget {
  const _TableEditor({required this.config, required this.table});

  final AdvancedConfig config;
  final AdvancedTable table;

  @override
  State<_TableEditor> createState() => _TableEditorState();
}

class _TableEditorState extends State<_TableEditor> {
  /// What is being edited: an entry's name, '' for a flat table, an index for a row, or null while nothing is.
  String? _editing;
  bool _adding = false;
  Map<String, Object?> _values = {};
  final _name = TextEditingController();
  String? _problem;
  int _generation = 0;

  AdvancedTable get table => widget.table;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _open(String key, Map<String, Object?> held, {bool adding = false}) => setState(() {
    _editing = key;
    _adding = adding;
    _values = toForm(held, table.jsonMembers);
    _problem = null;
    _generation++;
  });

  void _close() => setState(() {
    _editing = null;
    _adding = false;
    _problem = null;
  });

  Future<void> _save() async {
    final text = AppText.of(context);
    final read = fromForm(_values, table.jsonMembers);
    if (read.bad != null) {
      setState(() => _problem = text('cfg.advanced_not_json', {'key': read.bad}));
      return;
    }
    final values = read.values!;
    final List<String> path;
    final Object? value;
    switch (table.shape) {
      case AdvancedShape.flat:
        path = table.path;
        value = values;
      case AdvancedShape.entries || AdvancedShape.pairs:
        final name = _adding ? _name.text.trim() : _editing!;
        if (name.isEmpty) {
          setState(() => _problem = text('cfg.advanced_name_needed'));
          return;
        }
        if (_adding && (table.ownEntries.containsKey(name) || table.inherited.containsKey(name))) {
          setState(() => _problem = text('cfg.advanced_name_taken', {'name': name}));
          return;
        }
        path = [...table.path, name];
        value = table.shape == AdvancedShape.pairs ? values['value'] ?? '' : values;
      case AdvancedShape.list:
        final rows = table.ownRows;
        if (_adding) {
          rows.add(values);
        } else {
          rows[int.parse(_editing!)] = values;
        }
        path = table.path;
        value = rows;
    }
    final refused = await widget.config.set(path, value);
    if (!mounted) return;
    if (refused != null) {
      setState(() => _problem = refused);
      return;
    }
    _close();
  }

  Future<void> _remove(String key) async {
    final text = AppText.of(context);
    final what = table.shape == AdvancedShape.entries || table.shape == AdvancedShape.pairs ? key : table.shape == AdvancedShape.list ? '${table.title}[$key]' : table.title;
    if (!await confirmRemoval(
      context,
      title: text('cfg.remove_title', {'name': what}),
      body: text('cfg.advanced_remove_body'),
    )) {
      return;
    }
    final List<String> path;
    final Object? value;
    switch (table.shape) {
      case AdvancedShape.flat:
        path = table.path;
        value = null;
      case AdvancedShape.entries || AdvancedShape.pairs:
        path = [...table.path, key];
        value = null;
      case AdvancedShape.list:
        final rows = table.ownRows..removeAt(int.parse(key));
        path = table.path;
        value = rows.isEmpty ? null : rows;
    }
    final refused = await widget.config.set(path, value);
    if (!mounted) return;
    setState(() => _problem = refused);
  }

  Widget _form(String idPrefix) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
    child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      if (_adding && (table.shape == AdvancedShape.entries || table.shape == AdvancedShape.pairs))
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: TextField(key: Key('$idPrefix-name'), controller: _name, decoration: InputDecoration(labelText: AppText.of(context)('cfg.advanced_name'))),
        ),
      SchemaFields(
        key: ValueKey('$idPrefix-$_generation'),
        schema: table.schema,
        form: const {},
        values: _values,
        idPrefix: idPrefix,
        onValues: (held) => _values = held,
      ),
      if (_problem != null)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(_problem!, key: Key('$idPrefix-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
      Padding(
        padding: const EdgeInsets.only(top: 8),
        child: Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          TextButton(key: Key('$idPrefix-cancel'), onPressed: _close, child: Text(AppText.of(context)('common.cancel'))),
          const SizedBox(width: 8),
          FilledButton(key: Key('$idPrefix-save'), onPressed: _save, child: Text(AppText.of(context)('common.save'))),
        ]),
      ),
    ]),
  );

  Widget _row(BuildContext context, {required String key, required String title, String? subtitle, required bool own, String? layer}) {
    final text = AppText.of(context);
    final id = 'advanced-${table.id}-${key.isEmpty ? 'table' : key}';
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      ListTile(
        key: Key(id),
        title: Text(title),
        subtitle: Text(own ? (subtitle ?? '') : text('cfg.from_layer', {'layer': layer}), key: Key('$id-note')),
        trailing: own
            ? Row(mainAxisSize: MainAxisSize.min, children: [
                IconButton(key: Key('$id-edit'), icon: const Icon(Icons.edit_outlined), tooltip: text('cfg.advanced_edit'), onPressed: () => _open(key, _heldOf(key))),
                IconButton(key: Key('$id-remove'), icon: const Icon(Icons.delete_outline), tooltip: text('cfg.remove', {'name': title}), onPressed: () => _remove(key)),
              ])
            : null,
      ),
      if (_editing == key && !_adding) _form('advanced-${table.id}'),
    ]);
  }

  Map<String, Object?> _heldOf(String key) => switch (table.shape) {
    AdvancedShape.list => table.ownRows[int.parse(key)],
    _ => table.ownEntries[key] ?? const {},
  };

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    final rows = <Widget>[];
    switch (table.shape) {
      case AdvancedShape.flat:
        final mine = table.ownEntries[''] ?? const {};
        rows.add(_row(context, key: '', title: table.title, subtitle: mine.isEmpty ? text('cfg.advanced_nothing_set') : mine.keys.join(', '), own: true));
        // A value a lower layer gives and the person's file does not.
        for (final MapEntry(:key, :value) in table.inherited.entries) {
          if (mine.containsKey(key)) continue;
          rows.add(ListTile(
            key: Key('advanced-${table.id}-inherited-$key'),
            dense: true,
            title: Text('$key = ${jsonEncode(value.value)}'),
            subtitle: Text(text('cfg.from_layer_note', {'layer': value.layer})),
          ));
        }
      case AdvancedShape.entries || AdvancedShape.pairs:
        final mine = table.ownEntries;
        for (final name in [...mine.keys, ...table.inherited.keys.where((name) => !mine.containsKey(name))]) {
          rows.add(_row(context, key: name, title: name, subtitle: mine[name]?.keys.join(', '), own: mine.containsKey(name), layer: table.inherited[name]?.layer));
        }
      case AdvancedShape.list:
        final mine = table.ownRows;
        for (var at = 0; at < mine.length; at++) {
          rows.add(_row(context, key: '$at', title: '${mine[at]['label'] ?? mine[at]['id'] ?? at + 1}', subtitle: mine[at].keys.join(', '), own: true));
        }
        for (final MapEntry(:value) in table.inherited.entries) {
          rows.add(ListTile(key: Key('advanced-${table.id}-inherited'), dense: true, title: Text(jsonEncode(value.value)), subtitle: Text(text('cfg.from_layer_note', {'layer': value.layer}))));
        }
    }
    final addable = table.shape != AdvancedShape.flat;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      ...rows,
      if (_adding) _form('advanced-${table.id}'),
      if (_problem != null && !_adding && _editing == null)
        Padding(padding: const EdgeInsets.all(12), child: Text(_problem!, key: Key('advanced-${table.id}-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error))),
      if (addable && !_adding)
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: Key('advanced-${table.id}-add'),
            icon: const Icon(Icons.add),
            label: Text(text('cfg.advanced_add')),
            onPressed: () {
              _name.clear();
              _open('', const {}, adding: true);
            },
          ),
        ),
    ]);
  }
}
