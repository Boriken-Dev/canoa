import '../chat/agent_events.dart';
import '../chat/modes.dart';
import '../chat/session_options.dart';
import 'source_icons.dart';
import 'sources.dart';

/// The modes a source's agent is known to offer before it has run: those of `coqui-acp-claude` and `coqui-acp-codex` (the
/// presets start them with Bypass allowed) and Antigravity's three, which `agy acp` advertises in session/new. Models and
/// efforts are not here: the host asks the source's provider for them (see `SourceChoices`). An agent the app knows nothing of
/// has nothing here, and the composer says its choices come after it starts.
///
/// What the agent then offers wins: a choice made here that it does not offer is reported when it starts, not applied.
///
/// A source is known by its provider ([sourceProvider]), so an entry a served page reaches by address, which names its `harness`,
/// offers them as the host's own source does. Bypass is listed only where the app runs the agent allowing it (a local source): a
/// host's entry offers it only when its canoa.cfg says `bypass = true`, which a page cannot read, so the chat's own Bypass stands
/// for it until the agent says.
List<ConfigOption> knownModes(AgentSource source) {
  final provider = sourceProvider(source);
  return switch (provider?.toLowerCase()) {
    'claude' || 'codex' => [
      ConfigOption(
        id: 'mode',
        label: 'Mode',
        current: '',
        category: 'mode',
        choices: [for (final id in modeOrder) if (source.local || id != 'bypass') Choice(id, modeLabels[id]!)],
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
