import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:crypto/crypto.dart';
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

/// Files that run code when a build, a test, an editor or an agent starts, and the files agents read their instructions from. A
/// path rule never covers one, at any depth, with or without its case or a trailing dot or space. Every segment that starts with a
/// dot is protected as well (see [protectedSegment]).
const Set<String> protectedFiles = {
  'package.json', 'makefile', 'justfile', 'pyproject.toml', 'setup.py', 'build.rs', 'rakefile', 'cargo.toml', 'conftest.py',
  'agents.md', 'agents.override.md', 'claude.md', 'claude.local.md', 'gemini.md',
  'directory.build.props', 'directory.build.targets', 'mise.toml', 'node_modules',
};

final RegExp _shortName = RegExp(r'~[0-9]');
final RegExp _configFile = RegExp(r'\.config\.(js|mjs|cjs|ts)$');

/// Whether one path segment below the session's folder is out of reach of a path rule. The segment is first read the way a file
/// system reads it: Windows drops trailing dots and spaces, so `.claude.` and `package.json.` are the files they look like, and
/// an 8.3 short name (`GIT~1`) is another spelling of a long one. Anything but printable ASCII is refused rather than normalised,
/// since Unicode normalisation and ignorable characters give one name many spellings. Then it is protected when it starts with a
/// dot, is one of [protectedFiles], or is a `*.config.js`, `.mjs`, `.cjs` or `.ts`.
bool protectedSegment(String raw) {
  if (raw.codeUnits.any((unit) => unit < 0x20 || unit > 0x7e)) return true;
  var read = raw;
  while (read.endsWith('.') || read.endsWith(' ')) {
    read = read.substring(0, read.length - 1);
  }
  if (read.isEmpty || raw.startsWith('.') || read.startsWith('.')) return true;
  final lower = read.toLowerCase();
  return _shortName.hasMatch(lower) || protectedFiles.contains(lower) || _configFile.hasMatch(lower);
}

/// Whether [inner] (under [outer], both made by [cleanPath]) goes through a protected segment below [outer].
bool _touchesProtected(String outer, String inner) {
  final below = inner.length <= outer.length ? '' : inner.substring(outer.endsWith('/') ? outer.length : outer.length + 1);
  return below.split('/').where((part) => part.isNotEmpty).any(protectedSegment);
}

/// Folders no path rule may reach: the person's home folder and Canoa's own configuration folder.
final class PermissionGuards {
  const PermissionGuards({this.home, this.config});
  final String? home, config;
}

/// Resolves the links in a path, for what exists of it; null when it cannot. A desktop build resolves them; a browser, and a
/// session on another machine, cannot, and no path rule is made there.
typedef RealPath = String? Function(String path);

String? _resolved(RealPath real, String cleaned) {
  final resolved = real(cleaned);
  return resolved == null ? null : cleanPath(resolved);
}

/// Why no path rule can be made in [folder], or null when one can: the folder holds the person's home folder (or is it), or
/// overlaps Canoa's configuration folder, whether read as written or after its links are resolved; or links cannot be followed.
String? folderProblem(String folder, {PermissionGuards guards = const PermissionGuards(), RealPath? real}) {
  if (real == null) return 'Changes to files are only remembered for sessions on this computer, where links can be followed.';
  final d = cleanPath(folder);
  if (d == null) return 'The folder has to be an absolute path.';
  final rd = _resolved(real, d);
  if (rd == null) return 'A link in the folder could not be followed.';
  for (final guard in [guards.home, guards.config]) {
    final g = guard == null ? null : cleanPath(guard);
    if (g == null) continue;
    final rg = _resolved(real, g);
    final isHome = guard == guards.home;
    for (final candidate in [g, ?rg]) {
      if (_under(d, candidate) || _under(rd, candidate)) return isHome ? 'The folder holds your home folder.' : 'The folder holds Canoa\'s own settings.';
      if (!isHome && (_under(candidate, d) || _under(candidate, rd))) return 'The folder is inside Canoa\'s own settings.';
    }
  }
  return null;
}

/// Why [file] cannot be covered by a rule for [scope] in [folder], or null when it can. All three have to be absolute, and links
/// must be followable ([real]); the folder is not home and not Canoa's settings; the file lies under the scope and the scope under
/// the folder; and nothing below the folder is a protected segment; all of it after the links are resolved too.
String? pathProblem({required String scope, required String file, required String folder, PermissionGuards guards = const PermissionGuards(), RealPath? real}) {
  final bad = folderProblem(folder, guards: guards, real: real);
  if (bad != null) return bad;
  final s = cleanPath(scope), f = cleanPath(file), d = cleanPath(folder);
  if (s == null || f == null || d == null) return 'A path has to be absolute, with no ~, no : but a drive letter\'s, and not a network path.';
  if (!_under(d, s)) return 'That is not inside $folder.';
  if (!_under(s, f)) return 'That does not cover the file asked.';
  if (_touchesProtected(d, s) || _touchesProtected(d, f)) return 'Hidden files and folders, version control, settings, build scripts and agent instructions are never covered.';
  final rs = _resolved(real!, s), rf = _resolved(real, f), rd = _resolved(real, d);
  if (rs == null || rf == null || rd == null) return 'A link in that path could not be followed.';
  if (!_under(rd, rs) || !_under(rs, rf) || _touchesProtected(rd, rf) || _touchesProtected(rd, rs)) return 'A link in that path leads outside it, or to something never covered.';
  return null;
}

/// Characters an exact rule never holds: controls, line breaks, and what hides or reorders text.
final RegExp _hiddenInExact = RegExp(r'[\u0000-\u001f\u007f-\u009f\u00ad\u061c\u200b-\u200f\u2028-\u202e\u2060-\u2069\ufeff]');

/// How a rule says what it allows.
enum RuleType {
  /// This command, exactly.
  command,

  /// A change to this file, or any under this folder.
  path,

  /// This MCP tool, with any input.
  mcp,

  /// A rule an earlier version wrote, or another program: kept so it can be seen and revoked, and never applied.
  unsupported;

  static RuleType? named(Object? name) => values.where((type) => type.name == name).firstOrNull;

  /// Which kind of power it grants. A command can write any file, and a file change or an MCP tool (a file system tool among
  /// them) can write what a command then runs, so one source and folder keep commands or the rest, never both.
  bool get isCommand => this == command;
  bool get isFilesystem => this == path || this == mcp;
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
    String? typeName,
    required this.value,
    required this.made,
    this.fired = 0,
    this.lastUsed,
    this.external = false,
  }) : typeName = typeName ?? type.name;

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

  /// The type as stored, which for an [RuleType.unsupported] rule is what an earlier version wrote.
  final String typeName;

  /// The command, the path, or the MCP tool's name, by [type].
  final String value;
  final DateTime made;
  int fired;
  DateTime? lastUsed;

  /// Whether the stored rule was not written by this app: someone edited the file, or another program did.
  final bool external;

  /// What it allows, in words a person reads.
  String get describe => switch (type) {
    RuleType.command => 'Run: $value',
    RuleType.path => 'Change $value, or a file under it',
    RuleType.mcp => switch (_mcpName.firstMatch(value)) {
      final match? => 'Use the tool ${match.group(2)} of ${match.group(1)}, with any input',
      null => 'Use the tool $value, with any input',
    },
    RuleType.unsupported => 'No longer supported ($typeName): $value',
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
    'type': typeName,
    'value': value,
    'made': made.toUtc().toIso8601String(),
    'fired': fired,
    if (lastUsed != null) 'last_used': lastUsed!.toUtc().toIso8601String(),
    if (!external) 'by': 'canoa',
  };

  static PermissionRule? fromJson(Object? json) {
    if (json is! Map || json['type'] is! String) return null;
    final named = RuleType.named(json['type']);
    final type = named == null || named == RuleType.unsupported ? RuleType.unsupported : named;
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
      typeName: json['type'] as String,
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
    case (RuleType.path, PathSubject(:final path)):
      return pathProblem(scope: value, file: path, folder: folder, guards: guards, real: real);
    case (RuleType.mcp, McpSubject(:final name)):
      return name == value ? null : 'That is not the tool asked.';
    default:
      return 'A rule of that kind cannot cover this request.';
  }
}

/// An append-only record, one line a time.
abstract interface class RuleLog {
  Future<void> append(String line);

  /// Where it is, for a person to find.
  String get location;
}

/// A log kept in memory, for tests.
final class MemoryRuleLog implements RuleLog {
  final lines = <String>[];

  @override
  Future<void> append(String line) async => lines.add(line);

  @override
  String get location => 'memory only';
}

/// What a log line says of [detail] instead of the text: a command is its program and how many arguments follow, which cannot
/// hold a secret; a path or a tool is itself. [sha256Of] stands for the rest, so a line can be matched to a command and not read.
String redacted(RuleType type, String detail) {
  if (type != RuleType.command) return detail;
  final words = detail.trim().split(RegExp(r'\s+'));
  final program = words.first.length > 64 ? '${words.first.substring(0, 64)}…' : words.first;
  return words.length == 1 ? program : '$program … (${words.length - 1} arguments)';
}

String sha256Of(String text) => sha256.convert(utf8.encode(text)).toString();

/// The rules the person made, kept in a file beside canoa.cfg (IndexedDB in a browser), and a log of each rule made, used and
/// revoked. A change is written at once. The stored file is read again before each write and before a request is matched, so a
/// rule another program added is kept (and flagged) and a rule another instance revoked stops applying.
class PermissionRules extends ChangeNotifier {
  PermissionRules(this.backend, {this.log, this.guards = const PermissionGuards(), this.real, DateTime Function()? clock, Random? random})
      : _clock = clock ?? DateTime.now,
        _random = random ?? Random.secure();

  final LedgerBackend backend;

  /// Where the log is kept; null keeps none.
  final RuleLog? log;
  final PermissionGuards guards;

  /// How links are followed; null where they cannot be, and no path rule is then made.
  final RealPath? real;
  final DateTime Function() _clock;
  final Random _random;
  final List<PermissionRule> _rules = [];
  final Set<String> _revoked = {};

  /// The ids the stored file held when it was last read or written: one of them that the file no longer holds was revoked
  /// elsewhere.
  Set<String> _persisted = {};
  Future<void> _writing = Future.value();

  /// The most rules kept; a person who has made this many revokes some before making another.
  static const maxRules = 500;

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
      _persisted = {for (final rule in _rules) rule.id};
    } on FormatException catch (error) {
      problem = '${backend.location} is not a list of rules (${error.message}), so none apply';
    }
    notifyListeners();
  }

  /// Whether a path rule can be made in [folder], by [folderProblem].
  bool pathsAllowedIn(String folder) => folderProblem(folder, guards: guards, real: real) == null;

  /// Why a rule of [type] cannot be made for this source and folder beside the rules there, or null. Commands and the rest are
  /// not kept together for one source, in this folder or one that holds it or lies in it.
  String? conflict(String source, String peer, String folder, RuleType type) {
    if (type == RuleType.unsupported) return null;
    final here = cleanPath(folder);
    for (final rule in _rules) {
      if (rule.source != source || rule.peer != peer || rule.type == RuleType.unsupported) continue;
      if ((type.isCommand && rule.type.isFilesystem) || (type.isFilesystem && rule.type.isCommand)) {
        final there = cleanPath(rule.folder);
        if (here == null || there == null || _under(here, there) || _under(there, here)) {
          return type.isCommand
              ? 'A rule for file changes or a tool already stands here or in a folder next to it: revoke it before allowing commands.'
              : 'A rule for commands already stands here or in a folder next to it: revoke it before allowing this.';
        }
      }
    }
    return null;
  }

  /// Why [value] cannot be a rule of [type] for [subject] in [folder], by [scopeProblem] with this list's guards.
  String? checkScope(RuleType type, String value, RuleSubject subject, String folder) =>
      scopeProblem(type, value, subject, folder, guards: guards, real: real);

  /// The first rule that allows a request, or null. Where the list holds commands and the rest for one source in nested folders
  /// (a hand-edited file), neither applies.
  PermissionRule? match({required String source, String peer = '', required String folder, required String? kind, required RuleSubject subject}) {
    for (final rule in _rules) {
      if (!rule.matches(source: source, peer: peer, folder: folder, kind: kind, subject: subject, guards: guards, real: real)) continue;
      if (conflict(rule.source, rule.peer, rule.folder, rule.type) != null) continue;
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

  /// Takes up what the stored file says: a rule it no longer holds that it held before was revoked elsewhere and goes; a rule it
  /// holds that this list does not know and did not revoke is added, flagged where this app did not write it.
  Future<void> _mergeStored() async {
    final document = await backend.read();
    final List<PermissionRule> stored;
    try {
      stored = document == null ? const [] : _parse(document);
    } on FormatException {
      return;
    }
    final ids = {for (final rule in stored) rule.id};
    final before = _rules.length;
    _rules.removeWhere((rule) => _persisted.contains(rule.id) && !ids.contains(rule.id));
    final known = {for (final rule in _rules) rule.id};
    var changed = _rules.length != before;
    for (final rule in stored) {
      if (known.contains(rule.id) || _revoked.contains(rule.id) || _rules.length >= maxRules) continue;
      _rules.add(rule);
      changed = true;
    }
    _persisted = ids;
    if (changed) notifyListeners();
  }

  /// Reads the stored file again and takes up what it says, in order with the writes: call it before matching a request.
  Future<void> refresh() => _writing = _writing.then((_) async {
    try {
      await _mergeStored();
    } on Object catch (error) {
      debugPrint('Canoa permissions: could not read ${backend.location}: $error');
    }
  });

  Future<void> flush() {
    return _writing = _writing.then((_) async {
      try {
        await _mergeStored();
        await backend.write(encode());
        _persisted = {for (final rule in _rules) rule.id};
        if (writeProblem?.startsWith('could not write the log') != true) writeProblem = null;
      } on Object catch (error) {
        writeProblem = 'could not write ${backend.location}: $error';
        debugPrint('Canoa permissions: $writeProblem');
      }
      notifyListeners();
    });
  }

  Future<void> _logging = Future.value();

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
      'what': redacted(rule.type, detail),
      'sha256': sha256Of(detail),
    });
    _logging = _logging.then((_) => target.append(line)).catchError((Object error) {
      writeProblem = 'could not write the log ${target.location}: $error';
      debugPrint('Canoa permissions: $writeProblem');
      notifyListeners();
    });
  }

  /// Completes when every change and log line so far is written.
  Future<void> settled() async {
    await _logging;
    await flush();
  }
}
