import 'dart:convert';

import 'package:coqui_acp/coqui_acp.dart' show AcpSessionUpdate;

/// Where the conversation shows one `session/update`. [unshown] is the fallback: nothing on screen changes for it.
enum Rendering {
  /// A user's or the agent's message.
  message,

  /// The agent's thinking, folded away.
  thought,

  /// A tool's card: its title, status, content and places.
  toolCard,

  /// A command's output, inside the card of the tool that ran it.
  terminal,

  /// The plan panel above the composer.
  plan,

  /// The command suggestions a `/` brings up in the composer.
  commands,

  /// What the session has used, beside the choosers.
  usage,

  /// The model and mode choosers.
  choices,

  /// The conversation's name, in the list and its bar.
  title,

  /// Working or idle: the Stop button, the status chip and the notice a cancelled or failed turn leaves.
  status,

  unshown,
}

/// How [update] is shown. A message whose content carries no text, such as an image, is [Rendering.unshown].
Rendering renderingOf(AcpSessionUpdate update) => switch (update.kind) {
  'user_message' || 'user_message_chunk' || 'agent_message' || 'agent_message_chunk' =>
    update.text == null ? Rendering.unshown : Rendering.message,
  'agent_thought' || 'agent_thought_chunk' => update.text == null ? Rendering.unshown : Rendering.thought,
  'tool_call' || 'tool_call_update' => Rendering.toolCard,
  'terminal_update' => Rendering.terminal,
  'plan' => Rendering.plan,
  'available_commands_update' => Rendering.commands,
  'usage_update' => Rendering.usage,
  'config_option_update' || 'current_mode_update' => Rendering.choices,
  'session_info_update' => Rendering.title,
  'state_update' => Rendering.status,
  _ => Rendering.unshown,
};

/// Keys whose string values are the protocol's own words, such as a tool's `kind`, and so are kept in a shape.
const _protocolKeys = {'sessionUpdate', 'type', 'status', 'kind', 'state', 'stopReason', 'priority'};
final _word = RegExp(r'^[a-z][a-z0-9_]{0,39}$');
final _key = RegExp(r'^[A-Za-z_][A-Za-z0-9_./-]{0,63}$');

/// The most distinct element shapes one list keeps.
const int maxListShapes = 8;

/// [value] with every string replaced by `"string"`, every number and boolean by its type, and each list by its distinct element
/// shapes. Only a protocol word under one of [_protocolKeys] survives, so what an agent or a person wrote never does.
Object? shapeOf(Object? value, {String? key, int depth = 0}) {
  if (depth > 8) return '…';
  return switch (value) {
    final Map<dynamic, dynamic> map => {
      for (final entry in map.entries)
        _key.hasMatch('${entry.key}') ? '${entry.key}' : 'key': shapeOf(entry.value, key: '${entry.key}', depth: depth + 1),
    },
    final List<dynamic> list => () {
      final shapes = <String, Object?>{};
      for (final item in list) {
        final shape = shapeOf(item, key: key, depth: depth + 1);
        shapes.putIfAbsent(jsonEncode(shape), () => shape);
        if (shapes.length == maxListShapes) break;
      }
      return shapes.values.toList();
    }(),
    final String text when _protocolKeys.contains(key) && _word.hasMatch(text) => text,
    String() => 'string',
    bool() => 'bool',
    int() => 'int',
    double() => 'double',
    null => 'null',
    _ => 'other',
  };
}

/// One distinct shape of update a session received, and how often.
final class SeenUpdate {
  SeenUpdate(this.kind, this.rendering, this.shape);
  final String kind;
  final Rendering rendering;
  final Object? shape;
  int count = 0;

  Map<String, Object?> toJson() => {'kind': kind, 'rendering': rendering.name, 'shape': shape};
}

/// Records the kind and shape of every update a store applies, never its text. Each distinct one goes to [write] once, as a JSON
/// line, the first time it arrives.
final class UpdateRecorder {
  UpdateRecorder({this.write});

  final void Function(String line)? write;
  final Map<String, SeenUpdate> _seen = {};

  List<SeenUpdate> get seen => List.unmodifiable(_seen.values);

  /// The kinds that reached the fallback, which nothing on screen shows.
  Set<String> get unshown => {
    for (final update in _seen.values)
      if (update.rendering == Rendering.unshown) update.kind,
  };

  void record(AcpSessionUpdate update, Rendering rendering) {
    final kind = _word.hasMatch(update.kind) ? update.kind : 'other';
    final seen = SeenUpdate(kind, rendering, shapeOf(update.raw));
    final line = jsonEncode(seen.toJson());
    final known = _seen.putIfAbsent(line, () {
      write?.call(line);
      return seen;
    });
    known.count++;
  }
}
