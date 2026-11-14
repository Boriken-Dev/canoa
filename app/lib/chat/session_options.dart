import 'agent_events.dart';

// Where an agent's choice shows beside Send (its mode, model or effort), and a choice made before the agent can take it, are
// `coqui_acp_chat`'s.
export 'package:coqui_acp_chat/coqui_acp_chat.dart' show OptionRole, PendingChoice, optionIn, roleOf;

/// [option] as Canoa keeps it between runs, which [optionFromJson] reads back.
Map<String, Object?> optionToJson(ConfigOption option) => {
  'id': option.id,
  'name': option.label,
  'category': ?option.category,
  'currentValue': option.current,
  if (option.isMode) 'isMode': true,
  'options': [for (final choice in option.choices) {'value': choice.value, 'name': choice.label}],
};

/// Null for anything [optionToJson] did not write.
ConfigOption? optionFromJson(Object? json) {
  final parsed = parseConfigOptions([json], chatLimits);
  if (parsed == null || parsed.isEmpty) return null;
  final option = parsed.single;
  return json is Map && json['isMode'] == true
      ? ConfigOption(id: option.id, label: option.label, current: option.current, choices: option.choices, isMode: true,
          category: option.category)
      : option;
}
