import 'package:flutter/material.dart';

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
