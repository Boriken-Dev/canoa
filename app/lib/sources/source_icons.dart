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

/// The icon of [source]: its provider's for a hosted entry, and the terminal for a command a person wrote. A source of the
/// host's own that runs no harness has the provider its name says (Grok, Pi, Antigravity).
IconData sourceIcon(AgentSource? source) =>
    source == null ? Icons.terminal : providerIcon(source.harness ?? (source.local ? source.label.toLowerCase() : null));
