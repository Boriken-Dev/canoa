/// The values for the options an overlay declares: one typed field per non-sensitive option, written to
/// `[overlays.NAME] options`. A field left empty is left out, so the overlay's default applies.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../store/overlay_page.dart' show oneLine;

/// One option an overlay declares: its name, `string`, `integer` or `boolean`, and what its manifest says of it.
class DeclaredOption {
  const DeclaredOption({required this.name, required this.type, this.description, this.fallback});

  final String name;
  final String type;
  final String? description;

  /// The default the overlay gives: a `String`, a `BigInt` or a `bool`, or null for none.
  final Object? fallback;

  String? get fallbackText => fallback == null ? null : '$fallback';
}

/// The options [manifest] declares that take a value from the person: a sensitive one is never put in context, so it gets
/// no field. A declaration of an unknown type is skipped.
List<DeclaredOption> declaredOptions(Map<String, Object?> manifest) {
  final raw = manifest['options'];
  if (raw is! Map) return const [];
  return [
    for (final MapEntry(key: name, value: declared) in raw.entries)
      if (declared is Map && declared['sensitive'] != true && const {'string', 'integer', 'boolean'}.contains(declared['type']))
        DeclaredOption(
          name: '$name',
          type: declared['type'] as String,
          description: declared['description'] is String ? declared['description'] as String : null,
          fallback: declared['default'],
        ),
  ];
}

/// The form: [saved] are the values canoa.cfg holds now; [onSave] gets the whole map to write, each declared option
/// present only when the person gave it a value, and values for names the form does not show kept as they were.
class OverlayOptionsForm extends StatefulWidget {
  const OverlayOptionsForm({super.key, required this.overlay, required this.options, required this.saved,
      required this.onSave, this.busy = false});

  final String overlay;
  final List<DeclaredOption> options;
  final Map<String, Object?> saved;
  final Future<void> Function(Map<String, Object?> values) onSave;
  final bool busy;

  @override
  State<OverlayOptionsForm> createState() => _OverlayOptionsFormState();
}

class _OverlayOptionsFormState extends State<OverlayOptionsForm> {
  final _text = <String, TextEditingController>{};
  final _flags = <String, bool?>{};

  @override
  void initState() {
    super.initState();
    for (final option in widget.options) {
      final value = widget.saved[option.name];
      if (option.type == 'boolean') {
        _flags[option.name] = value is bool ? value : null;
      } else {
        _text[option.name] = TextEditingController(text: value == null ? '' : '$value')..addListener(_changed);
      }
    }
  }

  @override
  void dispose() {
    for (final controller in _text.values) {
      controller.dispose();
    }
    super.dispose();
  }

  void _changed() => setState(() {});

  /// The text's error, or null when it holds a value of the type or nothing.
  String? _error(DeclaredOption option) {
    final text = _text[option.name]?.text;
    if (text == null || text.isEmpty || option.type != 'integer') return null;
    return int.tryParse(text) == null ? 'A whole number' : null;
  }

  /// What the form would write, or null while a field holds a wrong-typed value.
  Map<String, Object?>? _values() {
    final values = <String, Object?>{...widget.saved};
    for (final option in widget.options) {
      values.remove(option.name);
      switch (option.type) {
        case 'boolean':
          if (_flags[option.name] case final flag?) values[option.name] = flag;
        case 'integer':
          final text = _text[option.name]!.text;
          if (text.isEmpty) break;
          final number = int.tryParse(text);
          if (number == null) return null;
          values[option.name] = number;
        default:
          final text = _text[option.name]!.text;
          if (text.isNotEmpty) values[option.name] = text;
      }
    }
    return values;
  }

  bool _differs(Map<String, Object?> values) {
    if (values.length != widget.saved.length) return true;
    for (final MapEntry(:key, :value) in values.entries) {
      if (!widget.saved.containsKey(key) || widget.saved[key] != value) return true;
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final values = _values();
    final canSave = values != null && _differs(values) && !widget.busy;
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      for (final option in widget.options) ...[
        _field(context, option),
        const SizedBox(height: 12),
      ],
      Row(children: [
        FilledButton(
          key: Key('overlay-options-save-${widget.overlay}'),
          onPressed: canSave ? () => unawaited(widget.onSave(values)) : null,
          child: const Text('Save options'),
        ),
        const SizedBox(width: 12),
        Expanded(child: Text('Takes effect at the next session.', style: text.bodySmall)),
      ]),
    ]);
  }

  Widget _field(BuildContext context, DeclaredOption option) {
    final text = Theme.of(context).textTheme;
    final key = 'overlay-option-${widget.overlay}-${option.name}';
    final helper = oneLine([
      if (option.description != null) option.description!,
      if (option.fallbackText != null) 'Default: ${option.fallbackText}' else 'No default',
    ].join(' · '));
    if (option.type == 'boolean') {
      final flag = _flags[option.name];
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(option.name, style: text.labelLarge),
        const SizedBox(height: 4),
        SegmentedButton<bool?>(
          key: Key(key),
          showSelectedIcon: false,
          emptySelectionAllowed: false,
          segments: const [
            ButtonSegment(value: null, label: Text('Default')),
            ButtonSegment(value: true, label: Text('On')),
            ButtonSegment(value: false, label: Text('Off')),
          ],
          selected: {flag},
          onSelectionChanged: (chosen) => setState(() => _flags[option.name] = chosen.first),
        ),
        const SizedBox(height: 4),
        Text(helper, style: text.bodySmall),
      ]);
    }
    final integer = option.type == 'integer';
    return TextField(
      key: Key(key),
      controller: _text[option.name],
      keyboardType: integer ? TextInputType.number : TextInputType.text,
      inputFormatters: [if (integer) FilteringTextInputFormatter.allow(RegExp(r'[0-9-]'))],
      decoration: InputDecoration(
        labelText: option.name,
        hintText: option.fallbackText,
        helperText: helper,
        helperMaxLines: 4,
        errorText: _error(option),
        border: const OutlineInputBorder(),
        isDense: true,
      ),
    );
  }
}
