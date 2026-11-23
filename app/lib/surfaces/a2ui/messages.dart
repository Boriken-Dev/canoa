/// A2UI 1.0 messages from an agent to the renderer: the envelope, which carries `"version":"1.0"` and exactly one message key, and
/// what each key holds. Anything else is an [A2uiProblem], never guessed at.
library;

import 'dart:convert';

import 'limits.dart';

/// The one version this reader takes.
const String a2uiVersion = '1.0';

/// A message the renderer cannot take or a surface it cannot draw, in the shape the protocol's `error` message has.
final class A2uiProblem {
  const A2uiProblem(this.code, this.message, {this.surfaceId = '', this.path = ''});

  static const validationFailed = 'VALIDATION_FAILED';
  static const unallowedParent = 'UNALLOWED_PARENT';
  static const unallowedChild = 'UNALLOWED_CHILD';

  final String code, message, surfaceId, path;

  /// The `error` message sent back to the agent.
  Map<String, Object?> toMessage() => {
    'version': a2uiVersion,
    'error': {'code': code, 'surfaceId': surfaceId, 'path': path, 'message': message},
  };

  @override
  String toString() => '$code${surfaceId.isEmpty ? '' : ' $surfaceId'}${path.isEmpty ? '' : ' $path'}: $message';
}

/// One message of the four an agent sends to draw.
sealed class A2uiMessage {
  const A2uiMessage(this.surfaceId);
  final String surfaceId;
}

final class CreateSurface extends A2uiMessage {
  const CreateSurface(super.surfaceId, this.catalogId, this.sendDataModel, this.components, this.dataModel);
  final String? catalogId;
  final bool sendDataModel;
  final List<Object?> components;
  final Object? dataModel;
}

final class UpdateComponents extends A2uiMessage {
  const UpdateComponents(super.surfaceId, this.components);
  final List<Object?> components;
}

final class UpdateDataModel extends A2uiMessage {
  const UpdateDataModel(super.surfaceId, this.path, this.value);

  /// A JSON Pointer; `/` is the whole model.
  final String path;

  /// Null removes what [path] names.
  final Object? value;
}

final class DeleteSurface extends A2uiMessage {
  const DeleteSurface(super.surfaceId);
}

/// What reading a message gave: [message], or [problem].
final class A2uiRead {
  const A2uiRead.message(A2uiMessage this.message) : problem = null;
  const A2uiRead.problem(A2uiProblem this.problem) : message = null;
  final A2uiMessage? message;
  final A2uiProblem? problem;
}

/// Reads one message. A function call either way (`callRendererFunction`, `callAgentFunction`) is a problem: no function is run for an
/// agent here beyond the catalog's own.
A2uiRead readA2uiMessage(Object? raw, A2uiLimits limits) {
  A2uiRead bad(String text, {String surface = '', String path = ''}) =>
      A2uiRead.problem(A2uiProblem(A2uiProblem.validationFailed, text, surfaceId: surface, path: path));
  if (raw is! Map) return bad('A message is an object');
  if (raw['version'] != a2uiVersion) return bad('The version is "$a2uiVersion"');
  final keys = [
    for (final key in raw.keys)
      if (key != 'version') '$key',
  ];
  if (keys.length != 1) return bad('A message has exactly one key beside its version, not ${keys.length}');
  try {
    if (utf8.encode(jsonEncode(raw)).length > limits.maxMessageBytes) {
      return bad('A message holds at most ${limits.maxMessageBytes} bytes');
    }
  } on Object {
    return bad('A message is JSON');
  }
  final key = keys.single;
  final body = raw[key];
  if (body is! Map) return bad('"$key" holds an object');
  final surface = body['surfaceId'];
  switch (key) {
    case 'createSurface' || 'updateComponents' || 'updateDataModel' || 'deleteSurface':
      if (surface is! String || surface.isEmpty) return bad('"$key" names a surfaceId');
    case 'callRendererFunction' || 'callAgentFunction' || 'agentFunctionResponse':
      return bad('Functions are not run for an agent: "$key" is not taken', surface: surface is String ? surface : '');
    default:
      return bad('"$key" is not a message an agent sends');
  }
  final id = surface;
  switch (key) {
    case 'createSurface':
      if (body.containsKey('theme')) return bad('A surface carries no theme: the app draws it', surface: id, path: '/createSurface/theme');
      final catalog = body['catalogId'];
      final components = body['components'];
      if (catalog != null && catalog is! String) return bad('catalogId is a string', surface: id, path: '/createSurface/catalogId');
      if (components != null && components is! List) return bad('components is a list', surface: id, path: '/createSurface/components');
      return A2uiRead.message(
        CreateSurface(id, catalog as String?, body['sendDataModel'] == true, (components as List<Object?>?) ?? const [], body['dataModel']),
      );
    case 'updateComponents':
      final components = body['components'];
      if (components is! List) return bad('components is a list', surface: id, path: '/updateComponents/components');
      return A2uiRead.message(UpdateComponents(id, components));
    case 'updateDataModel':
      final path = body['path'];
      if (path != null && path is! String) return bad('path is a JSON Pointer', surface: id, path: '/updateDataModel/path');
      return A2uiRead.message(UpdateDataModel(id, (path as String?) ?? '/', body['value']));
    default:
      return A2uiRead.message(DeleteSurface(id));
  }
}

/// The messages an update's `_meta.a2ui` carries, with where to show them: a message, a list of messages, or
/// `{"messages": [...], "target": "side_panel"|"tab"|"window", "title": "..."}`. Null when it carries none.
({List<Object?> messages, String? target, String? title})? a2uiCarried(Object? meta) {
  if (meta is! Map) return null;
  final carried = meta['a2ui'];
  if (carried is List) return (messages: carried, target: null, title: null);
  if (carried is! Map) return null;
  if (carried.containsKey('version')) return (messages: [carried], target: null, title: null);
  final messages = carried['messages'];
  if (messages is! List) return null;
  return (
    messages: messages,
    target: carried['target'] is String ? carried['target'] as String : null,
    title: carried['title'] is String ? carried['title'] as String : null,
  );
}

/// The JSON Pointer [path] as its reference tokens; `/` and the empty pointer are the document itself. Null for what is no pointer.
List<String>? pointerTokens(String path) {
  if (path.isEmpty || path == '/') return const [];
  if (!path.startsWith('/')) return null;
  return [for (final part in path.substring(1).split('/')) part.replaceAll('~1', '/').replaceAll('~0', '~')];
}

/// The value [path] names in [root], and whether there is one.
(bool, Object?) pointerRead(Object? root, String path) {
  final tokens = pointerTokens(path);
  if (tokens == null) return (false, null);
  Object? at = root;
  for (final token in tokens) {
    if (at is Map && at.containsKey(token)) {
      at = at[token];
    } else if (at is List) {
      final index = int.tryParse(token);
      if (index == null || index < 0 || index >= at.length) return (false, null);
      at = at[index];
    } else {
      return (false, null);
    }
  }
  return (true, at);
}

/// [root] with [value] set at [path] (what [path] names taken out when [removed]), copied where it changes; null when [path] cannot be
/// written.
Object? pointerWrite(Object? root, String path, Object? value, {bool removed = false}) {
  final tokens = pointerTokens(path);
  if (tokens == null) return null;
  if (tokens.isEmpty) return removed ? <String, Object?>{} : value;
  Object? set(Object? at, int index) {
    final token = tokens[index];
    final last = index == tokens.length - 1;
    if (at is List) {
      final i = int.tryParse(token);
      if (i == null || i < 0 || i > at.length) throw const FormatException('index');
      final copy = [...at];
      if (last) {
        if (removed) {
          if (i < copy.length) copy.removeAt(i);
        } else if (i == copy.length) {
          copy.add(value);
        } else {
          copy[i] = value;
        }
      } else {
        copy[i] = set(i < copy.length ? copy[i] : null, index + 1);
      }
      return copy;
    }
    final map = at is Map ? {for (final entry in at.entries) '${entry.key}': entry.value} : <String, Object?>{};
    if (last) {
      removed ? map.remove(token) : map[token] = value;
    } else {
      map[token] = set(map[token], index + 1);
    }
    return map;
  }

  try {
    return set(root, 0);
  } on FormatException {
    return null;
  }
}
