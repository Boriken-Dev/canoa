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
    final type = _typeOf(field);
    if ((type == 'integer' || type == 'number') && (value is! int || value < 0)) return '${_titleOf(schema, key)} is a whole number';
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
    return SettingsRow(title: title, description: description, control: _control(key));
  }

  @override
  Widget build(BuildContext context) {
    final advanced = _keys(advanced: true);
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
