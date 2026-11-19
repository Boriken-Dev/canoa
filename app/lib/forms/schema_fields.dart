/// The rows of a form a provider describes with a guatiao schema (a JSON Schema whose members name their `x-section`, `x-order` and
/// `x-advanced`, and mark a secret `x-sensitive` or `writeOnly`) and a guatiao-intake form document (sections, and for a member its
/// widget, placeholder, unit and `visibleWhen`). Speech providers, profile items and catalog sources all describe their values this
/// way, so one renderer draws them all and knows nothing of the provider behind it.
library;

import 'package:flutter/material.dart';

import '../settings/settings_row.dart';

/// A member's schema and its form hints, read the way the rows are drawn from them.
Map<String, Object?> _map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};

/// The members of [schema], by name.
Map<String, Object?> schemaProperties(Map<String, Object?> schema) => _map(schema['properties']);

/// The names [schema] says a value must have.
Set<String> schemaRequired(Map<String, Object?> schema) => {if (schema['required'] is List) for (final each in schema['required'] as List) if (each is String) each};

/// The type word of a schema member, `null` allowed beside it.
String _typeOf(Map<String, Object?> field) {
  final type = field['type'];
  if (type is String) return type;
  if (type is List) return type.firstWhere((t) => t != 'null', orElse: () => 'string').toString();
  return 'string';
}

bool _secret(Map<String, Object?> field) => field['writeOnly'] == true || field['x-sensitive'] == true;

String _titleOf(Map<String, Object?> schema, String key) {
  final title = _map(schemaProperties(schema)[key])['title'];
  return title is String && title.isNotEmpty ? title : key;
}

/// The schema of one entry of the array member [field], when each entry is an object (a list of rows), else null.
Map<String, Object?>? _entrySchema(Map<String, Object?> field) {
  if (_typeOf(field) != 'array') return null;
  final items = _map(field['items']);
  return _typeOf(items) == 'object' && items['properties'] is Map ? items : null;
}

/// The form hints of one entry of the array member [key]: the nested `form` a member's hints carry.
Map<String, Object?> _entryForm(Map<String, Object?> form, String key) => _map(_map(_map(form['fields'])[key])['form']);

/// Whether [form], or a form nested in it, gives any member the widget hint [widget].
bool formHasWidget(Map<String, Object?> form, String widget) {
  for (final hints in _map(form['fields']).values) {
    final each = _map(hints);
    if (each['widget'] == widget || formHasWidget(_map(each['form']), widget)) return true;
  }
  return false;
}

/// Whether [row], one entry of a list, holds nothing the person gave: every member is unset, empty, or what its default says.
bool _blankRow(Map<String, Object?> entry, Map<String, Object?> row) {
  final properties = schemaProperties(entry);
  for (final MapEntry(:key, :value) in row.entries) {
    if (value == null || (value is String && value.trim().isEmpty)) continue;
    if (value == _map(properties[key])['default']) continue;
    return false;
  }
  return true;
}

/// One of the values a member with a widget hint may be given, offered by whoever knows them (the host).
class SchemaChoice {
  const SchemaChoice({required this.value, required this.label, this.detail});

  /// What is kept.
  final String value;

  /// What the person reads as the choice.
  final String label;

  /// A second line under the label in the open list.
  final String? detail;
}

/// Whether the member [key] is shown for [values]: its form hint's `visibleWhen` names another member and what it must hold.
bool schemaVisible(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values, String key) {
  final when = _map(_map(form['fields'])[key])['visibleWhen'];
  if (when is! Map) return true;
  final other = values[when['field']] ?? _map(schemaProperties(schema)['${when['field']}'])['default'];
  if (when.containsKey('equals')) return other == when['equals'];
  final within = when['in'];
  return within is List ? within.contains(other) : true;
}

/// What a form holds of [values] that is to be kept: the shown members that have a value (a secret kept as typed, the rest only when
/// not empty).
Map<String, Object?> collectSchemaValues(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values) {
  final out = <String, Object?>{};
  final properties = schemaProperties(schema);
  for (final key in properties.keys) {
    if (!schemaVisible(schema, form, values, key)) continue;
    final value = values[key];
    if (value == null || (value is String && value.isEmpty && !_secret(_map(properties[key])))) continue;
    final entry = _entrySchema(_map(properties[key]));
    if (entry != null && value is List) {
      final entryForm = _entryForm(form, key);
      final rows = [
        for (final row in value)
          if (row is Map && !_blankRow(entry, row.cast<String, Object?>())) collectSchemaValues(entry, entryForm, row.cast<String, Object?>()),
      ];
      if (rows.isNotEmpty) out[key] = rows;
      continue;
    }
    out[key] = value;
  }
  return out;
}

/// Why [values] cannot be kept, in the words of the form: a shown member the schema requires has no value, or a whole number is not
/// one. None when they can.
String? schemaProblem(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values) {
  final properties = schemaProperties(schema);
  final required = schemaRequired(schema);
  for (final key in properties.keys) {
    if (!schemaVisible(schema, form, values, key)) continue;
    final value = values[key];
    final field = _map(properties[key]);
    if (value == null || (value is String && value.trim().isEmpty && !_secret(field))) {
      if (required.contains(key) && field['default'] == null) return '${_titleOf(schema, key)} is needed';
      continue;
    }
    final entry = _entrySchema(field);
    if (entry != null && value is List) {
      for (final row in value) {
        if (row is! Map || _blankRow(entry, row.cast<String, Object?>())) continue;
        final why = schemaProblem(entry, _entryForm(form, key), row.cast<String, Object?>());
        if (why != null) return '${_titleOf(schema, key)}: $why';
      }
      continue;
    }
    final type = _typeOf(field);
    if ((type == 'integer'|| type == 'number') && (value is! int || value < 0)) return '${_titleOf(schema, key)} is a whole number';
  }
  return null;
}

/// The rows of one form: a row for each member the schema has and the form shows, the advanced ones under Advanced.
///
/// The widget keeps what is typed; [onValues] is told the members' values at every change, and the caller reads what is to be kept
/// with [collectSchemaValues]. A secret the host already holds is not shown ([secretsSet] names it); the row says it is set, and a
/// button asks to take it out ([cleared] and [onClear]). Keys are `<idPrefix>-field-<member>`, `<idPrefix>-clear-<member>` and
/// `<idPrefix>-advanced`.
class SchemaFields extends StatefulWidget {
  const SchemaFields({
    super.key,
    required this.schema,
    required this.form,
    required this.values,
    required this.onValues,
    this.idPrefix = 'form',
    this.secretsSet = const {},
    this.cleared = const {},
    this.onClear,
    this.onSubmitted,
    this.unsetEntry = false,
    this.keys = const {},
    this.choices = const {},
    this.compact = false,
  });

  final Map<String, Object?> schema;
  final Map<String, Object?> form;

  /// What the form starts with.
  final Map<String, Object?> values;
  final ValueChanged<Map<String, Object?>> onValues;
  final String idPrefix;
  final Set<String> secretsSet;
  final Set<String> cleared;
  final void Function(String key, {required bool clear})? onClear;

  /// Called when Enter is pressed in a one-line text box.
  final VoidCallback? onSubmitted;

  /// Whether a choice has an entry that leaves it unset, which a member the schema does not require may be.
  final bool unsetEntry;

  /// The key of a member's control where it is not `<idPrefix>-field-<member>`.
  final Map<String, String> keys;

  /// The values a member may be given, by the widget hint that asks for them: a member whose form says `widget: "<name>"` is drawn as
  /// a pick from `choices["<name>"]` when that is given, else as text.
  final Map<String, List<SchemaChoice>> choices;

  /// Draws each member as a labelled control side by side, as the entry of a list is, instead of one settings row each.
  final bool compact;

  @override
  State<SchemaFields> createState() => _SchemaFieldsState();
}

class _SchemaFieldsState extends State<SchemaFields> {
  late final Map<String, Object?> _values = {...widget.values};
  final _typed = <String, TextEditingController>{};

  Map<String, Object?> get _properties => schemaProperties(widget.schema);
  Map<String, Object?> _field(String key) => _map(_properties[key]);
  Map<String, Object?> _hints(String key) => _map(_map(widget.form['fields'])[key]);

  @override
  void dispose() {
    for (final controller in _typed.values) {
      controller.dispose();
    }
    super.dispose();
  }

  bool _visible(String key) => schemaVisible(widget.schema, widget.form, _values, key);

  TextEditingController _controller(String key, String initial) => _typed.putIfAbsent(key, () => TextEditingController(text: initial));

  void _set(String key, Object? value) {
    setState(() => _values[key] = value);
    widget.onValues({..._values});
  }

  Widget _control(String key) {
    final field = _field(key);
    final hints = _hints(key);
    final type = _typeOf(field);
    final unit = hints['unit'] is String ? hints['unit']! as String : null;
    final placeholder = hints['placeholder'] is String ? hints['placeholder']! as String : null;
    final secret = _secret(field);
    final id = widget.keys[key] ?? '${widget.idPrefix}-field-$key';
    final picks = hints['widget'] is String ? widget.choices[hints['widget']] : null;
    if (picks != null) return _pick(id, key, picks);
    if (type == 'boolean' && widget.unsetEntry && !schemaRequired(widget.schema).contains(key)) {
      return DropdownButton<bool?>(
        key: Key(id),
        value: _values[key] as bool?,
        items: const [
          DropdownMenuItem<bool?>(value: null, child: Text('Not set')),
          DropdownMenuItem<bool?>(value: true, child: Text('Yes')),
          DropdownMenuItem<bool?>(value: false, child: Text('No')),
        ],
        onChanged: (choice) => _set(key, choice),
      );
    }
    if (type == 'boolean') {
      return Switch(
        key: Key(id),
        value: _values[key] == true || (!_values.containsKey(key) && field['default'] == true),
        onChanged: (on) => _set(key, on),
      );
    }
    if (field['enum'] is List) {
      final choices = (field['enum']! as List).whereType<String>().toList();
      final unsettable = widget.unsetEntry && !schemaRequired(widget.schema).contains(key);
      final dflt = field['default'] as String?;
      return DropdownButton<String?>(
        key: Key(id),
        value: choices.contains(_values[key]) ? _values[key]! as String : (unsettable ? null : dflt),
        items: [
          if (unsettable) DropdownMenuItem<String?>(value: null, child: Text(dflt == null ? 'Not set' : 'Default ($dflt)')),
          for (final choice in choices) DropdownMenuItem<String?>(value: choice, child: Text(choice)),
        ],
        onChanged: (choice) => _set(key, choice),
      );
    }
    final isList = type == 'array';
    final isNumber = type == 'integer' || type == 'number';
    final current = _values[key];
    final initial = isList
        ? (current is List ? current.join('\n') : '')
        : (current?.toString() ?? (isNumber && field['default'] != null ? field['default'].toString() : ''));
    final set = secret && widget.secretsSet.contains(key) && !widget.cleared.contains(key);
    return SizedBox(
      width: 280,
      child: TextField(
        key: Key(id),
        controller: _controller(key, secret ? '' : initial),
        obscureText: secret || hints['widget'] == 'password',
        minLines: hints['widget'] == 'textarea' ? 3 : 1,
        maxLines: isList ? 4 : (hints['widget'] == 'textarea' ? 6 : 1),
        keyboardType: isNumber ? TextInputType.number : TextInputType.text,
        decoration: InputDecoration(
          isDense: true,
          hintText: set ? 'Set. Type to replace' : (placeholder ?? (field['default'] != null && !isNumber ? '${field['default']}' : null)),
          suffixText: unit,
          suffixIcon: set
              ? IconButton(
                  key: Key('${widget.idPrefix}-clear-$key'),
                  tooltip: 'Remove it',
                  icon: const Icon(Icons.clear, size: 18),
                  onPressed: () => widget.onClear?.call(key, clear: true),
                )
              : null,
        ),
        onSubmitted: isList || widget.onSubmitted == null ? null : (_) => widget.onSubmitted!(),
        onChanged: (text) {
          if (secret) {
            widget.onClear?.call(key, clear: false);
            _values[key] = text;
          } else if (isList) {
            _values[key] = [for (final line in text.split('\n')) if (line.trim().isNotEmpty) line.trim()];
          } else if (isNumber) {
            final number = int.tryParse(text.trim());
            _values[key] = number ?? (text.trim().isEmpty ? null : text.trim());
          } else {
            _values[key] = text.isEmpty ? null : text;
          }
          widget.onValues({..._values});
        },
      ),
    );
  }

  /// A pick from [picks]; a value that is kept and no longer offered stays chosen and says it is not available.
  Widget _pick(String id, String key, List<SchemaChoice> picks) {
    final held = (_values[key] ?? _field(key)['default'])?.toString();
    final all = [
      ...picks,
      if (held != null && !picks.any((each) => each.value == held)) SchemaChoice(value: held, label: '$held (not available)'),
    ];
    final text = Theme.of(context).textTheme;
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    return SizedBox(
      width: 320,
      child: DropdownButton<String>(
        key: Key(id),
        isExpanded: true,
        itemHeight: null,
        value: all.any((each) => each.value == held) ? held : all.first.value,
        selectedItemBuilder: (context) => [
          for (final each in all) Align(alignment: Alignment.centerLeft, child: Text(each.label, overflow: TextOverflow.ellipsis)),
        ],
        items: [
          for (final each in all)
            DropdownMenuItem<String>(
              value: each.value,
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 6),
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(each.label, overflow: TextOverflow.ellipsis),
                  if (each.detail != null) Text(each.detail!, style: text.bodySmall?.copyWith(color: muted), overflow: TextOverflow.ellipsis),
                ]),
              ),
            ),
        ],
        onChanged: (choice) => _set(key, choice),
      ),
    );
  }

  List<String> _keys({required bool advanced}) {
    final sections = <String>[];
    final listed = widget.form['sections'];
    if (listed is List) {
      for (final section in listed) {
        if (section is Map && section['id'] is String) sections.add(section['id']! as String);
      }
    }
    int rank(String key) {
      final section = _field(key)['x-section'] is String ? _field(key)['x-section']! as String : '';
      final at = sections.indexOf(section);
      return at < 0 ? sections.length : at;
    }

    final keys = [
      for (final key in _properties.keys)
        if ((_field(key)['x-advanced'] == true) == advanced && _visible(key)) key,
    ];
    keys.sort((a, b) => rank(a).compareTo(rank(b)));
    return keys;
  }

  Widget _row(String key) {
    final field = _field(key);
    final title = field['title'] is String ? field['title']! as String : key;
    final description = field['description'] is String ? field['description']! as String : null;
    final entry = _entrySchema(field);
    if (entry != null) {
      final id = widget.keys[key] ?? '${widget.idPrefix}-field-$key';
      final held = _values[key];
      return SettingsRow(
        title: title,
        description: description,
        below: SchemaRows(
          key: Key(id),
          entry: entry,
          form: _entryForm(widget.form, key),
          values: [if (held is List) for (final each in held) if (each is Map) each.cast<String, Object?>()],
          choices: widget.choices,
          onValues: (rows) => _set(key, rows),
        ),
      );
    }
    if (widget.compact) {
      return Padding(
        padding: const EdgeInsets.only(right: 12, bottom: 4),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(title, style: Theme.of(context).textTheme.labelMedium),
          const SizedBox(height: 4),
          Tooltip(message: description ?? '', child: _control(key)),
        ]),
      );
    }
    return SettingsRow(title: title, description: description, control: _control(key));
  }

  @override
  Widget build(BuildContext context) {
    final advanced = _keys(advanced: true);
    if (widget.compact) return Wrap(crossAxisAlignment: WrapCrossAlignment.end, children: [for (final key in _keys(advanced: false)) _row(key)]);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final key in _keys(advanced: false)) _row(key),
      if (advanced.isNotEmpty)
        ExpansionTile(
          key: Key('${widget.idPrefix}-advanced'),
          title: const Text('Advanced'),
          children: [for (final key in advanced) _row(key)],
        ),
    ]);
  }
}

/// The entries of a list of objects as rows: each is drawn from the members of [entry], and the rows can be added, removed and moved,
/// since a list is read in order. [onValues] is told every row's values at each change, blank rows included.
///
/// With this widget's key `<id>`, keys are `<id>-add`, and for the row at position n `<id>-row-n`, `<id>-up-n`, `<id>-down-n`,
/// `<id>-remove-n`, and the row's members `<id>-n-field-<member>`.
class SchemaRows extends StatefulWidget {
  const SchemaRows({
    super.key,
    required this.entry,
    required this.form,
    required this.values,
    required this.onValues,
    this.choices = const {},
  });

  /// The schema of one entry, an object.
  final Map<String, Object?> entry;

  /// How an entry is drawn.
  final Map<String, Object?> form;
  final List<Map<String, Object?>> values;
  final ValueChanged<List<Map<String, Object?>>> onValues;
  final Map<String, List<SchemaChoice>> choices;

  @override
  State<SchemaRows> createState() => _SchemaRowsState();
}

class _SchemaRowsState extends State<SchemaRows> {
  var _next = 0;
  late final List<(int, Map<String, Object?>)> _rows = [for (final each in widget.values) (_next++, {...each})];

  String get _id => switch (widget.key) {
    ValueKey<String>(:final value) => value,
    _ => 'rows',
  };

  void _told() => widget.onValues([for (final (_, row) in _rows) {...row}]);

  void _changed() {
    setState(() {});
    _told();
  }

  /// A new row holds the members' defaults, so what is shown chosen is what is kept.
  Map<String, Object?> _fresh() => {
    for (final MapEntry(:key, :value) in schemaProperties(widget.entry).entries)
      if (_map(value)['default'] != null) key: _map(value)['default'],
  };

  void _move(int at, int by) {
    _rows.insert(at + by, _rows.removeAt(at));
    _changed();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (var at = 0; at < _rows.length; at++)
        Container(
          key: Key('$_id-row-$at'),
          margin: const EdgeInsets.only(bottom: 8),
          padding: const EdgeInsets.fromLTRB(12, 8, 4, 4),
          decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(8)),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Expanded(
              child: SchemaFields(
                key: ValueKey(_rows[at].$1),
                schema: widget.entry,
                form: widget.form,
                values: _rows[at].$2,
                idPrefix: '$_id-$at',
                choices: widget.choices,
                compact: true,
                onValues: (held) {
                  final row = _rows[at].$2;
                  row
                    ..clear()
                    ..addAll(held);
                  _told();
                },
              ),
            ),
            IconButton(
              key: Key('$_id-up-$at'),
              tooltip: 'Move up: the first entry that matches decides',
              icon: const Icon(Icons.arrow_upward, size: 18),
              onPressed: at == 0 ? null : () => _move(at, -1),
            ),
            IconButton(
              key: Key('$_id-down-$at'),
              tooltip: 'Move down',
              icon: const Icon(Icons.arrow_downward, size: 18),
              onPressed: at == _rows.length - 1 ? null : () => _move(at, 1),
            ),
            IconButton(
              key: Key('$_id-remove-$at'),
              tooltip: 'Remove',
              icon: const Icon(Icons.delete_outline, size: 18),
              onPressed: () {
                _rows.removeAt(at);
                _changed();
              },
            ),
          ]),
        ),
      Align(
        alignment: Alignment.centerLeft,
        child: TextButton.icon(
          key: Key('$_id-add'),
          icon: const Icon(Icons.add, size: 18),
          label: const Text('Add'),
          onPressed: () {
            _rows.add((_next++, _fresh()));
            _changed();
          },
        ),
      ),
    ]);
  }
}
