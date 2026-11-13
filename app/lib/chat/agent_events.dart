import 'dart:convert';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatLimits, shortText;

// What an agent reports beyond its messages that the app's own session state reads: its choices. The tool calls, terminals, plan,
// compactions, commands and usage it draws are `coqui_acp_chat`'s, re-exported here for the files that read them.
export 'package:coqui_acp_chat/coqui_acp_chat.dart'
    show
        AgentCommandInfo,
        ChatLimits,
        Compaction,
        CompactionStatus,
        PlanEntry,
        PlanStatus,
        Rendering,
        MessageRole,
        UpdateRecorder,
        attachedNames,
        renderingOf,
        SessionUsage,
        TerminalView,
        ToolBlock,
        ToolCall,
        ToolDiff,
        ToolPatch,
        ToolStatus,
        ToolTerminal,
        ToolText,
        UsageBucket,
        UsageCoverage,
        UsageLimitHit,
        UsageModelSpend,
        UsageRefill,
        UsageReport,
        UsageSpend,
        UsageSupport,
        UsageUnit,
        describeToolDiff,
        parseCommands,
        parsePlan,
        planItems,
        usageCapability,
        usageMethod,
        usageNotification;

/// The bounds on what an agent sends, at their defaults.
const ChatLimits chatLimits = ChatLimits();

const int maxConfigOptions = 32;
const int maxChoices = 200;

String boundedHead(String text, int limit) => text.length > limit ? '${text.substring(0, limit)}\n…' : text;

final class Choice {
  const Choice(this.value, this.label);
  final String value;
  final String label;
}

/// Something the agent lets a person choose, such as its model.
final class ConfigOption {
  const ConfigOption({required this.id, required this.label, required this.current, required this.choices, this.isMode = false,
      this.category, this.defaultLabel, this.defaultName});
  final String id;
  final String label;
  final String current;
  final List<Choice> choices;

  /// What ACP says it is, such as `model`, `thought_level` or `mode`, when the agent says.
  final String? category;

  /// What taking no choice stands for, such as `Default (Opus 5.5)`, when the provider says; a draft shows it for its Default.
  final String? defaultLabel;

  /// What taking no choice stands for, short, as the composer's pill reads it (`Opus 5.5`), when the provider says.
  final String? defaultName;

  /// Set with `session/set_mode` rather than `session/set_config_option`.
  final bool isMode;
}

Iterable<Object?> _flatChoices(List<dynamic> raw) sync* {
  for (final entry in raw) {
    // A group holds its own list of choices.
    if (entry is Map && entry['options'] is List) {
      yield* (entry['options'] as List);
    } else {
      yield entry;
    }
  }
}

List<ConfigOption>? parseConfigOptions(Object? raw) {
  if (raw is! List) return null;
  final parsed = <ConfigOption>[];
  for (final entry in raw.take(maxConfigOptions)) {
    // ACP v2 calls it `configId`, and v1 `id`.
    final id = entry is Map ? entry['configId'] ?? entry['id'] : null;
    if (entry is! Map || id is! String || entry['currentValue'] is! String || entry['options'] is! List) continue;
    final choices = _flatChoices(entry['options'] as List)
        .take(maxChoices)
        .whereType<Map>()
        .where((choice) => choice['value'] is String)
        .map((choice) => Choice(choice['value'] as String, shortText(choice['name'], chatLimits) ?? choice['value'] as String))
        .toList();
    final current = entry['currentValue'] as String;
    if (!choices.any((choice) => choice.value == current)) continue;
    parsed.add(ConfigOption(
      id: id,
      label: shortText(entry['name'], chatLimits) ?? id,
      current: current,
      choices: choices,
      category: shortText(entry['category'], chatLimits),
    ));
  }
  return parsed;
}

ConfigOption? parseModes(Object? raw) {
  if (raw is! Map || raw['currentModeId'] is! String || raw['availableModes'] is! List) return null;
  final choices = (raw['availableModes'] as List)
      .take(maxChoices)
      .whereType<Map>()
      .where((mode) => mode['id'] is String)
      .map((mode) => Choice(mode['id'] as String, shortText(mode['name'], chatLimits) ?? mode['id'] as String))
      .toList();
  final current = raw['currentModeId'] as String;
  if (!choices.any((choice) => choice.value == current)) return null;
  return ConfigOption(id: 'mode', label: 'Mode', current: current, choices: choices, isMode: true);
}

final _hidden = RegExp(r'[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200e\u200f\u2028-\u202e\u2066-\u2069]');

/// Text a person is about to approve: nothing in it can reorder or hide the rest, and each line break can be seen.
String _plainly(String text) => text.replaceAll('\r\n', '\n').replaceAll(_hidden, ' ').replaceAll('\n', '⏎\n');

/// What the person sees of what a tool is about to do. Without this, a card can only repeat the title an agent chose. All of what it
/// was given shows, the part that decides it first: a value left out is a value they did not approve.
String? describeToolInput(Object? raw) {
  if (raw is! Map) return null;
  const first = ['command', 'file_path', 'path', 'url', 'pattern', 'query'];
  final keys = [...first.where(raw.containsKey), ...raw.keys.whereType<String>().where((key) => !first.contains(key))];
  final lines = <String>[];
  for (final key in keys) {
    final value = raw[key];
    if (value is String && value.isNotEmpty) {
      lines.add(first.contains(key) && lines.isEmpty ? _plainly(value) : '$key: ${_plainly(value)}');
    } else if (value is num || value is bool) {
      lines.add('$key: $value');
    } else if (value is List || value is Map) {
      lines.add('$key: ${_plainly(jsonEncode(value))}');
    }
  }
  return lines.isEmpty ? null : boundedHead(lines.join('\n'), maxDescribedInput);
}

const int maxDescribedInput = 4000;
