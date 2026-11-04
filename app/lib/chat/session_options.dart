import 'agent_events.dart';

/// Where an agent's choice shows beside Send: its session mode, its model, its reasoning effort, or behind "more".
enum OptionRole { mode, model, effort, other }

/// The role of [option]: by its ACP `category` where the agent gives one, else by its id.
OptionRole roleOf(ConfigOption option) {
  if (option.isMode) return OptionRole.mode;
  switch (option.category) {
    case 'mode':
      return OptionRole.mode;
    case 'model':
      return OptionRole.model;
    case 'thought_level':
      return OptionRole.effort;
    case final String _:
      return OptionRole.other;
  }
  return switch (option.id) {
    'mode' => OptionRole.mode,
    'model' => OptionRole.model,
    'effort' || 'reasoning_effort' || 'thought_level' => OptionRole.effort,
    _ => OptionRole.other,
  };
}

/// The first of [options] in [role].
ConfigOption? optionIn(List<ConfigOption> options, OptionRole role) =>
    options.where((option) => roleOf(option) == role).firstOrNull;

/// A choice made before the agent can take it: before it starts, or while a turn runs. It goes to the agent before the next
/// prompt, to the option of the same id, else to one in the same [role], where that option offers [value].
final class PendingChoice {
  const PendingChoice(this.id, this.role, this.value, {this.byPerson = false});
  final String id;
  final OptionRole role;
  final String value;

  /// Whether the person chose it, rather than its being the profile's or their last choice put as a default.
  final bool byPerson;

  Map<String, Object?> toJson() => {'id': id, 'role': role.name, 'value': value, if (byPerson) 'byPerson': true};

  static PendingChoice? fromJson(Object? json) {
    if (json is! Map || json['id'] is! String || json['value'] is! String) return null;
    final role = OptionRole.values.where((role) => role.name == json['role']).firstOrNull;
    return role == null
        ? null
        : PendingChoice(json['id'] as String, role, json['value'] as String, byPerson: json['byPerson'] == true);
  }
}

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
  final parsed = parseConfigOptions([json]);
  if (parsed == null || parsed.isEmpty) return null;
  final option = parsed.single;
  return json is Map && json['isMode'] == true
      ? ConfigOption(id: option.id, label: option.label, current: option.current, choices: option.choices, isMode: true,
          category: option.category)
      : option;
}
