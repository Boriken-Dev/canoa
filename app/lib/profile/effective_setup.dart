/// The effective setup of a new session: every value it starts with beside the layer that gives it (built-in, overlays, yours, the
/// project's once trusted, the agent's entry, then the session's own, which is the profile chosen and what was changed from it),
/// and the choices the agent cannot take, each with why. It reads the layers as Settings, Profiles does and shows them the same way.
library;

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'profile_layers.dart';
import 'profiles_panel.dart' show EffectiveTable, visible;

/// What a new session starts with for [agent] working in [workingDirectory], with [session] as the session's own layer.
class EffectiveSetup extends StatefulWidget {
  const EffectiveSetup({
    super.key,
    required this.config,
    required this.agent,
    required this.session,
    this.home,
    this.workingDirectory,
    this.profile,
    this.offered = const {},
    this.unsupported = const {},
  });

  final CanoaConfig config;

  /// The source's label, whose entry's layer applies.
  final String agent;

  /// The session's own layer: the profile's values with what the person changed, and its starting instructions.
  final Profile session;
  final ProfileHome? home;
  final String? workingDirectory;

  /// The profile the session was given, by name.
  final String? profile;

  /// What the agent offers to choose among, by `model`, `effort` and `mode`; a key with no values says nothing is known of it.
  final Map<String, List<String>> offered;

  /// Values the profile asks for that the agent cannot take, by key, with why.
  final Map<String, String> unsupported;

  @override
  State<EffectiveSetup> createState() => _EffectiveSetupState();
}

class _EffectiveSetupState extends State<EffectiveSetup> {
  ProfileLayers? _layers;
  String? _problem;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final config = widget.config;
    final ProfileLayers found;
    try {
      found = await profileLayers(
        overlays: config.overlays,
        user: config.profileWithChosenSkills,
        agents: config.agentProfiles,
        agent: widget.agent,
        home: widget.home,
        workingDirectory: widget.workingDirectory,
        session: widget.session,
        iconLimits: config.store.iconLimits,
        skillPins: config.skillPins,
      );
    } on Object catch (error) {
      // The layers are the host's to read; one that cannot be asked leaves the setup unshown, and says why.
      if (mounted) setState(() => _problem = 'The host could not be asked for the layers: $error');
      return;
    }
    if (mounted) setState(() => _layers = found);
  }

  /// The choices the agent cannot take: what the profile asked and the agent lacks, then each effective value the agent lists
  /// choices for and does not list.
  List<String> _refused(EffectiveProfile effective) {
    final out = [for (final MapEntry(:key, :value) in widget.unsupported.entries) 'The profile\'s $key: $value'];
    for (final (key, value) in [('model', effective.model), ('effort', effective.effort), ('mode', effective.mode)]) {
      final list = widget.offered[key];
      if (value == null || list == null || list.isEmpty || list.contains(value.value)) continue;
      final line = 'The $key ${value.value}, from ${value.layer}: the agent does not offer it';
      if (!out.any((held) => held.contains(key) && held.contains(value.value))) out.add(line);
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final found = _layers;
    final refused = found == null ? const <String>[] : _refused(found.effective);
    final theme = Theme.of(context);
    return Column(
      key: const Key('effective-setup'),
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          '${widget.agent}${widget.profile == null ? '' : ' with the profile ${widget.profile}'}: every value beside the layer '
          'that gives it. A later layer wins.',
        ),
        const SizedBox(height: 8),
        if (_problem case final problem?)
          Text(
            visible(problem),
            key: const Key('effective-problem'),
            style: TextStyle(color: theme.colorScheme.error),
          )
        else if (found == null)
          const LinearProgressIndicator()
        else
          EffectiveTable(effective: found.effective),
        for (final note in found?.notes ?? const <LayerNote>[])
          Text(visible('${note.layer}: ${note.text}'), style: theme.textTheme.bodySmall),
        if (refused.isNotEmpty) ...[
          const SizedBox(height: 8),
          Text('Not taken', style: theme.textTheme.titleSmall),
          for (final line in refused)
            Text(
              visible(line),
              key: const Key('effective-refused'),
              style: TextStyle(color: theme.colorScheme.error),
            ),
        ],
      ],
    );
  }
}

/// [EffectiveSetup] in a dialog.
class EffectiveSetupDialog extends StatelessWidget {
  const EffectiveSetupDialog({super.key, required this.setup});

  final EffectiveSetup setup;

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Effective setup'),
    content: SizedBox(width: 640, child: SingleChildScrollView(child: setup)),
    actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
  );
}
