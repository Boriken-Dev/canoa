import '../chat/agent_events.dart';
import '../chat/modes.dart';
import '../chat/session_options.dart';
import 'sources.dart';

/// The modes a source's agent is known to offer before it has run: those of `coqui-acp-claude` and `coqui-acp-codex` (the
/// presets start them with Bypass allowed) and Antigravity's three, which `agy acp` advertises in session/new. Models and
/// efforts are not here: the host asks the source's provider for them (see `SourceChoices`). An agent the app knows nothing of
/// has nothing here, and the composer says its choices come after it starts.
///
/// What the agent then offers wins: a choice made here that it does not offer is reported when it starts, not applied.
List<ConfigOption> knownModes(AgentSource source) {
  if (!source.local) return const [];
  return switch (source.label) {
    'Claude' || 'Codex' => [
      ConfigOption(
        id: 'mode',
        label: 'Mode',
        current: '',
        category: 'mode',
        choices: [for (final id in modeOrder) Choice(id, modeLabels[id]!)],
      ),
    ],
    'Antigravity' => [
      const ConfigOption(id: 'mode', label: 'Mode', current: '', category: 'mode',
          choices: [Choice('default', 'Manual'), Choice('accept-edits', 'Accept edits'), Choice('plan', 'Plan')]),
    ],
    _ => const [],
  };
}

/// [first] over [then]: a role [first] has shows its list, any other the one of [then].
List<ConfigOption> overlaid(List<ConfigOption> first, List<ConfigOption> then) => [
  ...first,
  for (final option in then)
    if (!first.any((each) => roleOf(each) == roleOf(option))) option,
];
