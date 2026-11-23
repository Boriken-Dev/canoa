/// The rows of a form a provider describes with a guatiao schema (a JSON Schema whose members name their `x-section`, `x-order` and
/// `x-advanced`, and mark a secret `x-sensitive` or `writeOnly`, or, for a secret another provider may give, `x-from-secret`) and a
/// guatiao-intake form document (sections, and for a member its widget, placeholder, unit and `visibleWhen`). Speech providers,
/// profile items, catalog sources and secret providers all describe their values this way, so one renderer draws them all and knows
/// nothing of the provider behind it.
library;

import 'package:material_ui/material_ui.dart';

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

/// Whether the member is another secret, taken from a provider's entry rather than typed or written down.
bool _fromSecret(Map<String, Object?> field) => field['x-from-secret'] == true;

bool _isNumeric(String type) => type == 'integer' || type == 'number';

/// What [text] says as a number of [type] (`integer` or `number`), or null when it is not one: JSON Schema's integers include 3.0.
num? _parseNumber(String type, String text) {
  final number = num.tryParse(text.trim());
  if (number == null || !number.isFinite) return null;
  if (type == 'integer') return number == number.truncate() ? number.truncate() : null;
  return number;
}

/// Why [value] is not a number the member [field] of [type] accepts, in the words that follow its title, or null.
String? _numberProblem(Map<String, Object?> field, String type, Object? value) {
  final number = value is num && value.isFinite ? (type == 'integer' ? (value == value.truncate() ? value.truncate() : null) : value) : null;
  if (number == null) return type == 'integer' ? 'is a whole number' : 'is a number';
  final minimum = field['minimum'], maximum = field['maximum'];
  final above = field['exclusiveMinimum'], below = field['exclusiveMaximum'];
  if (minimum is num && number < minimum) return 'is at least $minimum';
  if (maximum is num && number > maximum) return 'is at most $maximum';
  if (above is num && number <= above) return 'is more than $above';
  if (below is num && number >= below) return 'is less than $below';
  return null;
}

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

/// A setting that is another secret: the entry that gives it and the secret's name there. A member its schema marks `x-from-secret` is
/// drawn as a choice of this, since such a setting is never typed into the form or written down.
final class SecretSource {
  const SecretSource(this.entry, this.name);

  final String entry, name;

  @override
  bool operator ==(Object other) => other is SecretSource && other.entry == entry && other.name == name;

  @override
  int get hashCode => Object.hash(entry, name);
}

/// An entry that can give a secret to a setting, as the form names it to the person.
final class SecretSourceChoice {
  const SecretSourceChoice(this.entry, this.label);

  final String entry, label;
}

/// The member a condition or a hint names by [path]: a member by its name, or by the store's spelling of a one-segment path
/// (`[a.b]`, `["a.b"]`).
String _memberKey(Map<String, Object?> schema, String path) {
  final properties = schemaProperties(schema);
  if (properties.containsKey(path)) return path;
  final spelled = RegExp(r'^\[(?:"(.*)"|(.*))\]$').firstMatch(path);
  final inner = spelled == null ? null : (spelled.group(1) ?? spelled.group(2));
  return inner != null && properties.containsKey(inner) ? inner : path;
}

/// Whether the member [key] is shown for [values], by guatiao-intake's rule. With no condition it is. Otherwise it is when the member
/// its `visibleWhen` reads is itself shown and holds `equals` (or one of `in`); a member holding nothing reads as its schema's default,
/// and one with neither does not meet the condition, so hiding a member hides everything that waits on it. A chain of conditions that
/// leads back to itself, or to a member the schema does not have, is not shown.
bool schemaVisible(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values, String key) {
  final properties = schemaProperties(schema);
  final hints = _map(form['fields']);
  final conditions = <Map>[];
  final seen = <String>{};
  var at = _memberKey(schema, key);
  while (true) {
    if (!properties.containsKey(at) || !seen.add(at)) return false;
    final when = _map(hints[at])['visibleWhen'];
    if (when is! Map) break;
    conditions.add(when);
    at = _memberKey(schema, '${when['field']}');
  }
  return conditions.every((when) {
    final read = _memberKey(schema, '${when['field']}');
    final held = values[read] ?? _map(properties[read])['default'];
    if (held == null) return false;
    if (when.containsKey('equals')) return held == when['equals'];
    final within = when['in'];
    return within is List && within.contains(held);
  });
}

/// Members a person sees together: a section of the form, or the default section, and the members in the order they are shown.
class SchemaGroup {
  const SchemaGroup({required this.section, required this.title, required this.help, required this.keys});

  /// The section's id; `''` is the default section.
  final String section;

  /// What the form calls the section, empty when it says nothing.
  final String title, help;

  /// The members, whether or not they are shown now.
  final List<String> keys;
}

/// The members of [schema] grouped into the sections [form] declares and put in order, by guatiao-intake's rule: the sections in the
/// form's order with the default section (`''`) first unless the form places it; a member whose `x-section` the form does not declare
/// joins the default section; within a section the members with an `x-order` come first, lowest first, then the rest as the schema
/// declares them; a section with no members is left out. Which members are shown now is [schemaVisible]'s.
List<SchemaGroup> schemaLayout(Map<String, Object?> schema, Map<String, Object?> form) {
  final declared = <String, Map<String, Object?>>{};
  final listed = form['sections'];
  if (listed is List) {
    for (final section in listed) {
      if (section is Map && section['id'] is String) declared.putIfAbsent(section['id']! as String, () => section.cast<String, Object?>());
    }
  }
  final ids = [if (!declared.containsKey('')) '', ...declared.keys];
  final into = {for (final id in ids) id: <String>[]};
  for (final MapEntry(:key, :value) in schemaProperties(schema).entries) {
    final named = _map(value)['x-section'];
    into[named is String && into.containsKey(named) ? named : '']!.add(key);
  }
  int? order(String key) {
    final at = _map(schemaProperties(schema)[key])['x-order'];
    return at is num ? at.toInt() : null;
  }

  final groups = <SchemaGroup>[];
  for (final id in ids) {
    final keys = into[id]!;
    if (keys.isEmpty) continue;
    final position = {for (var i = 0; i < keys.length; i++) keys[i]: i};
    final sorted = [...keys]
      ..sort((a, b) {
        final first = order(a), second = order(b);
        if (first != null && second != null && first != second) return first.compareTo(second);
        if ((first == null) != (second == null)) return first == null ? 1 : -1;
        return position[a]!.compareTo(position[b]!);
      });
    final section = declared[id];
    groups.add(SchemaGroup(
      section: id,
      title: section?['title'] is String ? section!['title']! as String : '',
      help: section?['description'] is String ? section!['description']! as String : '',
      keys: sorted,
    ));
  }
  return groups;
}

/// What a form holds of [values] that is to be kept: the shown members that have a value (a secret kept as typed, the rest only when
/// not empty).
Map<String, Object?> collectSchemaValues(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values) {
  final out = <String, Object?>{};
  final properties = schemaProperties(schema);
  for (final key in properties.keys) {
    if (!schemaVisible(schema, form, values, key) || _fromSecret(_map(properties[key]))) continue;
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

/// Why [values] cannot be kept, in the words of the form: a shown member the schema requires has no value, or a number is not one the
/// schema accepts (a whole number for an `integer`, within its `minimum` and `maximum`). None when they can.
String? schemaProblem(Map<String, Object?> schema, Map<String, Object?> form, Map<String, Object?> values) {
  final properties = schemaProperties(schema);
  final required = schemaRequired(schema);
  for (final key in properties.keys) {
    final field = _map(properties[key]);
    if (!schemaVisible(schema, form, values, key) || _fromSecret(field)) continue;
    final value = values[key];
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
    if (_isNumeric(type)) {
      final why = _numberProblem(field, type, value);
      if (why != null) return '${_titleOf(schema, key)} $why';
    }
  }
  return null;
}

/// The rows of one form: a row for each member the schema has and the form shows, the advanced ones under Advanced.
///
/// The widget keeps what is typed; [onValues] is told the members' values at every change, and the caller reads what is to be kept
/// with [collectSchemaValues]. A secret the host already holds is not shown ([secretsSet] names it); the row says it is set, and a
/// button asks to take it out ([cleared] and [onClear]). A member marked `x-from-secret` is a choice of the entry in [sourceChoices]
/// that gives it and the secret's name there ([sources], [onSource]), never a box to type it in. Keys are `<idPrefix>-field-<member>`,
/// `<idPrefix>-clear-<member>`, `<idPrefix>-advanced`, and for a secret's source `<idPrefix>-from-<member>` and
/// `<idPrefix>-from-name-<member>`.
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
    this.sources = const {},
    this.sourceChoices = const [],
    this.onSource,
    this.sourceNamePrefix = '',
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

  /// Where each member marked `x-from-secret` comes from now.
  final Map<String, SecretSource> sources;

  /// The entries that may give such a member.
  final List<SecretSourceChoice> sourceChoices;

  /// Told where a member marked `x-from-secret` now comes from: null when it is not set.
  final void Function(String key, SecretSource? source)? onSource;

  /// What the name of a secret starts with when an entry is first chosen for a member: `<prefix>_<member>`.
  final String sourceNamePrefix;

  @override
  State<SchemaFields> createState() => _SchemaFieldsState();
}

class _SchemaFieldsState extends State<SchemaFields> {
  late final Map<String, Object?> _values = {...widget.values};
  late final Map<String, SecretSource?> _from = {...widget.sources};
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

  /// The choice of where a secret member comes from: not set, or an entry and the name of the secret there.
  Widget _sourceControl(String key) {
    final id = widget.idPrefix;
    final source = _from[key];
    final fallback = widget.sourceNamePrefix.isEmpty ? key : '${widget.sourceNamePrefix}_$key';
    final entries = [for (final choice in widget.sourceChoices) choice.entry];
    final chosen = source != null && entries.contains(source.entry) ? source.entry : null;
    void told(SecretSource? now) {
      _from[key] = now;
      widget.onSource?.call(key, now);
    }

    return SizedBox(
      width: 280,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
        DropdownButton<String?>(
          key: Key('$id-from-$key'),
          isExpanded: true,
          value: chosen,
          hint: const Text('Not set'),
          items: [
            const DropdownMenuItem<String?>(value: null, child: Text('Not set')),
            for (final choice in widget.sourceChoices) DropdownMenuItem<String?>(value: choice.entry, child: Text(choice.label)),
          ],
          onChanged: (entry) => setState(() {
            if (entry == null) return told(null);
            told(SecretSource(entry, _controller('$key/name', source?.name ?? fallback).text.trim()));
          }),
        ),
        if (chosen != null)
          TextField(
            key: Key('$id-from-name-$key'),
            controller: _controller('$key/name', source?.name ?? fallback),
            decoration: const InputDecoration(isDense: true, labelText: 'Secret name', helperText: 'What it is called in that provider'),
            onChanged: (text) => told(SecretSource(chosen, text.trim())),
          ),
      ]),
    );
  }

  Widget _control(String key) {
    final field = _field(key);
    if (_fromSecret(field)) return _sourceControl(key);
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
      final choices = [for (final each in field['enum']! as List) ?each];
      final unsettable = widget.unsetEntry && !schemaRequired(widget.schema).contains(key);
      final dflt = field['default'];
      return DropdownButton<Object?>(
        key: Key(id),
        value: choices.contains(_values[key]) ? _values[key] : (unsettable ? null : (choices.contains(dflt) ? dflt : null)),
        items: [
          if (unsettable) DropdownMenuItem<Object?>(value: null, child: Text(dflt == null ? 'Not set' : 'Default ($dflt)')),
          for (final choice in choices) DropdownMenuItem<Object?>(value: choice, child: Text('$choice')),
        ],
        onChanged: (choice) => _set(key, choice),
      );
    }
    final isList = type == 'array';
    final isNumber = _isNumeric(type);
    final current = _values[key];
    final initial = isList
        ? (current is List ? current.join('\n') : '')
        : (current?.toString() ?? (isNumber && field['default'] != null ? field['default'].toString() : ''));
    final set = secret && widget.secretsSet.contains(key) && !widget.cleared.contains(key);
    final controller = _controller(key, secret ? '' : initial);
    void typed(String text) {
      if (secret) {
        widget.onClear?.call(key, clear: false);
        _values[key] = text;
      } else if (isList) {
        _values[key] = [for (final line in text.split('\n')) if (line.trim().isNotEmpty) line.trim()];
      } else if (isNumber) {
        final number = _parseNumber(type, text);
        _values[key] = number ?? (text.trim().isEmpty ? null : text.trim());
      } else {
        _values[key] = text.isEmpty ? null : text;
      }
      widget.onValues({..._values});
    }

    // The schema's examples are suggestions: a tap fills the box, and any other text is as good.
    final examples = field['examples'] is List && !secret && !isList && !isNumber ? [for (final each in field['examples']! as List) if (each is String) each] : const <String>[];
    return SizedBox(
      width: 280,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
        TextField(
          key: Key(id),
          controller: controller,
          obscureText: secret || hints['widget'] == 'password',
          minLines: hints['widget'] == 'textarea' ? 3 : 1,
          maxLines: isList ? 4 : (hints['widget'] == 'textarea' ? 6 : 1),
          keyboardType: isNumber ? TextInputType.numberWithOptions(signed: true, decimal: type == 'number') : TextInputType.text,
          decoration: InputDecoration(
            isDense: true,
            hintText: set ? 'Set. Type to replace' : (placeholder ?? (field['default'] != null && !isNumber && !isList ? '${field['default']}' : null)),
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
          onChanged: typed,
        ),
        if (examples.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Wrap(spacing: 6, runSpacing: 4, children: [
              for (final each in examples)
                ActionChip(
                  key: Key('${widget.idPrefix}-suggest-$key-$each'),
                  label: Text(each),
                  visualDensity: VisualDensity.compact,
                  onPressed: () {
                    controller.text = each;
                    typed(each);
                  },
                ),
            ]),
          ),
      ]),
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

  /// The members shown now, in the layout's order, [advanced] ones or the rest, by section.
  List<SchemaGroup> _shown({required bool advanced}) {
    final groups = <SchemaGroup>[];
    for (final group in schemaLayout(widget.schema, widget.form)) {
      final keys = [for (final key in group.keys) if ((_field(key)['x-advanced'] == true) == advanced && _visible(key)) key];
      if (keys.isNotEmpty) groups.add(SchemaGroup(section: group.section, title: group.title, help: group.help, keys: keys));
    }
    return groups;
  }

  Widget _heading(SchemaGroup group) {
    final text = Theme.of(context).textTheme;
    return Padding(
      key: Key('${widget.idPrefix}-section-${group.section}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Semantics(header: true, child: Text(group.title, style: text.titleSmall)),
        if (group.help.isNotEmpty) Text(group.help, style: text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant)),
      ]),
    );
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
    final basic = _shown(advanced: false);
    final advanced = [for (final group in _shown(advanced: true)) ...group.keys];
    if (widget.compact) return Wrap(crossAxisAlignment: WrapCrossAlignment.end, children: [for (final group in basic) for (final key in group.keys) _row(key)]);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final group in basic) ...[
        if (group.title.isNotEmpty) _heading(group),
        for (final key in group.keys) _row(key),
      ],
      if (advanced.isNotEmpty)
        ExpansionTile(
          key: Key('${widget.idPrefix}-advanced'),
          title: const Text('Advanced'),
          children: [for (final key in advanced) _row(key)],
        ),
    ]);
  }
}

/// One provider's settings as a form with a save button: [SchemaFields] over [schema] and [form], the values the host holds, and what
/// [onSave] is given when it is pressed: what was set, the secrets to take out, and where each member marked `x-from-secret` comes
/// from. [onSave] answers why it could not save, or null. Keys are `<keyPrefix>-save`, `<keyPrefix>-save-error` and the fields' (above),
/// so a screen with two forms names them apart.
class SchemaSettingsForm extends StatefulWidget {
  const SchemaSettingsForm({
    super.key,
    required this.schema,
    required this.form,
    required this.name,
    required this.onSave,
    this.values = const {},
    this.secretsSet = const {},
    this.sources = const {},
    this.choices = const [],
    this.saveLabel,
    this.keyPrefix = 'speech',
  });

  final Map<String, Object?> schema, form;

  /// The name the provider is saved under; it starts the name a secret is given by default.
  final String name;

  /// What each setting is now.
  final Map<String, Object?> values;

  /// The secret settings that are set already, which the form never shows.
  final Set<String> secretsSet;

  /// Where each setting marked `x-from-secret` comes from now.
  final Map<String, SecretSource> sources;

  /// The entries that may give such a setting.
  final List<SecretSourceChoice> choices;
  final String? saveLabel;
  final String keyPrefix;
  final Future<String?> Function(Map<String, Object?> settings, List<String> cleared, Map<String, SecretSource> from) onSave;

  @override
  State<SchemaSettingsForm> createState() => _SchemaSettingsFormState();
}

class _SchemaSettingsFormState extends State<SchemaSettingsForm> {
  late final Map<String, Object?> _values = {...widget.values};
  late final Map<String, SecretSource?> _from = {...widget.sources};
  final _cleared = <String>{};
  String? _problem;
  bool _saving = false;

  Future<void> _save() async {
    final invalid = schemaProblem(widget.schema, widget.form, _values);
    if (invalid != null) {
      setState(() => _problem = invalid);
      return;
    }
    final settings = collectSchemaValues(widget.schema, widget.form, _values);
    final properties = schemaProperties(widget.schema);
    final from = <String, SecretSource>{
      for (final key in properties.keys)
        if (_fromSecret(_map(properties[key])) && schemaVisible(widget.schema, widget.form, _values, key))
          if (_from[key] case final source? when source.entry.isNotEmpty && source.name.isNotEmpty) key: source,
    };
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      final problem = await widget.onSave(settings, _cleared.toList(), from);
      if (mounted) {
        setState(() {
          _problem = problem;
          if (problem == null) _cleared.clear();
        });
      }
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final prefix = widget.keyPrefix;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SchemaFields(
        schema: widget.schema,
        form: widget.form,
        values: _values,
        idPrefix: prefix,
        secretsSet: widget.secretsSet,
        cleared: _cleared,
        sources: widget.sources,
        sourceChoices: widget.choices,
        sourceNamePrefix: widget.name,
        onValues: (values) => _values
          ..clear()
          ..addAll(values),
        onClear: (key, {required clear}) => setState(() => clear ? _cleared.add(key) : _cleared.remove(key)),
        onSource: (key, source) => _from[key] = source,
      ),
      if (_problem case final problem?)
        Padding(
          padding: const EdgeInsets.all(8),
          child: Text(problem, key: Key('$prefix-save-error'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
      Align(
        alignment: Alignment.centerRight,
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: FilledButton(key: Key('$prefix-save'), onPressed: _saving ? null : _save, child: Text(widget.saveLabel ?? 'Save')),
        ),
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
