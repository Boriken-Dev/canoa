import 'package:flutter/material.dart';

import 'sources.dart';

/// The icon for a hosted entry's provider id: one for each provider the app knows, a generic one for any other id and for
/// an entry with no provider (a program on standard streams).
IconData providerIcon(String? harness) => switch (harness) {
  'claude' => Icons.auto_awesome_outlined,
  'codex' => Icons.code,
  'grok' => Icons.bolt_outlined,
  'antigravity' => Icons.rocket_launch_outlined,
  'pi' => Icons.functions,
  _ => Icons.dns_outlined,
};

/// The icon of [source]: its provider's for a hosted entry and for a preset, so a source looks the same hosted or started
/// here, and the terminal for a command a person wrote.
IconData sourceIcon(AgentSource? source) => switch (source?.kind) {
  SourceKind.hosted => providerIcon(source?.harness),
  SourceKind.preset => providerIcon(_presetProvider(source!)),
  _ => Icons.terminal,
};

/// The provider id a preset stands for: the harness `canoa agent` serves, else the preset's name in lower case.
String _presetProvider(AgentSource source) {
  final command = source.command!;
  return command.hosts && command.args.length > 1 ? command.args[1] : source.label.toLowerCase();
}
