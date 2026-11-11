import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../ledger/session_ledger.dart' show LedgerBackend;

/// What a permission request would do, in the terms a remembered rule can state. [toolName] is the name the agent gave the tool
/// in its structured fields (`_meta.claudeCode.toolName`), empty when it gave none: a rule for `Bash` never covers `PowerShell`.
sealed class RuleSubject {
  const RuleSubject(this.toolName);
  final String toolName;
}

/// A command the agent would run.
final class CommandSubject extends RuleSubject {
  const CommandSubject(super.toolName, this.command);
  final String command;
}

/// A file the agent would change, as it named it.
final class PathSubject extends RuleSubject {
  const PathSubject(super.toolName, this.path);
  final String path;
}

/// An MCP tool the agent would call, by its structured name (`mcp__<server>__<tool>`).
final class McpSubject extends RuleSubject {
  const McpSubject(super.name, this.server, this.tool);
  final String server, tool;
  String get name => toolName;
}

/// The input keys a command carries that change nothing about what it runs.
const Set<String> _commandKeys = {'command', 'description', 'timeout'};

/// The input keys of a file change besides its path: what to write, never where else or how.
const Set<String> _contentKeys = {'old_string', 'new_string', 'content', 'replace_all', 'text', 'new_content', 'old_text', 'new_text', 'description', 'edits'};

/// The keys one entry of `edits` may carry.
const Set<String> _editKeys = {'old_string', 'new_string', 'replace_all'};

const List<String> _pathKeys = ['file_path', 'path', 'notebook_path'];

final RegExp _mcpName = RegExp(r'^mcp__([^_](?:[^_]|_(?!_))*)__(.+)$');

/// What [rawInput] of a request of [kind] would do, when a rule can say so and nothing in it could mean more than that: an MCP
/// tool the agent named in its structured fields, a command whose input carries only a command, or a change to one file named
/// by an absolute path whose input carries nothing that names another place. Null for anything else, which a rule never covers.
RuleSubject? subjectOf({String? kind, Object? rawInput, Map<String, dynamic>? toolCall}) {
  final meta = toolCall?['_meta'];
  final claude = meta is Map ? meta['claudeCode'] : null;
  final structured = claude is Map && claude['toolName'] is String ? claude['toolName'] as String : '';
  if (structured.isNotEmpty) {
    final match = _mcpName.firstMatch(structured);
    if (match != null) return McpSubject(structured, match.group(1)!, match.group(2)!);
  }
  if (rawInput is! Map) return null;
  switch (kind) {
    case 'execute':
      final command = rawInput['command'];
      if (command is! String || command.isEmpty || rawInput.keys.any((key) => !_commandKeys.contains(key))) return null;
      return CommandSubject(structured, command);
    case 'edit':
      final paths = [for (final key in _pathKeys) if (rawInput.containsKey(key)) rawInput[key]];
      if (paths.length != 1 || paths.single is! String || cleanPath(paths.single as String) == null) return null;
      if (rawInput.keys.any((key) => !_pathKeys.contains(key) && !_contentKeys.contains(key))) return null;
      final edits = rawInput['edits'];
      if (edits != null && (edits is! List || edits.any((edit) => edit is! Map || edit.keys.any((key) => !_editKeys.contains(key))))) {
        return null;
      }
      return PathSubject(structured, paths.single as String);
  }
  return null;
}

final RegExp _drive = RegExp(r'^[A-Za-z]:/');

/// [path] with `.` and `..` resolved and `/` for every separator, lower-cased for a Windows drive path, which has no case.
/// Null unless it is absolute as the agent wrote it: not relative, not starting with `~`, not a UNC or device path, and with no
/// `:` but the one after a drive letter (which also refuses an alternate data stream and a drive-relative `C:x`). Null too when
/// it climbs out of its root. Links are not followed here: see [RealPath].
String? cleanPath(String path) {
  if (path.contains('\u0000') || path.startsWith('~')) return null;
  final text = path.replaceAll('\\', '/');
  if (text.startsWith('//')) return null;
  final drive = _drive.hasMatch(text) ? text.substring(0, 2) : null;
  final rest = drive == null ? text : text.substring(2);
  if (rest.contains(':') || (drive == null && !text.startsWith('/'))) return null;
  final segments = <String>[];
  for (final part in rest.split('/')) {
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

/// Where a change can alter what Canoa or the tools run next, so no path rule ever covers it, at any depth: the folders of
/// version control and of the agents' and editors' own settings and hooks, and these files, which hold instructions, tool servers
/// or the scripts a build or a test starts. Compared without case.
const Set<String> protectedNames = {
  '.git', '.claude', '.codex', '.gemini', '.cursor', '.vscode', '.idea', '.husky',
  '.mcp.json', '.envrc', 'agents.md', 'claude.md', 'gemini.md',
  'package.json', 'makefile', 'justfile', 'pyproject.toml', 'setup.py', 'build.rs', 'rakefile',
};

/// Whether a segment of [cleaned] is one of [protectedNames].
bool touchesProtected(String cleaned) => cleaned.split('/').any((part) => protectedNames.contains(part.toLowerCase()));

/// Folders no path rule may reach: the person's home folder (a scope may not contain it) and Canoa's own configuration folder
/// (a scope may not contain it or lie in it).
final class PermissionGuards {
  const PermissionGuards({this.home, this.config});
  final String? home, config;
}

/// Resolves the links in a path, for what exists of it; null when it cannot. A desktop build resolves them, a browser has no files.
typedef RealPath = String? Function(String path);

/// Why [file] cannot be covered by a rule for [scope] in [folder], or null when it can. All three have to be absolute; the file
/// lies under the scope, the scope under the folder; neither touches [protectedNames] nor reaches the guarded folders; and
/// where [real] is given, the same holds after the links in them are resolved.
String? pathProblem({required String scope, required String file, required String folder, PermissionGuards guards = const PermissionGuards(), RealPath? real}) {
  final s = cleanPath(scope), f = cleanPath(file), d = cleanPath(folder);
  if (s == null || f == null || d == null) return 'A path has to be absolute, with no ~, no : but a drive letter\'s, and not a network path.';
  if (!_under(d, s)) return 'That is not inside $folder.';
  if (!_under(s, f)) return 'That does not cover the file asked.';
  if (touchesProtected(s) || touchesProtected(f)) return 'Version control, settings, hooks and build scripts are never covered.';
  final home = guards.home == null ? null : cleanPath(guards.home!), config = guards.config == null ? null : cleanPath(guards.config!);
  if (home != null && _under(s, home)) return 'That holds your home folder.';
  if (config != null && (_under(s, config) || _under(config, s))) return 'Canoa\'s own settings are never covered.';
  if (real != null) {
    final rs = _resolved(real, s), rf = _resolved(real, f), rd = _resolved(real, d);
    if (rs == null || rf == null || rd == null) return 'A link in that path could not be followed.';
    if (!_under(rd, rs) || !_under(rs, rf) || touchesProtected(rf)) return 'A link in that path leads outside it.';
    if (config != null && _under(config, rf)) return 'Canoa\'s own settings are never covered.';
  }
  return null;
}

String? _resolved(RealPath real, String cleaned) {
  final resolved = real(cleaned);
  return resolved == null ? null : cleanPath(resolved);
}

/// What ends a command or reaches beyond it: control and line characters (hidden ones too), the shell's separators, redirections,
/// substitutions and groupings, and the quotes, `=` and `~` that give an argument a meaning of its own.
final RegExp _beyondPrefix = RegExp(r'''[\u0000-\u001f\u007f-\u009f\u200e\u200f\u2028-\u202e\u2066-\u2069;&|<>`$(){}\\'"=~]''');

/// Characters an exact rule never holds: controls, line breaks, and what hides or reorders text.
final RegExp _hiddenInExact = RegExp(r'[\u0000-\u001f\u007f-\u009f\u00ad\u061c\u200b-\u200f\u2028-\u202e\u2060-\u2069\ufeff]');

/// Programs a prefix rule is never made for: they run what they are given (shells, interpreters, launchers and wrappers), or
/// fetch, copy or rewrite things whatever the arguments (`git` runs hooks and aliases). A person who wants one allows it by an
/// exact rule, or in the agent's own settings. Matched without case or `.exe`.
const Set<String> launcherPrograms = {
  'sh', 'bash', 'zsh', 'fish', 'dash', 'ksh', 'csh', 'tcsh', 'nu', 'pwsh', 'powershell', 'cmd', 'wsl',
  'env', 'sudo', 'su', 'doas', 'xargs', 'find', 'timeout', 'nice', 'nohup', 'time', 'watch', 'eval', 'exec', 'source', 'start',
  'node', 'deno', 'bun', 'ruby', 'perl', 'php', 'lua', 'npx', 'uv', 'uvx', 'make', 'cmake', 'ninja',
  'git', 'tar', 'rsync', 'ssh', 'scp', 'docker', 'podman', 'curl', 'wget',
};

/// With these a package manager runs a script or a program.
const Set<String> packageRunners = {'npm', 'pnpm', 'yarn'};
const Set<String> packageRunWords = {'run', 'run-script', 'exec', 'x', 'dlx'};

final RegExp _programName = RegExp(r'^[A-Za-z][A-Za-z0-9._+-]*$');
final RegExp _pythonName = RegExp(r'^python[0-9.]*$');

/// Why [prefix] cannot stand for the start of a command, or null when it can. It is whole words the person read: at least two,
/// a plain program name first (no assignment, no path) that is not one of [launcherPrograms], and none of the characters that
/// start another command or give an argument a meaning of its own.
String? prefixProblem(String prefix) {
  if (prefix.trim().isEmpty) return 'Name the start of the command.';
  if (prefix != prefix.trim()) return 'It starts and ends with a word, not a space.';
  if (_beyondPrefix.hasMatch(prefix)) return 'It holds a character that starts another command, hides one, or is a quote, = or ~.';
  final words = prefix.split(' ');
  if (words.length < 2 || words.any((word) => word.isEmpty)) return 'Name at least two words, one space apart: a program and what it does.';
  if (!_programName.hasMatch(words.first)) return 'The first word has to be a plain program name.';
  final program = words.first.toLowerCase().replaceFirst(RegExp(r'\.(exe|cmd|bat|ps1)$'), '');
  if (launcherPrograms.contains(program) || _pythonName.hasMatch(program)) return '${words.first} runs whatever it is given, so no prefix covers it.';
  if (packageRunners.contains(program) && packageRunWords.contains(words[1].toLowerCase())) {
    return '${words.first} ${words[1]} runs a script or a program, so no prefix covers it.';
  }
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

  /// Whether it is about commands, which run whatever the folder's files make them do.
  bool get isCommand => this == command || this == prefix;
}

/// One thing the person said to allow, for a source in a folder. Canoa answers a request it matches with the agent's allow-once
/// answer, never an "always", and never when the agent offers no allow once.
final class PermissionRule {
  PermissionRule({
    required this.id,
    required this.source,
    this.peer = '',
    required this.folder,
    required this.tool,
    this.toolName = '',
    required this.type,
    required this.value,
    required this.made,
    this.fired = 0,
    this.lastUsed,
    this.external = false,
  });

  final String id;

  /// The agent source (its identity, not its display name) the request came from, which must be the same.
  final String source;

  /// The peer or provider the session ran on, which must be the same; empty for a source with none.
  final String peer;

  /// The folder the session works in, which must be the same.
  final String folder;

  /// The kind of tool the request names (`execute`, `edit`, ...), which must be the same.
  final String tool;

  /// The tool's structured name (`Bash`, `Edit`, `mcp__server__tool`), which must be the same; empty when the agent sent none.
  final String toolName;
  final RuleType type;

  /// The command, the start of a command, the path, or the MCP tool's name, by [type].
  final String value;
  final DateTime made;
  int fired;
  DateTime? lastUsed;

  /// Whether the stored rule was not written by this app: someone edited the file, or another program did.
  final bool external;

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

  /// Whether a request from [source] on [peer] in [folder] of [kind] doing [subject] is what this rule allows.
  bool matches({
    required String source,
    String peer = '',
    required String folder,
    required String? kind,
    required RuleSubject subject,
    PermissionGuards guards = const PermissionGuards(),
    RealPath? real,
  }) {
    if (source != this.source || peer != this.peer || kind != tool || subject.toolName != toolName) return false;
    final mine = cleanPath(this.folder), here = cleanPath(folder);
    if (mine == null || here == null || mine != here) return false;
    switch ((type, subject)) {
      case (RuleType.command, CommandSubject(:final command)):
        return command == value;
      case (RuleType.prefix, CommandSubject(:final command)):
        return prefixCovers(value, command);
      case (RuleType.path, PathSubject(:final path)):
        return pathProblem(scope: value, file: path, folder: this.folder, guards: guards, real: real) == null;
      case (RuleType.mcp, McpSubject(:final name)):
        return name == value;
      default:
        return false;
    }
  }

  Map<String, Object?> toJson() => {
    'id': id,
    'source': source,
    if (peer.isNotEmpty) 'peer': peer,
    'folder': folder,
    'tool': tool,
    if (toolName.isNotEmpty) 'tool_name': toolName,
    'type': type.name,
    'value': value,
    'made': made.toUtc().toIso8601String(),
    'fired': fired,
    if (lastUsed != null) 'last_used': lastUsed!.toUtc().toIso8601String(),
    if (!external) 'by': 'canoa',
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
      peer: json['peer'] is String ? json['peer'] as String : '',
      folder: json['folder'] as String,
      tool: json['tool'] as String,
      toolName: json['tool_name'] is String ? json['tool_name'] as String : '',
      type: type,
      value: json['value'] as String,
      made: made,
      fired: fired is int && fired >= 0 ? fired : 0,
      lastUsed: DateTime.tryParse('${json['last_used']}'),
      external: json['by'] != 'canoa',
    );
  }
}

/// Why [value] cannot be a rule of [type] for [subject] in [folder], or null when it can. A rule has to cover the very request it
/// is made from, so what the person edits cannot drift from what they were shown.
String? scopeProblem(RuleType type, String value, RuleSubject subject, String folder, {PermissionGuards guards = const PermissionGuards(), RealPath? real}) {
  switch ((type, subject)) {
    case (RuleType.command, CommandSubject(:final command)):
      if (_hiddenInExact.hasMatch(command)) return 'The command holds a control, line-break or hidden character, so only you can allow it, each time.';
      return command == value ? null : 'That is not the command asked.';
    case (RuleType.prefix, CommandSubject(:final command)):
      return prefixProblem(value) ?? (prefixCovers(value, command) ? null : 'That does not cover the command asked.');
    case (RuleType.path, PathSubject(:final path)):
      return pathProblem(scope: value, file: path, folder: folder, guards: guards, real: real);
    case (RuleType.mcp, McpSubject(:final name)):
      return name == value ? null : 'That is not the tool asked.';
    default:
      return 'A rule of that kind cannot cover this request.';
  }
}

/// The rules the person made, kept in a file beside canoa.cfg (IndexedDB in a browser), and a log of each rule made, used and
/// revoked with the full command or path. A change is written at once; before each write the stored file is read again, so a
/// rule someone else added is kept (and flagged) rather than overwritten.
class PermissionRules extends ChangeNotifier {
  PermissionRules(this.backend, {this.log, this.guards = const PermissionGuards(), this.real, DateTime Function()? clock, Random? random})
      : _clock = clock ?? DateTime.now,
        _random = random ?? Random.secure();

  final LedgerBackend backend;

  /// Where the log is kept, one JSON line per event; null keeps none.
  final LedgerBackend? log;
  final PermissionGuards guards;
  final RealPath? real;
  final DateTime Function() _clock;
  final Random _random;
  final List<PermissionRule> _rules = [];
  final Set<String> _revoked = {};
  Future<void> _writing = Future.value();
  Future<void> _logging = Future.value();

  /// The most rules kept; a person who has made this many revokes some before making another.
  static const maxRules = 500;

  /// The most lines the log keeps; the oldest go past it.
  static const maxLogLines = 50000;

  /// Why the stored rules could not be read, if they could not.
  String? problem;

  /// Why the last write of the rules or the log failed, if it did.
  String? writeProblem;

  /// The rules, the oldest first.
  List<PermissionRule> get rules => List.unmodifiable(_rules);

  static List<PermissionRule> _parse(String document) {
    final decoded = jsonDecode(document);
    if (decoded is! Map || decoded['version'] != 1 || decoded['rules'] is! List) {
      throw const FormatException('not a version 1 list of rules');
    }
    return [for (final json in (decoded['rules'] as List).take(maxRules)) ?PermissionRule.fromJson(json)];
  }

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
      _rules
        ..clear()
        ..addAll(_parse(document));
    } on FormatException catch (error) {
      problem = '${backend.location} is not a list of rules (${error.message}), so none apply';
    }
    notifyListeners();
  }

  /// Why a rule of [type] cannot be made for this source and folder beside the rules there: a rule for commands and a rule for
  /// changes to files together let a command write any file or a change write what a command then runs, so only one kind is kept.
  String? conflict(String source, String peer, String folder, RuleType type) {
    final here = cleanPath(folder);
    for (final rule in _rules) {
      if (rule.source != source || rule.peer != peer || cleanPath(rule.folder) != here) continue;
      if (type.isCommand && rule.type == RuleType.path) return 'A rule for changes to files already stands here: revoke it before allowing commands.';
      if (type == RuleType.path && rule.type.isCommand) return 'A rule for commands already stands here: revoke it before allowing changes to files.';
    }
    return null;
  }

  /// Why [value] cannot be a rule of [type] for [subject] in [folder], by [scopeProblem] with this list's guards.
  String? checkScope(RuleType type, String value, RuleSubject subject, String folder) =>
      scopeProblem(type, value, subject, folder, guards: guards, real: real);

  /// The first rule that allows a request, or null. Where a hand-edited file holds both a command rule and a path rule for the
  /// same source and folder, neither applies.
  PermissionRule? match({required String source, String peer = '', required String folder, required String? kind, required RuleSubject subject}) {
    for (final rule in _rules) {
      if (!rule.matches(source: source, peer: peer, folder: folder, kind: kind, subject: subject, guards: guards, real: real)) continue;
      if (rule.type != RuleType.mcp && conflict(rule.source, rule.peer, rule.folder, rule.type) != null) continue;
      return rule;
    }
    return null;
  }

  /// Makes a rule, or returns the one already made with the same meaning; null when [maxRules] are kept. The scope was checked by
  /// [checkScope] and [conflict].
  PermissionRule? add({
    required String source,
    String peer = '',
    required String folder,
    required String kind,
    String toolName = '',
    required RuleType type,
    required String value,
  }) {
    final stored = type == RuleType.path ? cleanPath(value) ?? value : value;
    final same = _rules.where((rule) =>
        rule.source == source && rule.peer == peer && rule.folder == folder && rule.tool == kind && rule.toolName == toolName && rule.type == type && rule.value == stored);
    if (same.isNotEmpty) return same.first;
    if (_rules.length >= maxRules) return null;
    final rule = PermissionRule(
      id: [for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join(),
      source: source,
      peer: peer,
      folder: folder,
      tool: kind,
      toolName: toolName,
      type: type,
      value: stored,
      made: _clock(),
    );
    _rules.add(rule);
    _logEvent('made', rule, rule.value);
    _changed();
    return rule;
  }

  /// Notes that [rule] answered a request that would [detail]: the full command or path.
  void recordUse(PermissionRule rule, String detail) {
    rule.fired++;
    rule.lastUsed = _clock();
    _logEvent('used', rule, detail);
    _changed();
  }

  /// Takes a rule away; what it allowed asks again. Whether there was one.
  bool revoke(String id) {
    final gone = _rules.where((rule) => rule.id == id).firstOrNull;
    if (gone == null) return false;
    _rules.remove(gone);
    _revoked.add(id);
    _logEvent('revoked', gone, gone.value);
    _changed();
    return true;
  }

  String encode() => jsonEncode({'version': 1, 'rules': [for (final rule in _rules) rule.toJson()]});

  void _changed() {
    notifyListeners();
    unawaited(flush());
  }

  /// Adds the rules the stored file holds that this list does not know and did not revoke, as flagged where this app did not
  /// write them.
  Future<void> _mergeStored() async {
    final document = await backend.read();
    if (document == null) return;
    final known = {for (final rule in _rules) rule.id};
    try {
      for (final rule in _parse(document)) {
        if (known.contains(rule.id) || _revoked.contains(rule.id) || _rules.length >= maxRules) continue;
        _rules.add(rule);
      }
    } on FormatException {
      // A file that is not a list is replaced by this one: the person is told by `problem` when it was loaded.
    }
  }

  Future<void> flush() {
    return _writing = _writing.then((_) async {
      try {
        await _mergeStored();
        await backend.write(encode());
        if (writeProblem?.startsWith('could not write the log') != true) writeProblem = null;
      } on Object catch (error) {
        writeProblem = 'could not write ${backend.location}: $error';
        debugPrint('Canoa permissions: $writeProblem');
      }
      notifyListeners();
    });
  }

  void _logEvent(String event, PermissionRule rule, String detail) {
    final target = log;
    if (target == null) return;
    final line = jsonEncode({
      'at': _clock().toUtc().toIso8601String(),
      'event': event,
      'rule': rule.id,
      'source': rule.source,
      'folder': rule.folder,
      'type': rule.type.name,
      'detail': detail,
    });
    _logging = _logging.then((_) async {
      try {
        final lines = ((await target.read()) ?? '').split('\n').where((text) => text.isNotEmpty).toList()..add(line);
        if (lines.length > maxLogLines) lines.removeRange(0, lines.length - maxLogLines);
        await target.write('${lines.join('\n')}\n');
      } on Object catch (error) {
        writeProblem = 'could not write the log ${target.location}: $error';
        debugPrint('Canoa permissions: $writeProblem');
        notifyListeners();
      }
    });
  }

  /// Completes when every change and log line so far is written.
  Future<void> settled() async {
    await _logging;
    await flush();
  }
}
