import '../chat/agent_events.dart';
import '../chat/modes.dart';
import '../chat/session_options.dart';
import 'sources.dart';

/// What a source's agent is known to offer before it has run, from the translators Canoa carries: the modes of
/// `coqui-acp-claude` and `coqui-acp-codex` (the presets start them with Bypass allowed), the aliases Claude Code takes for
/// `--model`, and the efforts each takes; Antigravity's and Grok's are what their ACP servers and documentation say. A model Codex offers comes from its app server, so it is known only after start. An
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
    // What `agy acp` advertises in session/new: three modes (its Default is Manual; Bypass is its launch flag, so Canoa's
    // own is offered), five models, and a thinking level of its own id.
    'Antigravity' => [
      const ConfigOption(id: 'mode', label: 'Mode', current: '', category: 'mode',
          choices: [Choice('default', 'Manual'), Choice('accept-edits', 'Accept edits'), Choice('plan', 'Plan')]),
      const ConfigOption(id: 'model', label: 'Model', current: '', category: 'model', choices: [
        Choice('gemini-4-argon', 'Gemini 4 Argon'),
        Choice('gemini-3.8-flash', 'Gemini 3.8 Flash'),
        Choice('claude-opus-5-5', 'Claude Opus 5.5'),
        Choice('claude-sonnet-5-5', 'Claude Sonnet 5.5'),
        Choice('gpt-oss-120b-medium', 'GPT-OSS 120B (Medium)'),
      ]),
      _efforts(const ['low', 'medium', 'high'], id: 'thought_level'),
    ],
    // What `grok agent stdio` advertises in initialize (its models) and its documentation (the options' ids). Its modes are
    // not offered by session/new until signed in, so none is known: Canoa's own Bypass is all before it starts.
    'Grok' => [
      const ConfigOption(id: 'model', label: 'Model', current: '', category: 'model',
          choices: [Choice('grok-4.6', 'Grok 4.6'), Choice('grok-4.5', 'Grok 4.5')]),
      _efforts(const ['low', 'medium', 'high', 'xhigh'], id: 'reasoning_effort'),
    ],
    _ => const [],
  };
}

ConfigOption _efforts(List<String> ids, {String id = 'effort'}) => ConfigOption(
  id: id,
  label: 'Effort',
  current: '',
  category: 'thought_level',
  choices: [for (final each in ids) Choice(each, effortLabels[each] ?? each)],
);

/// [remembered], what the source's agent offered last time, over [known]: a role the agent has offered shows its list, any
/// other the known one.
List<ConfigOption> withKnown(List<ConfigOption> remembered, List<ConfigOption> known) => [
  ...remembered,
  for (final option in known)
    if (!remembered.any((each) => roleOf(each) == roleOf(option))) option,
];
