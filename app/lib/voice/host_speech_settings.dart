import 'package:material_ui/material_ui.dart';

import '../settings/settings_row.dart';
import '../store/store_catalogs.dart' show confirmRemoval;
import 'host_speech.dart';
import 'provider_form.dart';

/// Settings > Voice, the host's part: the speech providers its canoa.cfg defines, each with its settings drawn from the form its
/// kind declares, and a way to add one. A provider is a program the host runs or an address it sends audio to, which the person
/// names: the host runs and calls what they say.
class HostProvidersGroup extends StatefulWidget {
  const HostProvidersGroup({super.key, required this.catalog});

  final HostSpeechCatalog catalog;

  @override
  State<HostProvidersGroup> createState() => _HostProvidersGroupState();
}

class _HostProvidersGroupState extends State<HostProvidersGroup> {
  String? _editing;
  SpeechKind? _newKind;
  final _newName = TextEditingController();
  String? _problem;

  @override
  void initState() {
    super.initState();
    widget.catalog.refresh();
  }

  @override
  void dispose() {
    _newName.dispose();
    super.dispose();
  }

  Future<void> _remove(String name) async {
    if (!await confirmRemoval(
      context,
      title: 'Remove $name?',
      body: 'This speech provider, with its address and settings, is taken away from this host. Any program or account it used stays as it is.',
    )) {
      return;
    }
    if (!mounted) return;
    try {
      await widget.catalog.remove(name);
      if (mounted) setState(() => _editing = _editing == name ? null : _editing);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = error.toString());
    }
  }

  bool get _nameOk => RegExp(r'^[A-Za-z0-9_-]{1,40}$').hasMatch(_newName.text) &&
      !widget.catalog.providers.any((p) => p.name == _newName.text);

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.catalog,
    builder: (context, _) {
      final catalog = widget.catalog;
      final scheme = Theme.of(context).colorScheme;
      if (catalog.problem != null && catalog.providers.isEmpty) {
        return SettingsGroup(title: 'Speech providers on the host', children: [
          SettingsRow(
            key: const Key('speech-host-problem'),
            title: 'This host\'s speech providers are not available',
            description: catalog.problem,
            child: true,
          ),
        ]);
      }
      return SettingsGroup(title: 'Speech providers on the host', children: [
        for (final provider in catalog.providers) ...[
          SettingsRow(
            key: Key('speech-provider-${provider.name}'),
            title: provider.name,
            description: provider.problem ?? (provider.description.isEmpty ? provider.title : provider.description),
            control: Row(mainAxisSize: MainAxisSize.min, children: [
              TextButton(
                key: Key('speech-edit-${provider.name}'),
                onPressed: () => setState(() => _editing = _editing == provider.name ? null : provider.name),
                child: Text(_editing == provider.name ? 'Close' : 'Settings'),
              ),
              IconButton(
                key: Key('speech-remove-${provider.name}'),
                tooltip: 'Remove ${provider.name}',
                icon: const Icon(Icons.delete_outline),
                onPressed: () => _remove(provider.name),
              ),
            ]),
          ),
          if (_editing == provider.name)
            if (catalog.types.where((t) => t.id == provider.kind).firstOrNull case final kind?)
              ProviderSettingsForm(key: Key('speech-form-${provider.name}'), catalog: catalog, kind: kind, name: provider.name, provider: provider),
        ],
        SettingsRow(
          key: const Key('speech-add'),
          title: 'Add a provider',
          description: 'A program the host runs, or a speech-to-text address you name. The audio goes where you say',
          below: Row(children: [
            DropdownButton<SpeechKind>(
              key: const Key('speech-new-kind'),
              hint: const Text('Kind'),
              value: _newKind,
              items: [for (final kind in catalog.types) DropdownMenuItem(value: kind, child: Text(kind.title))],
              onChanged: (kind) => setState(() => _newKind = kind),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: TextField(
                key: const Key('speech-new-name'),
                controller: _newName,
                decoration: const InputDecoration(isDense: true, hintText: 'Name, such as dictation'),
                onChanged: (_) => setState(() {}),
              ),
            ),
          ]),
        ),
        if (_newKind case final kind? when _nameOk)
          ProviderSettingsForm(key: Key('speech-new-${kind.id}-${_newName.text}'), catalog: catalog, kind: kind, name: _newName.text),
        if (_problem case final problem?)
          Padding(padding: const EdgeInsets.all(8), child: Text(problem, style: TextStyle(color: scheme.error))),
      ]);
    },
  );
}
