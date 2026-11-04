import 'dart:convert';

// What an agent reports beyond its messages. All of it comes from the agent, so every field is checked for its type and bounded:
// a wrong one is ignored rather than trusted, and an agent cannot grow this without limit.

const int maxToolText = 256 * 1024;
const int maxTerminalOutput = 256 * 1024;
const int maxPlanEntries = 200;
const int maxCommands = 500;
const int maxConfigOptions = 32;
const int maxChoices = 200;
const int maxToolBlocks = 64;
const int maxLocations = 64;
const int maxShortText = 512;
const int maxTerminals = 256;

final _unprintable = RegExp(r'[\u0000-\u001f\u007f-\u009f\u200e\u200f\u2028-\u202e\u2066-\u2069]');

String? shortText(Object? value) {
  if (value is! String) return null;
  // Format characters can reorder what is shown around them.
  final clean = value.replaceAll(_unprintable, ' ').trim();
  if (clean.isEmpty) return null;
  return clean.length > maxShortText ? '${clean.substring(0, maxShortText)}…' : clean;
}

String boundedTail(String text, int limit) => text.length > limit ? text.substring(text.length - limit) : text;

String boundedHead(String text, int limit) => text.length > limit ? '${text.substring(0, limit)}\n…' : text;

/// [cancelled] is ACP v2's: a tool a stopped turn never ran.
enum ToolStatus { pending, running, done, failed, cancelled }

ToolStatus? _toolStatus(Object? value) => switch (value) {
  'pending' => ToolStatus.pending,
  'in_progress' => ToolStatus.running,
  'completed' => ToolStatus.done,
  'failed' => ToolStatus.failed,
  'cancelled' => ToolStatus.cancelled,
  _ => null,
};

sealed class ToolBlock {
  const ToolBlock();
}

final class ToolText extends ToolBlock {
  const ToolText(this.text);
  final String text;
}

/// Whole-file text before and after. No [oldText] is a new file.
final class ToolDiff extends ToolBlock {
  const ToolDiff({required this.path, required this.newText, this.oldText});
  final String path;
  final String? oldText;
  final String newText;
}

/// A change an agent already wrote as a unified patch.
final class ToolPatch extends ToolBlock {
  const ToolPatch(this.text);
  final String text;
}

final class ToolTerminal extends ToolBlock {
  const ToolTerminal(this.terminalId);
  final String terminalId;
}

ToolBlock? _toolBlock(Object? raw) {
  if (raw is! Map) return null;
  final type = raw['type'];
  if (type == 'content') {
    final content = raw['content'];
    if (content is Map && content['type'] == 'text' && content['text'] is String) {
      return ToolText(boundedHead(content['text'] as String, maxToolText));
    }
    return null;
  }
  if (type == 'diff' && raw['newText'] is String) {
    return ToolDiff(
      path: shortText(raw['path']) ?? 'a file',
      oldText: raw['oldText'] is String ? boundedHead(raw['oldText'] as String, maxToolText) : null,
      newText: boundedHead(raw['newText'] as String, maxToolText),
    );
  }
  if (type == 'terminal' && raw['terminalId'] is String) {
    return ToolTerminal(raw['terminalId'] as String);
  }
  final patch = raw['patch'];
  if (patch is Map && patch['text'] is String) {
    return ToolPatch(boundedHead(patch['text'] as String, maxToolText));
  }
  return null;
}

/// One tool the agent is using, built up from however many updates it sends: a field an update leaves out keeps its value.
final class ToolCall {
  ToolCall(this.id);
  final String id;
  String title = 'A tool';
  String? kind;
  ToolStatus status = ToolStatus.pending;
  List<ToolBlock> blocks = const [];
  List<String> locations = const [];

  void apply(Map<String, dynamic> raw) {
    title = shortText(raw['title']) ?? (title == 'A tool' ? shortText(raw['name']) ?? title : title);
    kind = shortText(raw['kind']) ?? kind;
    status = _toolStatus(raw['status']) ?? status;
    final content = raw['content'];
    if (raw['sessionUpdate'] == 'tool_call_content_chunk') {
      // One more item after what came before; past the limit the oldest goes.
      final block = _toolBlock(content);
      if (block != null) blocks = [...blocks.skip(blocks.length >= maxToolBlocks ? 1 : 0), block];
    } else if (content is List) {
      final parsed = content.take(maxToolBlocks).map(_toolBlock).whereType<ToolBlock>().toList();
      // An update with an empty list is how one starts, not a request to forget what came before.
      if (parsed.isNotEmpty) blocks = parsed;
    }
    final places = raw['locations'];
    if (places is List && places.isNotEmpty) {
      locations = places
          .take(maxLocations)
          .whereType<Map>()
          .map((place) {
            final path = shortText(place['path']);
            if (path == null) return null;
            return place['line'] is int ? '$path:${place['line']}' : path;
          })
          .whereType<String>()
          .toList();
    }
  }
}

/// Terminal output as ACP v2 sends it, base64-encoded bytes, as text. What is not base64 is shown as it came.
String terminalText(String data) {
  try {
    return utf8.decode(base64.decode(data), allowMalformed: true);
  } on FormatException {
    return data;
  }
}

/// A command the agent ran, as it reports it. The output keeps its end, which is where a failure says why.
final class TerminalView {
  String? command;
  String output = '';
  int? exitCode;

  /// A `terminal_update`, whose output replaces what came before, or a `terminal_output_chunk`, which adds to it.
  void apply(Map<String, dynamic> raw) {
    command = shortText(raw['command']) ?? command;
    final said = raw['output'];
    if (said is Map && said['data'] is String) {
      output = boundedTail(terminalText(said['data'] as String), maxTerminalOutput);
    }
    if (raw['sessionUpdate'] == 'terminal_output_chunk' && raw['data'] is String) {
      output = boundedTail(output + terminalText(raw['data'] as String), maxTerminalOutput);
    }
    final exit = raw['exitStatus'];
    if (exit is Map && exit['exitCode'] is int) exitCode = exit['exitCode'] as int;
  }
}

enum PlanStatus { pending, running, done, cancelled }

final class PlanEntry {
  const PlanEntry(this.content, this.status);
  final String content;
  final PlanStatus status;
}

List<PlanEntry>? parsePlan(Object? raw) {
  if (raw is! List) return null;
  return raw
      .take(maxPlanEntries)
      .whereType<Map>()
      .map((entry) {
        final content = shortText(entry['content']);
        if (content == null) return null;
        final status = switch (entry['status']) {
          'in_progress' => PlanStatus.running,
          'completed' => PlanStatus.done,
          'cancelled' => PlanStatus.cancelled,
          _ => PlanStatus.pending,
        };
        return PlanEntry(content, status);
      })
      .whereType<PlanEntry>()
      .toList();
}

enum CompactionStatus { running, done, failed, cancelled }

/// The entries of a v2 `plan_update`'s plan, when it is a list of items; null for a plan content type this client does not know.
Object? planItems(Object? plan) => plan is Map && plan['type'] == 'items' ? plan['entries'] : null;

String _texts(Iterable<Object?> blocks) =>
    blocks.whereType<Map>().where((block) => block['type'] == 'text' && block['text'] is String).map((block) => block['text']).join();

/// One compaction of the agent's context, built up from its `compaction_update`s and `compaction_summary_chunk`s.
final class Compaction {
  Compaction(this.id);
  final String id;
  CompactionStatus status = CompactionStatus.running;
  String summary = '';
  String? error;

  void apply(Map<String, dynamic> raw) {
    if (raw['sessionUpdate'] == 'compaction_summary_chunk') {
      summary = boundedHead(summary + _texts([raw['content']]), maxToolText);
      return;
    }
    final now = switch (raw['status']) {
      'in_progress' => CompactionStatus.running,
      'completed' => CompactionStatus.done,
      'failed' => CompactionStatus.failed,
      'cancelled' => CompactionStatus.cancelled,
      _ => null,
    };
    status = now ?? status;
    // Both are patches: a field left out keeps its value, and null clears it.
    if (raw.containsKey('summary')) {
      final said = raw['summary'];
      summary = said is List ? boundedHead(_texts(said), maxToolText) : '';
    }
    if (raw.containsKey('error')) error = shortText(raw['error']);
  }
}

/// The names of what went with a person's message, from its content: one block, as a chunk has, or a list of them.
List<String> attachedNames(Object? content) {
  final blocks = content is List ? content.take(maxToolBlocks) : [content];
  return [
    for (final block in blocks.whereType<Map>())
      ?switch (block['type']) {
        'resource' => _nameOf(block['resource'] is Map ? (block['resource'] as Map)['uri'] : null) ?? 'a file',
        'resource_link' => shortText(block['name']) ?? _nameOf(block['uri']) ?? 'a file',
        'image' => _nameOf(block['uri']) ?? 'an image',
        'audio' => 'a recording',
        _ => null,
      },
  ];
}

/// The last part of [uri]'s path, which is the file's name for `attachment:notes.txt` and `file:///work/notes.txt` alike.
String? _nameOf(Object? uri) {
  if (uri is! String) return null;
  final parsed = Uri.tryParse(uri);
  final segments = parsed?.pathSegments.where((segment) => segment.isNotEmpty) ?? const <String>[];
  return shortText(segments.isEmpty ? uri : segments.last);
}

final class AgentCommandInfo {
  const AgentCommandInfo(this.name, this.description);
  final String name;
  final String description;
}

List<AgentCommandInfo>? parseCommands(Object? raw) {
  if (raw is! List) return null;
  return raw
      .take(maxCommands)
      .whereType<Map>()
      .map((entry) {
        final name = shortText(entry['name']);
        return name == null ? null : AgentCommandInfo(name, shortText(entry['description']) ?? '');
      })
      .whereType<AgentCommandInfo>()
      .toList();
}

final class Choice {
  const Choice(this.value, this.label);
  final String value;
  final String label;
}

/// Something the agent lets a person choose, such as its model.
final class ConfigOption {
  const ConfigOption({required this.id, required this.label, required this.current, required this.choices, this.isMode = false});
  final String id;
  final String label;
  final String current;
  final List<Choice> choices;

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
        .map((choice) => Choice(choice['value'] as String, shortText(choice['name']) ?? choice['value'] as String))
        .toList();
    final current = entry['currentValue'] as String;
    if (!choices.any((choice) => choice.value == current)) continue;
    parsed.add(ConfigOption(
      id: id,
      label: shortText(entry['name']) ?? id,
      current: current,
      choices: choices,
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
      .map((mode) => Choice(mode['id'] as String, shortText(mode['name']) ?? mode['id'] as String))
      .toList();
  final current = raw['currentModeId'] as String;
  if (!choices.any((choice) => choice.value == current)) return null;
  return ConfigOption(id: 'mode', label: 'Mode', current: current, choices: choices, isMode: true);
}

/// How much of the agent's context is used, and what the session has cost, as the agent reports them.
final class SessionUsage {
  const SessionUsage({this.used, this.size, this.cost, this.currency});
  final int? used;
  final int? size;
  final double? cost;
  final String? currency;

  SessionUsage applied(Map<String, dynamic> raw) {
    final said = raw['cost'];
    final amount = said is Map && said['amount'] is num ? (said['amount'] as num).toDouble() : null;
    return SessionUsage(
      used: raw['used'] is int ? raw['used'] as int : used,
      size: raw['size'] is int ? raw['size'] as int : size,
      cost: amount ?? cost,
      currency: said is Map ? shortText(said['currency']) ?? currency : currency,
    );
  }

  String? describe() {
    final parts = <String>[
      if (used != null && size != null && size! > 0) '${(100 * used! / size!).clamp(0, 100).round()}% of context',
      if (cost != null) '${cost!.toStringAsFixed(2)} ${currency ?? ''}'.trim(),
    ];
    return parts.isEmpty ? null : parts.join(' · ');
  }
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

/// Whether what [describeToolInput] gave is more than its box shows at once, which a person has to be told.
bool isLongToApprove(String detail) => detail.length > 300 || '\n'.allMatches(detail).length > 4;
