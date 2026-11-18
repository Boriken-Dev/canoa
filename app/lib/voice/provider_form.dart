import 'package:flutter/material.dart';

import '../forms/schema_fields.dart';
import 'host_speech.dart';
import 'speech_backend.dart';

/// Draws the settings of one speech provider from the document the host serves for its kind (a schema and a form, drawn by
/// [SchemaFields]). Saving writes the host's canoa.cfg through `speech/configure`.
class ProviderSettingsForm extends StatefulWidget {
  const ProviderSettingsForm({super.key, required this.catalog, required this.kind, required this.name, this.provider, this.saveLabel, this.onSaved});

  final HostSpeechCatalog catalog;
  final SpeechKind kind;

  /// The name the provider is saved under.
  final String name;

  /// The provider as the host lists it, for the values to start from; null for a new one.
  final HostSpeechProvider? provider;

  /// What the save button says, in place of Add for a new provider and Save for a listed one.
  final String? saveLabel;

  /// Called once the host has saved it.
  final VoidCallback? onSaved;

  @override
  State<ProviderSettingsForm> createState() => _ProviderSettingsFormState();
}

class _ProviderSettingsFormState extends State<ProviderSettingsForm> {
  final _values = <String, Object?>{};
  final _cleared = <String>{};
  String? _problem;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _values.addAll(widget.provider?.settings ?? const {});
  }

  Future<void> _save() async {
    final settings = collectSchemaValues(widget.kind.schema, widget.kind.form, _values);
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      await widget.catalog.save(widget.name, widget.kind.id, settings, clear: _cleared.toList());
      if (mounted) setState(_cleared.clear);
      widget.onSaved?.call();
    } on SpeechFailure catch (failure) {
      if (mounted) setState(() => _problem = failure.message);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SchemaFields(
        schema: widget.kind.schema,
        form: widget.kind.form,
        values: _values,
        idPrefix: 'speech',
        secretsSet: widget.provider?.secretsSet.toSet() ?? const {},
        cleared: _cleared,
        onValues: (values) => _values
          ..clear()
          ..addAll(values),
        onClear: (key, {required clear}) => setState(() => clear ? _cleared.add(key) : _cleared.remove(key)),
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
            child: Text(widget.saveLabel ?? (widget.provider == null ? 'Add' : 'Save')),
          ),
        ),
      ),
    ]);
  }
}
