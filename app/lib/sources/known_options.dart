import '../chat/agent_events.dart';
import '../chat/modes.dart';
import '../chat/session_options.dart';
import 'sources.dart';

/// What a source's agent is known to offer before it has run, from the translators Canoa carries: the modes of
/// `coqui-acp-claude` and `coqui-acp-codex` (the presets start them with Bypass allowed), the aliases Claude Code takes for
/// `--model`, and the efforts each takes. A model Codex offers comes from its app server, so it is known only after start. An
/// agent the app knows nothing of has nothing here, and the composer says its choices come after it starts.
///
/// What the agent then offers wins: a choice made here that it does not offer is reported when it starts, not applied.
List<ConfigOption> knownOptions(AgentSource source) {
  if (source.kind != SourceKind.preset) return const [];
  final mode = ConfigOption(
    id: 'mode',
    label: 'Mode',
    current: '',
    category: 'mode',
    choices: [for (final id in modeOrder) Choice(id, modeLabels[id]!)],
  );
  return switch (source.label) {
    'Claude' => [
      mode,
      const ConfigOption(id: 'model', label: 'Model', current: '', category: 'model',
          choices: [Choice('opus', 'Opus'), Choice('sonnet', 'Sonnet'), Choice('haiku', 'Haiku')]),
      _efforts(const ['low', 'medium', 'high', 'xhigh', 'max']),
    ],
    'Codex' => [mode, _efforts(const ['low', 'medium', 'high'])],
    _ => const [],
  };
}

ConfigOption _efforts(List<String> ids) => ConfigOption(
  id: 'effort',
  label: 'Effort',
  current: '',
  category: 'thought_level',
  choices: [for (final id in ids) Choice(id, effortLabels[id] ?? id)],
);

/// [remembered], what the source's agent offered last time, over [known]: a role the agent has offered shows its list, any
/// other the known one.
List<ConfigOption> withKnown(List<ConfigOption> remembered, List<ConfigOption> known) => [
  ...remembered,
  for (final option in known)
    if (!remembered.any((each) => roleOf(each) == roleOf(option))) option,
];
