import 'package:material_ui/material_ui.dart';

import '../forms/schema_fields.dart';
import 'host_speech.dart';
import 'speech_backend.dart';

/// Draws the settings of one speech provider from the document the host serves for its kind (a schema and a form, drawn by
/// [SchemaSettingsForm]). Saving writes the host's canoa.cfg through `speech/configure`.
class ProviderSettingsForm extends StatelessWidget {
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
  Widget build(BuildContext context) => SchemaSettingsForm(
    schema: kind.schema,
    form: kind.form,
    name: name,
    values: provider?.settings ?? const {},
    secretsSet: provider?.secretsSet.toSet() ?? const {},
    saveLabel: saveLabel ?? (provider == null ? 'Add' : 'Save'),
    onSave: (settings, cleared, from) async {
      try {
        await catalog.save(name, kind.id, settings, clear: cleared);
        onSaved?.call();
        return null;
      } on SpeechFailure catch (failure) {
        return failure.message;
      }
    },
  );
}
