import 'dart:math';

/// canoa.cfg's `[sessions] workspace` when it is absent.
const defaultWorkspace = r'${documents}/Canoa/scratch/${session}';

/// A session id to show what a template makes, in Settings, before any session has one.
const exampleSessionId = '3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40';

/// The variables a workspace template may use. `documents` and `home` are folders and bring their own separators; every
/// other value is made safe as one path segment.
const workspaceVariables = {
  'documents', 'home', 'session', 'date', 'time', 'datetime', 'handler', 'source', 'host', 'fqdn',
};

const _folderVariables = {'documents', 'home'};
final _variable = RegExp(r'\$\{([^}]*)\}');

/// Why [template] cannot be a workspace, in a person's words, or null when it can: every `${name}` is a known variable and
/// `${session}` is in it, so two sessions never share a folder by accident.
String? workspaceProblem(String template) {
  if (template.trim().isEmpty) return 'it is empty';
  for (final match in _variable.allMatches(template)) {
    final name = match.group(1)!;
    if (!workspaceVariables.contains(name)) {
      return 'unknown variable \${$name} (use ${workspaceVariables.map((name) => '\${$name}').join(', ')})';
    }
  }
  if (template.replaceAll(_variable, '').contains(r'${')) return r'a ${ is never closed';
  if (!template.contains(r'${session}')) return r'it has no ${session}, so two sessions could share a folder';
  return null;
}

final _unsafe = RegExp(r'[\\/:*?"<>|\x00-\x1f\x7f]');
final _reservedNames = RegExp(r'^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$', caseSensitive: false);

/// [value] as one path segment: separators, `..`, control and reserved characters are replaced with `_`, and a trailing dot
/// or space (which Windows drops) is trimmed. Empty stays empty.
String safeSegment(String value, {int limit = 100}) {
  var text = value.replaceAll(_unsafe, '_').replaceAll('..', '_').trim();
  text = text.replaceFirst(RegExp(r'[. ]+$'), '');
  if (text.length > limit) text = text.substring(0, limit).replaceFirst(RegExp(r'[. ]+$'), '');
  if (text.isEmpty) return '';
  if (text.startsWith('.')) text = '_${text.substring(1)}';
  if (_reservedNames.hasMatch(text)) text = '_$text';
  return text;
}

/// What a template's variables stand for. A null or empty value leaves its variable empty.
final class WorkspaceValues {
  const WorkspaceValues({
    this.documents = '',
    this.home = '',
    required this.session,
    required this.now,
    this.handler = '',
    this.source = '',
    this.host = '',
    this.fqdn = '',
  });

  /// The person's Documents and home folders, as the operating system names them.
  final String documents, home;

  /// Canoa's own id for the session.
  final String session;

  /// The local time the folder is made at.
  final DateTime now;
  final String handler, source, host, fqdn;

  String? _value(String name) {
    String two(int n) => n.toString().padLeft(2, '0');
    final date = '${now.year.toString().padLeft(4, '0')}-${two(now.month)}-${two(now.day)}';
    final time = '${two(now.hour)}${two(now.minute)}${two(now.second)}';
    return switch (name) {
      'documents' => documents,
      'home' => home,
      'session' => session,
      'date' => date,
      'time' => time,
      'datetime' => '${date}T$time',
      'handler' => handler,
      'source' => source,
      'host' => host,
      'fqdn' => fqdn,
      _ => null,
    };
  }
}

/// [template] with its variables put in, separated by [separator]. Only the template places separators: a value other than
/// a folder is one segment, and a part of the template left empty by its variables is dropped. [template] must pass
/// [workspaceProblem].
String expandWorkspace(String template, WorkspaceValues values, {String separator = '/'}) {
  final problem = workspaceProblem(template);
  if (problem != null) throw FormatException(problem);
  final parts = template.split(RegExp(r'[\\/]'));
  final out = <String>[];
  for (var index = 0; index < parts.length; index++) {
    final part = parts[index];
    var hadVariable = false;
    final text = part.replaceAllMapped(_variable, (match) {
      hadVariable = true;
      final name = match.group(1)!;
      final value = values._value(name) ?? '';
      return _folderVariables.contains(name) ? value : safeSegment(value);
    });
    // A leading empty part is the root of an absolute template.
    if (text.isEmpty && (hadVariable || index > 0)) continue;
    out.add(text);
  }
  final joined = out.join(separator);
  // A folder value may end in a separator (a drive's root); a network path keeps its two leading ones.
  final lead = RegExp('^${RegExp.escape(separator)}{1,2}').stringMatch(joined) ?? '';
  return lead + joined.substring(lead.length).replaceAll(RegExp('${RegExp.escape(separator)}{2,}'), separator);
}

/// A new session id: a random (version 4) UUID, in lower case.
String newSessionUuid([Random? random]) {
  final draw = random ?? Random.secure();
  final bytes = List<int>.generate(16, (_) => draw.nextInt(256));
  bytes[6] = bytes[6] & 0x0f | 0x40;
  bytes[8] = bytes[8] & 0x3f | 0x80;
  final hex = bytes.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
  return '${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}';
}
