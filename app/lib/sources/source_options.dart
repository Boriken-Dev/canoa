import '../chat/agent_events.dart';
import '../chat/modes.dart';
import '../chat/session_options.dart';
import 'source_icons.dart';
import 'sources.dart';

/// The modes a source's agent is known to offer before it has run: those of `coqui-acp-claude`, `coqui-acp-codex` and
/// `coqui-acp-pi`, every mode the agent has, Bypass included, and Antigravity's three, which `agy acp` advertises in session/new. Models and
/// efforts are not here: the host asks the source's provider for them (see `SourceChoices`). An agent the app knows nothing of
/// has nothing here, and the composer says its choices come after it starts.
///
/// What the agent then offers wins: a choice made here that it does not offer is reported when it starts, not applied.
///
/// A source is known by its provider ([sourceProvider]), so an entry a served page reaches by address, which names its `harness`,
/// offers them as the host's own source does: the same for every agent, with no key in canoa.cfg to gate Bypass.
List<ConfigOption> knownModes(AgentSource source) {
  final provider = sourceProvider(source);
  return switch (provider?.toLowerCase()) {
    'claude' || 'codex' || 'pi' => [
      ConfigOption(
        id: 'mode',
        label: 'Mode',
        current: '',
        category: 'mode',
        // Pi has nothing to back an automatic reviewer, so Auto is not one of its modes.
        choices: [for (final id in modeOrder) if (provider?.toLowerCase() != 'pi' || id != 'auto') Choice(id, modeLabels[id]!)],
      ),
    ],
    _ when !source.local => const [],
    'antigravity' => [
      ConfigOption(id: 'mode', label: 'Mode', current: '', category: 'mode',
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
