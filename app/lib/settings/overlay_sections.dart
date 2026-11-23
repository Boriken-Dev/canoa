import 'package:material_ui/material_ui.dart';

import 'settings_row.dart';

/// Where the Store's own settings are: its catalogs are each type's, set on the type's own page, so Settings only says so and opens
/// the Store.
class StoreSection extends StatelessWidget {
  const StoreSection({super.key, this.onStore});

  /// Opens the Store; null where the app has none.
  final VoidCallback? onStore;

  @override
  Widget build(BuildContext context) => SettingsGroup(children: [
    SettingsRow(
      title: 'Catalogs',
      description:
          'Each Store type reads its own catalogs: Overlays, Skills, MCP servers, Extensions and any type an extension adds. Open a type in the '
          'Store and its settings (the button at the top) list its catalogs, add one in that type\'s words and set what is read from them.',
      control: FilledButton(key: const Key('settings-open-store'), onPressed: onStore, child: const Text('Open the Store')),
    ),
  ]);
}

/// What this build is.
class AboutSection extends StatelessWidget {
  const AboutSection({super.key, this.version = ''});

  final String version;

  @override
  Widget build(BuildContext context) => SettingsGroup(children: [
    SettingsRow(
      title: 'Canoa',
      description: 'A client for agents and the tools they use',
      control: Text(version.isEmpty ? 'development build' : version, key: const Key('about-version')),
    ),
    const SettingsRow(title: 'Publisher', description: 'Boriken Dev, boriken.dev'),
  ]);
}
