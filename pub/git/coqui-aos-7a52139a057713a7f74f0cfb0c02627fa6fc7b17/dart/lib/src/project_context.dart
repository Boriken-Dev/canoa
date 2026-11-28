/// A project's context files: `AGENTS.md` and its like in each directory from the project root to
/// the working directory, and in each directory's `.agents` one, in the order a later file
/// overrides an earlier one.
library;

import 'fs.dart';
import 'text.dart';

const List<String> _defaultDirectories = ['.agents'];
const List<String> _defaultPatterns = ['AGENTS.md', 'AGENTS.*.md'];

/// What [contextFiles] finds.
class ContextFiles {
  /// Creates the answer.
  const ContextFiles({
    required this.files,
    required this.skipped,
    required this.ignored,
    required this.levels,
  });

  /// The files to feed, in order: a later file overrides an earlier one.
  final List<String> files;

  /// The files of the walk that the harness loads itself, in the same order.
  final List<String> skipped;

  /// The entries of `AGENTS_DIR` and `AGENTS_CONTEXT` that are not valid, as written.
  final List<String> ignored;

  /// The levels this answer covers: the walk without the levels already delivered.
  final List<String> levels;
}

/// The project's context files for the working directory [cwd], read through [fs].
///
/// [root] is the project root, null when the session has none. [agentsDir] and [agentsContext] are
/// the values of `AGENTS_DIR` and `AGENTS_CONTEXT`, null when unset, and [separator] is the
/// separator of their lists (`;` on Windows, `:` elsewhere). [native] holds the paths, relative to
/// a level's directory and written with `/`, that the harness loads itself, and [delivered] the
/// levels a session has already received. Paths are absolute and written with `/`.
///
/// The levels are the root and each directory below it down to [cwd]; a working directory outside
/// the root, or no root, is one level. A level's files come as the specification's *Project context
/// files* section orders them: the files that are not local, place by place (the directory, then
/// each `AGENTS_DIR` entry), then the local files. A link to a file is a context file and a link
/// to a directory is not.
Future<ContextFiles> contextFiles(
  FileSystem fs, {
  String? root,
  required String cwd,
  String? agentsDir,
  String? agentsContext,
  String separator = ':',
  List<String> native = const [],
  List<String> delivered = const [],
}) async {
  final (directories, badDirectories) = _entries(
    agentsDir,
    separator,
    _defaultDirectories,
    true,
  );
  final (patterns, badPatterns) = _entries(
    agentsContext,
    separator,
    _defaultPatterns,
    false,
  );
  final done = delivered.map(_trimmed).toSet();
  final levels = [
    for (final level in _walk(root, cwd, directories))
      if (!done.contains(level)) level,
  ];
  final files = <String>[];
  final skipped = <String>[];
  for (final level in levels) {
    for (final local in [false, true]) {
      for (final directory in [null, ...directories]) {
        final place = directory == null ? level : '$level/$directory';
        for (final name in await _namesIn(fs, place, patterns, local)) {
          final path = '$place/$name';
          if (!await _isContextFile(fs, path)) continue;
          final relative = directory == null ? name : '$directory/$name';
          (native.contains(relative) ? skipped : files).add(path);
        }
      }
    }
  }
  return ContextFiles(
    files: files,
    skipped: skipped,
    ignored: [...badDirectories, ...badPatterns],
    levels: levels,
  );
}

String _trimmed(String path) {
  var end = path.length;
  while (end > 1 && (path[end - 1] == '/' || path[end - 1] == r'\')) {
    end -= 1;
  }
  return path.substring(0, end);
}

/// The valid entries of a list variable, once each, and the entries that are not valid.
(List<String>, List<String>) _entries(
  String? value,
  String separator,
  List<String> fallback,
  bool directory,
) {
  final kept = <String>[];
  final ignored = <String>[];
  for (final entry in (value ?? '').split(separator)) {
    if (entry.isEmpty) continue;
    if (entry.contains('/') ||
        entry.contains(r'\') ||
        (directory && (entry == '.' || entry == '..'))) {
      ignored.add(entry);
    } else if (!kept.contains(entry)) {
      kept.add(entry);
    }
  }
  return (kept.isEmpty ? List.of(fallback) : kept, ignored);
}

/// The levels of the walk, broadest first.
List<String> _walk(String? root, String cwd, List<String> directories) {
  final here = _trimmed(cwd);
  final top = root == null ? null : _trimmed(root);
  if (top == null || !(here == top || here.startsWith('$top/'))) {
    return [here];
  }
  final levels = [top];
  var at = top;
  for (final part in here.substring(top.length).split('/')) {
    if (part.isEmpty) continue;
    at = '$at/$part';
    if (!directories.contains(part)) levels.add(at);
  }
  return levels;
}

/// The names in [place] that a pattern matches and whose locality is [local]: pattern by
/// pattern, each in byte order, none twice.
Future<List<String>> _namesIn(
  FileSystem fs,
  String place,
  List<String> patterns,
  bool local,
) async {
  final List<Object> listed;
  try {
    listed = await fs.list(place);
  } on Object {
    return [];
  }
  final names = [
    for (final name in listed)
      if (name is String && _isLocal(name) == local) name,
  ]..sort(_byBytes);
  final found = <String>[];
  for (final pattern in patterns) {
    for (final name in names) {
      if (_matches(pattern, name) && !found.contains(name)) found.add(name);
    }
  }
  return found;
}

int _byBytes(String a, String b) {
  final left = encodeUtf8(a);
  final right = encodeUtf8(b);
  for (var at = 0; at < left.length && at < right.length; at += 1) {
    if (left[at] != right[at]) return left[at] - right[at];
  }
  return left.length - right.length;
}

Future<bool> _isContextFile(FileSystem fs, String path) async {
  try {
    final kind = (await fs.metadata(path)).kind;
    if (kind == FileKind.file) return true;
    if (kind != FileKind.symlink) return false;
    try {
      await fs.list(path);
      return false;
    } on Object {
      return true;
    }
  } on Object {
    return false;
  }
}

/// Whether a name is `<stem>.local.<extension>`.
bool _isLocal(String name) {
  final dot = name.lastIndexOf('.');
  if (dot < 0 || dot == name.length - 1) return false;
  final rest = name.substring(0, dot);
  return rest.length > '.local'.length && rest.endsWith('.local');
}

/// A name pattern: `*` is any run of characters, none included, and the rest is literal.
bool _matches(String pattern, String name) {
  final parts = pattern.split('*');
  if (!name.startsWith(parts.first)) return false;
  var rest = name.substring(parts.first.length);
  if (parts.length == 1) return rest.isEmpty;
  for (final part in parts.sublist(1, parts.length - 1)) {
    final at = rest.indexOf(part);
    if (at < 0) return false;
    rest = rest.substring(at + part.length);
  }
  return rest.endsWith(parts.last);
}
