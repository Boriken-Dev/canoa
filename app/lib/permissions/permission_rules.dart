import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../ledger/session_ledger.dart' show LedgerBackend;

/// What a permission request would do, in the terms a remembered rule can state.
sealed class RuleSubject {
  const RuleSubject();
}

/// A command the agent would run.
final class CommandSubject extends RuleSubject {
  const CommandSubject(this.command);
  final String command;
}

/// A file the agent would change, as it named it.
final class PathSubject extends RuleSubject {
  const PathSubject(this.path);
  final String path;
}

/// An MCP tool the agent would call, by the name the agent gave it (`mcp__<server>__<tool>`).
final class McpSubject extends RuleSubject {
  const McpSubject(this.name, this.server, this.tool);
  final String name, server, tool;
}

/// The input keys a command carries that change nothing about what it runs.
const Set<String> _commandKeys = {'command', 'description', 'timeout'};

/// The input keys of a file change besides its path: what to write, never where else or how.
const Set<String> _contentKeys = {
  'old_string', 'new_string', 'content', 'edits', 'replace_all', 'diff', 'text', 'new_content', 'old_text', 'new_text', 'patch',
  'description',
};

const List<String> _pathKeys = ['file_path', 'path', 'notebook_path'];

final RegExp _mcpName = RegExp(r'^mcp__(.+?)__(.+)$');

/// What [rawInput] of a request of [kind] would do, when a rule can say so and nothing in it could mean more than that:
/// a command whose input carries only a command, a change to one named file whose input carries nothing that names another
/// place, or an MCP tool. Null for anything else, which a rule never covers.
RuleSubject? subjectOf({String? kind, String? title, Object? rawInput, Map<String, dynamic>? toolCall}) {
  final meta = toolCall?['_meta'];
  final claude = meta is Map ? meta['claudeCode'] : null;
  final named = claude is Map && claude['toolName'] is String ? claude['toolName'] as String : title;
  if (named != null) {
    final match = _mcpName.firstMatch(named);
    if (match != null) return McpSubject(named, match.group(1)!, match.group(2)!);
  }
  if (rawInput is! Map) return null;
  switch (kind) {
    case 'execute':
      final command = rawInput['command'];
      if (command is! String || command.isEmpty || rawInput.keys.any((key) => !_commandKeys.contains(key))) return null;
      return CommandSubject(command);
    case 'edit':
      final paths = [for (final key in _pathKeys) if (rawInput.containsKey(key)) rawInput[key]];
      if (paths.length != 1 || paths.single is! String || (paths.single as String).isEmpty) return null;
      if (rawInput.keys.any((key) => !_pathKeys.contains(key) && !_contentKeys.contains(key))) return null;
      return PathSubject(paths.single as String);
  }
  return null;
}

final RegExp _drive = RegExp(r'^[A-Za-z]:/');

/// [path] made absolute against [base] when it is not, with `.` and `..` resolved and `/` for every separator, lower-cased
/// for a Windows drive path, which has no case. Null when it cannot be made absolute or climbs out of its root. Links are
/// not followed: a path through one is read as written.
String? cleanPath(String path, {String? base}) {
  if (path.contains('\u0000')) return null;
  var text = path.replaceAll('\\', '/');
  final absolute = text.startsWith('/') || _drive.hasMatch(text);
  if (!absolute) {
    if (base == null) return null;
    final cleanedBase = cleanPath(base);
    if (cleanedBase == null) return null;
    text = '$cleanedBase/$text';
  }
  final drive = _drive.hasMatch(text) ? text.substring(0, 2) : null;
  final segments = <String>[];
  for (final part in (drive == null ? text : text.substring(2)).split('/')) {
    if (part.isEmpty || part == '.') continue;
    if (part == '..') {
      if (segments.isEmpty) return null;
      segments.removeLast();
    } else {
      segments.add(part);
    }
  }
  final cleaned = '${drive ?? ''}/${segments.join('/')}';
  return drive == null ? cleaned : cleaned.toLowerCase();
}

/// Whether [inner] is [outer] or lies under it, both already made by [cleanPath].
bool _under(String outer, String inner) => inner == outer || inner.startsWith(outer.endsWith('/') ? outer : '$outer/');

/// Whether [path] (relative to [folder] when it is relative) is [folder] or lies under it, both read by [cleanPath].
bool pathInside(String folder, String path) {
  final inner = cleanPath(path, base: folder), outer = cleanPath(folder);
  return inner != null && outer != null && _under(outer, inner);
}

/// What ends a command or reaches beyond it: control and line characters (hidden ones too), and the shell's separators,
/// redirections, substitutions and groupings.
final RegExp _beyondPrefix = RegExp(r'[\u0000-\u001f\u007f-\u009f\u200e\u200f\u2028-\u202e\u2066-\u2069;&|<>`$(){}\\]');

/// Why [prefix] cannot stand for the start of a command, or null when it can. It is a prefix the person read: whole words, and
/// none of the characters that start another command.
String? prefixProblem(String prefix) {
  if (prefix.trim().isEmpty) return 'Name the start of the command.';
  if (prefix != prefix.trim()) return 'It starts and ends with a word, not a space.';
  if (_beyondPrefix.hasMatch(prefix)) return 'It holds a character that starts another command or hides one.';
  return null;
}

/// Whether [command] is [prefix] followed only by arguments: a space, then nothing that starts another command. `npm test`
/// covers `npm test -- a.js`, and neither `npm testing` nor `npm test; rm -rf ~`.
bool prefixCovers(String prefix, String command) {
  if (prefixProblem(prefix) != null || !command.startsWith(prefix)) return false;
  final rest = command.substring(prefix.length);
  return rest.isEmpty || (rest.startsWith(' ') && !_beyondPrefix.hasMatch(rest));
}

/// How a rule says what it allows.
enum RuleType {
  /// This command, exactly.
  command,

  /// A command that starts with these words and carries only arguments.
  prefix,

  /// A change to this file, or any under this folder.
  path,

  /// This MCP tool, with any input.
  mcp;

  static RuleType? named(Object? name) => values.where((type) => type.name == name).firstOrNull;
}

/// One thing the person said to allow, for a source in a folder. Canoa answers a request it matches with the agent's allow-once
/// answer, never an "always", and never when the agent offers no allow once.
final class PermissionRule {
  PermissionRule({
    required this.id,
    required this.source,
    required this.folder,
    required this.tool,
    required this.type,
    required this.value,
    required this.made,
    this.fired = 0,
    this.lastUsed,
  });

  final String id;

  /// The agent source (its entry label) the request came from, which must be the same.
  final String source;

  /// The folder the session works in, which must be the same.
  final String folder;

  /// The kind of tool the request names (`execute`, `edit`, ...), which must be the same.
  final String tool;
  final RuleType type;

  /// The command, the start of a command, the path, or the MCP tool's name, by [type].
  final String value;
  final DateTime made;
  int fired;
  DateTime? lastUsed;

  /// What it allows, in words a person reads.
  String get describe => switch (type) {
    RuleType.command => 'Run: $value',
    RuleType.prefix => 'Run a command starting with: $value',
    RuleType.path => 'Change $value, or a file under it',
    RuleType.mcp => switch (_mcpName.firstMatch(value)) {
      final match? => 'Use the tool ${match.group(2)} of ${match.group(1)}, with any input',
      null => 'Use the tool $value, with any input',
    },
  };

  /// Whether a request from [source] in [folder] of [kind] doing [subject] is what this rule allows.
  bool matches({required String source, required String folder, required String? kind, required RuleSubject subject}) {
    if (source != this.source || kind != tool) return false;
    final mine = cleanPath(this.folder), here = cleanPath(folder);
    if (mine == null || here == null || mine != here) return false;
    switch ((type, subject)) {
      case (RuleType.command, CommandSubject(:final command)):
        return command == value;
      case (RuleType.prefix, CommandSubject(:final command)):
        return prefixCovers(value, command);
      case (RuleType.path, PathSubject(:final path)):
        final scope = cleanPath(value, base: this.folder), file = cleanPath(path, base: folder);
        return scope != null && file != null && pathInside(this.folder, scope) && _under(scope, file);
      case (RuleType.mcp, McpSubject(:final name)):
        return name == value;
      default:
        return false;
    }
  }

  Map<String, Object?> toJson() => {
    'id': id,
    'source': source,
    'folder': folder,
    'tool': tool,
    'type': type.name,
    'value': value,
    'made': made.toUtc().toIso8601String(),
    'fired': fired,
    if (lastUsed != null) 'last_used': lastUsed!.toUtc().toIso8601String(),
  };

  static PermissionRule? fromJson(Object? json) {
    if (json is! Map) return null;
    final type = RuleType.named(json['type']);
    if (type == null) return null;
    for (final key in ['id', 'source', 'folder', 'tool', 'value']) {
      if (json[key] is! String || (json[key] as String).isEmpty) return null;
    }
    final made = DateTime.tryParse('${json['made']}');
    if (made == null) return null;
    final fired = json['fired'];
    return PermissionRule(
      id: json['id'] as String,
      source: json['source'] as String,
      folder: json['folder'] as String,
      tool: json['tool'] as String,
      type: type,
      value: json['value'] as String,
      made: made,
      fired: fired is int && fired >= 0 ? fired : 0,
      lastUsed: DateTime.tryParse('${json['last_used']}'),
    );
  }
}

/// Why [value] cannot be a rule of [type] for [subject] in [folder], or null when it can. A rule has to cover the very request
/// it is made from, so what the person edits cannot drift from what they were shown.
String? scopeProblem(RuleType type, String value, RuleSubject subject, String folder) {
  switch ((type, subject)) {
    case (RuleType.command, CommandSubject(:final command)):
      return command == value ? null : 'That is not the command asked.';
    case (RuleType.prefix, CommandSubject(:final command)):
      return prefixProblem(value) ?? (prefixCovers(value, command) ? null : 'That does not cover the command asked.');
    case (RuleType.path, PathSubject(:final path)):
      final scope = cleanPath(value, base: folder), file = cleanPath(path, base: folder);
      if (scope == null || !pathInside(folder, scope)) return 'That is not inside $folder.';
      return file != null && _under(scope, file) ? null : 'That does not cover the file asked.';
    case (RuleType.mcp, McpSubject(:final name)):
      return name == value ? null : 'That is not the tool asked.';
    default:
      return 'A rule of that kind cannot cover this request.';
  }
}

/// The rules the person made, kept in a file beside canoa.cfg (IndexedDB in a browser). A change is written at once.
class PermissionRules extends ChangeNotifier {
  PermissionRules(this.backend, {DateTime Function()? clock, Random? random})
      : _clock = clock ?? DateTime.now,
        _random = random ?? Random.secure();

  final LedgerBackend backend;
  final DateTime Function() _clock;
  final Random _random;
  final List<PermissionRule> _rules = [];
  Future<void> _writing = Future.value();

  /// The most rules kept; a person who has made this many revokes some before making another.
  static const maxRules = 500;

  /// Why the stored rules could not be read, if they could not.
  String? problem;

  /// The rules, the oldest first.
  List<PermissionRule> get rules => List.unmodifiable(_rules);

  Future<void> load() async {
    final String? document;
    try {
      document = await backend.read();
    } on Object catch (error) {
      problem = 'could not read ${backend.location}: $error';
      notifyListeners();
      return;
    }
    if (document == null) return;
    try {
      final decoded = jsonDecode(document);
      if (decoded is! Map || decoded['version'] != 1 || decoded['rules'] is! List) {
        throw const FormatException('not a version 1 list of rules');
      }
      _rules
        ..clear()
        ..addAll([for (final json in (decoded['rules'] as List).take(maxRules)) ?PermissionRule.fromJson(json)]);
    } on FormatException catch (error) {
      problem = '${backend.location} is not a list of rules (${error.message}), so none apply';
    }
    notifyListeners();
  }

  /// The first rule that allows a request, or null.
  PermissionRule? match({required String source, required String folder, required String? kind, required RuleSubject subject}) {
    for (final rule in _rules) {
      if (rule.matches(source: source, folder: folder, kind: kind, subject: subject)) return rule;
    }
    return null;
  }

  /// Makes a rule, or returns the one already made with the same meaning; null when [maxRules] are kept. The scope was
  /// checked by [scopeProblem].
  PermissionRule? add({required String source, required String folder, required String kind, required RuleType type, required String value}) {
    final stored = type == RuleType.path ? cleanPath(value, base: folder) ?? value : value;
    final same = _rules.where((rule) => rule.source == source && rule.folder == folder && rule.tool == kind && rule.type == type && rule.value == stored);
    if (same.isNotEmpty) return same.first;
    if (_rules.length >= maxRules) return null;
    final rule = PermissionRule(
      id: [for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join(),
      source: source,
      folder: folder,
      tool: kind,
      type: type,
      value: stored,
      made: _clock(),
    );
    _rules.add(rule);
    _changed();
    return rule;
  }

  /// Notes that [rule] answered a request.
  void recordUse(PermissionRule rule) {
    rule.fired++;
    rule.lastUsed = _clock();
    _changed();
  }

  /// Takes a rule away; what it allowed asks again. Whether there was one.
  bool revoke(String id) {
    final before = _rules.length;
    _rules.removeWhere((rule) => rule.id == id);
    if (_rules.length == before) return false;
    _changed();
    return true;
  }

  String encode() => jsonEncode({'version': 1, 'rules': [for (final rule in _rules) rule.toJson()]});

  void _changed() {
    notifyListeners();
    unawaited(flush());
  }

  Future<void> flush() {
    final document = encode();
    return _writing = _writing.then((_) => backend.write(document)).catchError((Object error) {
      debugPrint('Canoa permissions: could not write ${backend.location}: $error');
    });
  }
}
