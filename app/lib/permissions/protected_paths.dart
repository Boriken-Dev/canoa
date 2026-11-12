/// The paths a remembered rule never covers: one list in gitignore syntax, the person's to edit.
///
/// A line is a pattern; a leading `!` allows what an earlier line protected, and the last line that matches a path wins. A pattern
/// that matches a directory covers everything under it. `*` and `?` stand for part of a name, `**` for any number of names, `#`
/// starts a comment. Where a pattern starts says what it is relative to: `~/` the home folder, a drive (`C:/x`) or `/` the file
/// system, anything else any place (`.env` matches every file of that name, `a/b` every `b` under an `a`). Names are compared
/// whatever their case. The syntax is the one of coqui-acp-pi's `protected` setting.
library;

/// What Canoa recommends protecting, for a person who has not written a list of their own: the files that run code when a build,
/// a test, an editor or an agent starts, and the files agents read their instructions from. Written out in a list to keep,
/// change or remove any of it; a list the person writes replaces it.
const List<String> recommendedProtected = [
  '# Every file and folder whose name starts with a dot: version control, editor and agent settings, secrets.',
  '.*',
  '# Files that run code when a build, a test or an editor starts.',
  'package.json', 'makefile', 'justfile', 'pyproject.toml', 'setup.py', 'build.rs', 'rakefile', 'cargo.toml', 'conftest.py',
  'directory.build.props', 'directory.build.targets', 'directory.packages.props', 'mise.toml', 'pytest.ini', 'tox.ini',
  'setup.cfg', 'noxfile.py', 'shell.nix', 'flake.nix', 'gradlew', 'mvnw', 'gradle-wrapper.properties', 'build.gradle*',
  'pom.xml', 'go.mod', 'go.work', 'nuget.config', 'cmakelists.txt', 'meson.build', 'gemfile', 'dockerfile',
  'rust-toolchain*', 'requirements*.txt', 'gulpfile.*', 'gruntfile.*', '*.config.*', '*.pth', '*.csproj', '*.code-workspace',
  '# Installed dependencies and environments.',
  'node_modules', 'venv', 'env', '__pycache__', 'vendor',
  '# The files agents read their instructions from.',
  'agents.md', 'agents.override.md', 'claude.md', 'claude.local.md', 'gemini.md',
];

enum _Anchor { home, root, anywhere }

final class _Line {
  const _Line(this.allow, this.anchor, this.names);
  final bool allow;
  final _Anchor anchor;
  final List<String> names;
}

/// Whether one path name is spelled in a way no list can judge, or what the file system reads it as. A name is first read the way a
/// file system reads it: Windows drops trailing dots and spaces, so `.claude.` and `package.json.` are the files they look like.
/// Returns the lower-case name that lists are matched against, or null when the name is refused: anything but printable ASCII
/// (Unicode normalisation and ignorable characters give one name many spellings), a name that is nothing once its dots and spaces
/// are gone, an 8.3 short name (`GIT~1`, another spelling of a long one) and a Windows device (`CON`, `con.txt`).
String? readName(String raw) {
  if (raw.codeUnits.any((unit) => unit < 0x20 || unit > 0x7e)) return null;
  var read = raw;
  while (read.endsWith('.') || read.endsWith(' ')) {
    read = read.substring(0, read.length - 1);
  }
  if (read.isEmpty) return null;
  final lower = read.toLowerCase();
  if (_shortName.hasMatch(lower) || _deviceName.hasMatch(lower)) return null;
  return lower;
}

final RegExp _shortName = RegExp(r'~[0-9]');
final RegExp _deviceName = RegExp(r'^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\..*)?$');

/// A list of patterns, ready to match paths against.
final class ProtectedPaths {
  /// The list [lines] write; a blank line and a `#` line are nothing.
  ProtectedPaths(Iterable<String> lines) : lines = List.unmodifiable(lines), _rules = _parse(lines);

  /// The recommended list, [recommendedProtected].
  static final ProtectedPaths recommended = ProtectedPaths(recommendedProtected);

  /// The lines as written.
  final List<String> lines;
  final List<_Line> _rules;

  static List<_Line> _parse(Iterable<String> lines) {
    final rules = <_Line>[];
    for (final line in lines) {
      var text = line.trim().replaceAll('\\', '/');
      if (text.isEmpty || text.startsWith('#')) continue;
      final allow = text.startsWith('!');
      if (allow) text = text.substring(1);
      _Anchor anchor;
      String rest;
      if (text.startsWith('~/')) {
        anchor = _Anchor.home;
        rest = text.substring(2);
      } else if (text.startsWith('/')) {
        anchor = _Anchor.root;
        rest = text.replaceFirst(RegExp(r'^/+'), '');
      } else if (text.length > 1 && text[1] == ':' && RegExp(r'[A-Za-z]').hasMatch(text[0])) {
        anchor = _Anchor.root;
        rest = text;
      } else {
        anchor = _Anchor.anywhere;
        rest = text;
      }
      final names = [
        for (final name in rest.split('/'))
          if (name.isNotEmpty && name != '.') name.toLowerCase(),
      ];
      if (names.isNotEmpty) rules.add(_Line(allow, anchor, names));
    }
    return rules;
  }

  /// Whether the list is exactly the recommended one, line for line.
  bool get isRecommended => lines.length == recommendedProtected.length && [for (var i = 0; i < lines.length; i++) lines[i] == recommendedProtected[i]].every((same) => same);

  /// Whether the last line that matches the path made of [names] (lower case, drive first, as [readName] gives them) protects it.
  /// [home] is the names of the folder `~/` means. A pattern with no anchor counts only where it ends below the first [from] names
  /// (the session's folder): a dot above the folder a person works in is theirs. An anchored pattern counts anywhere, since a
  /// folder inside a protected place is protected.
  bool protects(List<String> names, {List<String>? home, int from = 0}) {
    var protected = false;
    for (final rule in _rules) {
      var hit = false;
      for (var end = 1; end <= names.length && !hit; end++) {
        switch (rule.anchor) {
          case _Anchor.anywhere:
            if (end > from) {
              for (var start = 0; start < end && !hit; start++) {
                hit = _fits(rule.names, 0, names, start, end);
              }
            }
          case _Anchor.root:
            hit = _fits(rule.names, 0, names, 0, end);
          case _Anchor.home:
            if (home != null && end > home.length) {
              var inside = true;
              for (var i = 0; i < home.length && inside; i++) {
                inside = names[i] == home[i];
              }
              hit = inside && _fits(rule.names, 0, names, home.length, end);
            }
        }
      }
      if (hit) protected = !rule.allow;
    }
    return protected;
  }
}

bool _glob(String pattern, int p, String name, int n) {
  if (p == pattern.length) return n == name.length;
  final c = pattern[p];
  if (c == '*') {
    for (var skip = n; skip <= name.length; skip++) {
      if (_glob(pattern, p + 1, name, skip)) return true;
    }
    return false;
  }
  if (n == name.length) return false;
  if (c == '?' || c == name[n]) return _glob(pattern, p + 1, name, n + 1);
  return false;
}

/// Whether pattern[at..] is the whole of names[from..end].
bool _fits(List<String> pattern, int at, List<String> names, int from, int end) {
  if (at == pattern.length) return from == end;
  if (pattern[at] == '**') {
    for (var skip = from; skip <= end; skip++) {
      if (_fits(pattern, at + 1, names, skip, end)) return true;
    }
    return false;
  }
  return from < end && _glob(pattern[at], 0, names[from], 0) && _fits(pattern, at + 1, names, from + 1, end);
}

/// A `protected` value of canoa.cfg as lines: one string (a line a line) or a list of strings; null when it is neither.
List<String>? protectedLinesOf(Object? value) {
  if (value is String) return value.split(RegExp(r'\r?\n'));
  if (value is List && value.every((line) => line is String)) return [for (final line in value) (line as String)];
  return null;
}
