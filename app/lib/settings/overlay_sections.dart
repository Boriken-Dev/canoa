import 'package:material_ui/material_ui.dart';

import 'settings_row.dart';

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
