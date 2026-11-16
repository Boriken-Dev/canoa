import 'package:flutter/material.dart';

import '../settings/settings_row.dart';
import 'host_speech.dart';
import 'speech_backend.dart';

/// Draws the settings of one speech provider from the document the host serves for its kind: a JSON Schema whose members name
/// their `x-section`, `x-order` and `x-advanced`, and the guatiao-intake form that gives the sections their titles and the fields
/// their widget, placeholder, unit and `visibleWhen`. Saving writes the host's canoa.cfg through `speech/configure`.
class ProviderSettingsForm extends StatefulWidget {
  const ProviderSettingsForm({super.key, required this.catalog, required this.kind, required this.name, this.provider});

  final HostSpeechCatalog catalog;
  final SpeechKind kind;

  /// The name the provider is saved under.
  final String name;

  /// The provider as the host lists it, for the values to start from; null for a new one.
  final HostSpeechProvider? provider;

  @override
  State<ProviderSettingsForm> createState() => _ProviderSettingsFormState();
}

class _ProviderSettingsFormState extends State<ProviderSettingsForm> {
  final _values = <String, Object?>{};
  final _typed = <String, TextEditingController>{};
  final _cleared = <String>{};
  String? _problem;
  bool _saving = false;

  Map<String, Object?> get _properties {
    final properties = widget.kind.schema['properties'];
    return properties is Map ? properties.cast<String, Object?>() : const {};
  }

  Map<String, Object?> _field(String key) {
    final field = _properties[key];
    return field is Map ? field.cast<String, Object?>() : const {};
  }

  Map<String, Object?> _hints(String key) {
    final fields = widget.kind.form['fields'];
    final hints = fields is Map ? fields[key] : null;
    return hints is Map ? hints.cast<String, Object?>() : const {};
  }

  bool _secret(String key) => _field(key)['writeOnly'] == true || _field(key)['x-sensitive'] == true;

  @override
  void initState() {
    super.initState();
    _values.addAll(widget.provider?.settings ?? const {});
  }

  @override
  void dispose() {
    for (final controller in _typed.values) {
      controller.dispose();
    }
    super.dispose();
  }

  /// The type word of a schema member, `null` allowed beside it.
  String _type(Map<String, Object?> field) {
    final type = field['type'];
    if (type is String) return type;
    if (type is List) return type.firstWhere((t) => t != 'null', orElse: () => 'string').toString();
    return 'string';
  }

  bool _visible(String key) {
    final when = _hints(key)['visibleWhen'];
    if (when is! Map) return true;
    final other = _values[when['field']];
    if (when.containsKey('equals')) return other == when['equals'];
    final within = when['in'];
    return within is List ? within.contains(other) : true;
  }

  TextEditingController _controller(String key, String initial) =>
      _typed.putIfAbsent(key, () => TextEditingController(text: initial));

  Widget _control(String key) {
    final field = _field(key);
    final hints = _hints(key);
    final type = _type(field);
    final unit = hints['unit'] is String ? hints['unit']! as String : null;
    final placeholder = hints['placeholder'] is String ? hints['placeholder']! as String : null;
    final secret = _secret(key);
    if (type == 'boolean') {
      return Switch(
        key: Key('speech-field-$key'),
        value: _values[key] == true || (!_values.containsKey(key) && field['default'] == true),
        onChanged: (on) => setState(() => _values[key] = on),
      );
    }
    if (field['enum'] is List) {
      final choices = (field['enum']! as List).whereType<String>().toList();
      return DropdownButton<String>(
        key: Key('speech-field-$key'),
        value: choices.contains(_values[key]) ? _values[key]! as String : (field['default'] as String?),
        items: [for (final choice in choices) DropdownMenuItem(value: choice, child: Text(choice))],
        onChanged: (choice) => setState(() => _values[key] = choice),
      );
    }
    final isList = type == 'array';
    final isNumber = type == 'integer' || type == 'number';
    final current = _values[key];
    final initial = isList
        ? (current is List ? current.join('\n') : '')
        : (current?.toString() ?? (isNumber && field['default'] != null ? field['default'].toString() : ''));
    final set = secret && (widget.provider?.secretsSet.contains(key) ?? false) && !_cleared.contains(key);
    return SizedBox(
      width: 280,
      child: TextField(
        key: Key('speech-field-$key'),
        controller: _controller(key, secret ? '' : initial),
        obscureText: secret || hints['widget'] == 'password',
        minLines: 1,
        maxLines: isList ? 4 : 1,
        keyboardType: isNumber ? TextInputType.number : TextInputType.text,
        decoration: InputDecoration(
          isDense: true,
          hintText: set ? 'Set. Type to replace' : placeholder,
          suffixText: unit,
          suffixIcon: set
              ? IconButton(
                  key: Key('speech-clear-$key'),
                  tooltip: 'Remove it',
                  icon: const Icon(Icons.clear, size: 18),
                  onPressed: () => setState(() => _cleared.add(key)),
                )
              : null,
        ),
        onChanged: (text) {
          if (secret) {
            _cleared.remove(key);
            _values[key] = text;
          } else if (isList) {
            _values[key] = [for (final line in text.split('\n')) if (line.trim().isNotEmpty) line.trim()];
          } else if (isNumber) {
            _values[key] = int.tryParse(text.trim());
          } else {
            _values[key] = text.isEmpty ? null : text;
          }
        },
      ),
    );
  }

  List<String> _keys({required bool advanced}) {
    final sections = <String>[];
    final listed = widget.kind.form['sections'];
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

  Future<void> _save() async {
    final settings = <String, Object?>{};
    for (final key in _properties.keys) {
      if (!_visible(key)) continue;
      final value = _values[key];
      if (value == null || (value is String && value.isEmpty && !_secret(key))) continue;
      settings[key] = value;
    }
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      await widget.catalog.save(widget.name, widget.kind.id, settings, clear: _cleared.toList());
      if (mounted) setState(_cleared.clear);
    } on SpeechFailure catch (failure) {
      if (mounted) setState(() => _problem = failure.message);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final advanced = _keys(advanced: true);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final key in _keys(advanced: false)) _row(key),
      if (advanced.isNotEmpty)
        ExpansionTile(
          key: const Key('speech-advanced'),
          title: const Text('Advanced'),
          children: [for (final key in advanced) _row(key)],
        ),
      if (_problem case final problem?)
        Padding(
          padding: const EdgeInsets.all(8),
          child: Text(problem, key: const Key('speech-save-error'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
      Align(
        alignment: Alignment.centerRight,
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: FilledButton(
            key: const Key('speech-save'),
            onPressed: _saving ? null : _save,
            child: Text(widget.provider == null ? 'Add' : 'Save'),
          ),
        ),
      ),
    ]);
  }
}
