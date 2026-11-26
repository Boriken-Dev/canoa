import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../voice/host_speech.dart';
import 'settings_row.dart';

/// What this build is: the app's version, the publisher and, when the host reports it, each engine it carries with its release and
/// what it was built for.
class AboutSection extends StatefulWidget {
  const AboutSection({super.key, this.version = '', this.host});

  final String version;

  /// The host's speech providers, which say the engine behind each.
  final HostSpeechCatalog? host;

  @override
  State<AboutSection> createState() => _AboutSectionState();
}

class _AboutSectionState extends State<AboutSection> {
  @override
  void initState() {
    super.initState();
    final host = widget.host;
    if (host != null && !host.loaded) unawaited(host.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final host = widget.host;
    return ListenableBuilder(
      listenable: host ?? Listenable.merge(const <Listenable>[]),
      builder: (context, _) => SettingsGroup(children: [
        SettingsRow(
          title: 'Canoa',
          description: 'A client for agents and the tools they use',
          control: Text(widget.version.isEmpty ? 'development build' : widget.version, key: const Key('about-version')),
        ),
        const SettingsRow(title: 'Publisher', description: 'Boriken Dev, boriken.dev'),
        for (final provider in host?.providers ?? const <HostSpeechProvider>[])
          if (provider.version != null)
            SettingsRow(
              key: Key('about-engine-${provider.name}'),
              title: context.t(provider.builtFor == null ? 'about.engine' : 'about.engine_built', {
                'engine': provider.title,
                'version': provider.version,
                'target': provider.builtFor,
              }),
            ),
      ]),
    );
  }
}
