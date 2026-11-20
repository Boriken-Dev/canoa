import 'package:flutter/material.dart';

import '../forms/schema_fields.dart';
import 'secrets_controller.dart';
import 'secrets_host.dart';
import 'secrets_section.dart' show SetSecretDialog;

/// Sets up a vault (an entry of a secret provider): its settings are drawn from the schema and the form the host serves for it, and a setting that is another
/// secret (the key that unlocks a vault) is a choice of the provider that gives it, never a box to type it in. Then, for each such
/// secret the person gives by typing, the form that asks for it is opened. [providerId] starts the choice on that provider, and
/// [editing] is the entry being changed.
Future<void> showSecretProviderSetup(BuildContext context, SecretsController secrets, {String? providerId, SecretEntry? editing}) async {
  final typed = await showDialog<List<SecretSource>>(
    context: context,
    builder: (_) => SecretProviderSetupDialog(controller: secrets, providerId: providerId, editing: editing),
  );
  if (typed == null || !context.mounted) return;
  for (final source in typed) {
    if (!context.mounted) return;
    await showDialog<void>(
      context: context,
      builder: (_) => SetSecretDialog(controller: secrets, initialEntry: source.entry, initialName: source.name),
    );
  }
}

class SecretProviderSetupDialog extends StatefulWidget {
  const SecretProviderSetupDialog({super.key, required this.controller, this.providerId, this.editing});

  final SecretsController controller;
  final String? providerId;
  final SecretEntry? editing;

  @override
  State<SecretProviderSetupDialog> createState() => _SecretProviderSetupDialogState();
}

class _SecretProviderSetupDialogState extends State<SecretProviderSetupDialog> {
  late String? _provider;
  late final TextEditingController _name;

  List<SecretProviderInfo> get _offered => [for (final each in widget.controller.providers) if (each.hasSettings) each];

  @override
  void initState() {
    super.initState();
    final offered = _offered;
    final wanted = widget.editing?.provider ?? widget.providerId;
    // Without a word on which, the first provider nobody has set up yet.
    final unused = offered.where((each) => !widget.controller.entries.any((entry) => entry.provider == each.id && entry.setUp));
    _provider = offered.any((each) => each.id == wanted) ? wanted : (unused.isNotEmpty ? unused.first.id : offered.firstOrNull?.id);
    _name = TextEditingController(text: widget.editing?.name ?? _provider ?? '');
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  SecretProviderInfo? get _info => _offered.where((each) => each.id == _provider).firstOrNull;

  @override
  Widget build(BuildContext context) {
    final info = _info;
    final editing = widget.editing;
    final entries = widget.controller.entries;
    final name = _name.text.trim();
    return AlertDialog(
      key: const Key('secret-provider-dialog'),
      title: Text(editing == null ? 'Set up a vault' : 'Change ${editing.name}'),
      content: SizedBox(
        width: 520,
        child: info == null
            ? const Text('This host offers no kind of vault that has anything to set up.', key: Key('secret-provider-none'))
            : SingleChildScrollView(
                child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                  if (editing == null)
                    DropdownButtonFormField<String>(
                      key: const Key('secret-provider-choice'),
                      initialValue: _provider,
                      decoration: const InputDecoration(labelText: 'Provider'),
                      items: [for (final each in _offered) DropdownMenuItem(value: each.id, child: Text(each.title))],
                      onChanged: (value) => setState(() {
                        _provider = value;
                        _name.text = value ?? '';
                      }),
                    ),
                  if (info.description case final description?) Padding(padding: const EdgeInsets.only(top: 8), child: Text(description, style: Theme.of(context).textTheme.bodySmall)),
                  TextField(
                    key: const Key('secret-provider-name'),
                    controller: _name,
                    enabled: editing == null,
                    decoration: const InputDecoration(labelText: 'Name of this vault', helperText: 'What canoa.cfg and the Secrets list call it'),
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: 8),
                  SchemaSettingsForm(
                    key: Key('secret-provider-form-${info.id}'),
                    keyPrefix: 'secret-provider',
                    schema: info.schema,
                    form: info.form,
                    name: name,
                    values: editing?.settings ?? const {},
                    sources: editing?.from ?? const {},
                    choices: [
                      for (final entry in entries)
                        if (entry.name != name)
                          SecretSourceChoice(entry.name, entry.needsPerson ? '${entry.name}: you type it when asked' : '${entry.name} (${entry.title})'),
                    ],
                    saveLabel: editing == null ? 'Add' : 'Save',
                    onSave: (settings, cleared, from) async {
                      if (name.isEmpty) return 'Give this setup a name.';
                      final problem = await widget.controller.configure(name: name, provider: info.id, settings: settings, from: from);
                      if (problem != null) return problem;
                      final byTyping = {for (final entry in entries) entry.name: entry.needsPerson};
                      if (context.mounted) {
                        Navigator.pop(context, [for (final source in from.values) if (byTyping[source.entry] ?? false) source]);
                      }
                      return null;
                    },
                  ),
                ]),
              ),
      ),
      actions: [TextButton(key: const Key('secret-provider-cancel'), onPressed: () => Navigator.pop(context), child: const Text('Cancel'))],
    );
  }
}
