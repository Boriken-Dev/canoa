// A small, dependency-free syntax highlighter: a line-at-a-time lexer driven by a table of languages. It sorts the text of a line into
// comments, strings, numbers, keywords, literals and types, and carries what a line leaves open (a block comment, a string that runs
// on) to the next one, so a caller that draws lines as they scroll into view lexes only those, from a checkpoint.

/// What a stretch of text is.
enum Tok { plain, keyword, type, literal, string, number, comment, meta, inserted, deleted }

/// [start, end) of a line, and what it is.
final class Span {
  const Span(this.start, this.end, this.tok);

  final int start, end;
  final Tok tok;

  @override
  String toString() => '$tok[$start,$end)';
}

/// How a string opens and closes.
final class StringRule {
  const StringRule(this.open, {String? close, this.escape = true, this.multiline = false}) : close = close ?? open;

  final String open, close;

  /// Whether a backslash makes the next character part of the string.
  final bool escape;

  /// Whether the string runs on over line breaks (a triple quote, a template literal).
  final bool multiline;
}

/// Where an identifier followed by a colon or an equals sign is a key (`key: value`, `key = value`).
enum KeyStyle { none, colon, equals }

/// One language: its words and its comment and string forms.
final class Language {
  const Language(
    this.id, {
    this.names = const [],
    this.extensions = const [],
    this.lineComments = const [],
    this.blockComment,
    this.strings = const [StringRule('"'), StringRule("'")],
    this.keywords = const {},
    this.literals = const {},
    this.types = const {},
    this.capitalIsType = false,
    this.metaStart,
    this.ignoreCase = false,
    this.markup = false,
    this.jsonKeys = false,
    this.keys = KeyStyle.none,
    this.diff = false,
  });

  /// The name the table is keyed by, lower case.
  final String id;

  /// Other names a fence's info string may use.
  final List<String> names;

  /// File extensions, lower case, without the dot.
  final List<String> extensions;
  final List<String> lineComments;
  final (String, String)? blockComment;
  final List<StringRule> strings;
  final Set<String> keywords, literals, types;

  /// Whether a name that starts with a capital letter is a type.
  final bool capitalIsType;

  /// A character that starts a name that is meta (`@override`, `#include`, `$var`).
  final String? metaStart;
  final bool ignoreCase;

  /// HTML and XML: a name right after `<` or `</` is a keyword.
  final bool markup;

  /// A string followed by a colon is a key.
  final bool jsonKeys;
  final KeyStyle keys;

  /// A unified diff: a line is read by its first character.
  final bool diff;

  bool _hasWord(Set<String> set, String word) => set.contains(ignoreCase ? word.toLowerCase() : word);
}

Set<String> _w(String words) => words.split(RegExp(r'\s+')).where((w) => w.isNotEmpty).toSet();

const _cStrings = [StringRule('"'), StringRule("'")];
const _tripleStrings = [StringRule('"""', multiline: true), StringRule("'''", multiline: true), StringRule('"'), StringRule("'")];

/// The languages the highlighter knows, by id.
final List<Language> languages = [
  Language(
    'dart',
    extensions: ['dart'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"""', multiline: true), StringRule("'''", multiline: true), StringRule('"'), StringRule("'")],
    keywords: _w('abstract as assert async await base break case catch class const continue covariant default deferred do dynamic else enum export extends extension external factory final finally for get hide if implements import in interface is late library mixin new on operator part required rethrow return sealed set show static super switch sync this throw try typedef var void when while with yield'),
    literals: _w('true false null'),
    types: _w('int double num String bool List Map Set Object Future Stream Iterable Function Never Type Record'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'rust',
    names: ['rs'],
    extensions: ['rs'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"'), StringRule("'")],
    keywords: _w('as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self static struct super trait type unsafe use where while union'),
    literals: _w('true false None Some Ok Err'),
    types: _w('i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str String Vec Option Result Box Self'),
    capitalIsType: true,
    metaStart: '#',
  ),
  Language(
    'javascript',
    names: ['js', 'jsx', 'node', 'mjs', 'cjs'],
    extensions: ['js', 'jsx', 'mjs', 'cjs'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"'), StringRule("'"), StringRule('`', multiline: true)],
    keywords: _w('async await break case catch class const continue debugger default delete do else export extends finally for from function if import in instanceof let new of return static super switch this throw try typeof var void while with yield'),
    literals: _w('true false null undefined NaN Infinity'),
    types: _w('Array Object String Number Boolean Promise Map Set Date Error'),
    capitalIsType: true,
  ),
  Language(
    'typescript',
    names: ['ts', 'tsx'],
    extensions: ['ts', 'tsx', 'mts', 'cts'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"'), StringRule("'"), StringRule('`', multiline: true)],
    keywords: _w('abstract as async await break case catch class const continue declare default delete do else enum export extends finally for from function if implements import in instanceof interface keyof let namespace new of private protected public readonly return static super switch this throw try type typeof var void while with yield'),
    literals: _w('true false null undefined NaN Infinity'),
    types: _w('any string number boolean unknown never object symbol bigint Array Promise Record Partial Map Set'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'python',
    names: ['py', 'python3', 'py3'],
    extensions: ['py', 'pyi', 'pyw'],
    lineComments: ['#'],
    strings: _tripleStrings,
    keywords: _w('and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case'),
    literals: _w('True False None'),
    types: _w('int float str bool list dict set tuple bytes object type self cls'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'go',
    names: ['golang'],
    extensions: ['go'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"'), StringRule("'"), StringRule('`', escape: false, multiline: true)],
    keywords: _w('break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var'),
    literals: _w('true false nil iota'),
    types: _w('int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 string bool byte rune error any'),
    capitalIsType: true,
  ),
  Language(
    'c',
    names: ['h'],
    extensions: ['c', 'h'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: _cStrings,
    keywords: _w('auto break case const continue default do else enum extern for goto if inline register restrict return sizeof static struct switch typedef union volatile while'),
    literals: _w('true false NULL nullptr'),
    types: _w('int char short long float double void unsigned signed size_t bool uint8_t uint16_t uint32_t uint64_t int8_t int16_t int32_t int64_t'),
    metaStart: '#',
  ),
  Language(
    'cpp',
    names: ['c++', 'cc', 'cxx', 'hpp'],
    extensions: ['cpp', 'cc', 'cxx', 'hpp', 'hh', 'hxx'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: _cStrings,
    keywords: _w('alignas auto break case catch class const constexpr continue default delete do else enum explicit export extern for friend goto if inline namespace new noexcept operator override private protected public return sizeof static struct switch template this throw try typedef typename union using virtual volatile while'),
    literals: _w('true false NULL nullptr'),
    types: _w('int char short long float double void unsigned signed size_t bool string vector map set'),
    metaStart: '#',
  ),
  Language(
    'java',
    extensions: ['java'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"""', multiline: true), StringRule('"'), StringRule("'")],
    keywords: _w('abstract assert break case catch class const continue default do else enum extends final finally for goto if implements import instanceof interface native new package private protected public return static strictfp super switch synchronized this throw throws transient try void volatile while record var'),
    literals: _w('true false null'),
    types: _w('int long short byte char float double boolean String Object'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'kotlin',
    names: ['kt'],
    extensions: ['kt', 'kts'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"""', multiline: true), StringRule('"'), StringRule("'")],
    keywords: _w('as break class continue do else for fun if in interface is object override package return super this throw try typealias val var when while by catch constructor finally import init open private protected public data sealed suspend'),
    literals: _w('true false null'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'swift',
    extensions: ['swift'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"""', multiline: true), StringRule('"')],
    keywords: _w('actor associatedtype async await break case catch class continue default defer do else enum extension fallthrough for func guard if import in init inout let private protocol public return self static struct subscript super switch throw throws try typealias var where while'),
    literals: _w('true false nil'),
    capitalIsType: true,
    metaStart: '@',
  ),
  Language(
    'csharp',
    names: ['cs', 'c#'],
    extensions: ['cs'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"'), StringRule("'")],
    keywords: _w('abstract as async await base break case catch class const continue default delegate do else enum event explicit extern finally fixed for foreach if implicit in interface internal is lock namespace new operator out override params private protected public readonly ref return sealed static struct switch this throw try typeof unsafe using var virtual void volatile while yield'),
    literals: _w('true false null'),
    types: _w('int long short byte char float double decimal bool string object'),
    capitalIsType: true,
  ),
  Language(
    'shell',
    names: ['sh', 'bash', 'zsh', 'console', 'shellscript', 'fish'],
    extensions: ['sh', 'bash', 'zsh', 'fish'],
    lineComments: ['#'],
    strings: [StringRule('"'), StringRule("'", escape: false)],
    keywords: _w('if then else elif fi for while until do done case esac in function select time return exit break continue export local readonly declare unset source alias set'),
    literals: _w('true false'),
    metaStart: r'$',
  ),
  Language(
    'powershell',
    names: ['ps1', 'pwsh', 'ps'],
    extensions: ['ps1', 'psm1'],
    lineComments: ['#'],
    blockComment: ('<#', '#>'),
    strings: [StringRule('"', escape: false), StringRule("'", escape: false)],
    keywords: _w('begin break catch class continue data do dynamicparam else elseif end exit filter finally for foreach from function if in param process return switch throw trap try until using var while'),
    literals: _w(r'$true $false $null'),
    metaStart: r'$',
    ignoreCase: true,
  ),
  Language(
    'json',
    names: ['jsonc', 'json5', 'jsonl', 'ndjson'],
    extensions: ['json', 'jsonc', 'json5', 'jsonl', 'ndjson'],
    lineComments: ['//'],
    blockComment: ('/*', '*/'),
    strings: [StringRule('"')],
    literals: _w('true false null'),
    jsonKeys: true,
  ),
  Language(
    'yaml',
    names: ['yml'],
    extensions: ['yaml', 'yml'],
    lineComments: ['#'],
    strings: [StringRule('"'), StringRule("'", escape: false)],
    literals: _w('true false null yes no on off True False Null'),
    keys: KeyStyle.colon,
  ),
  Language(
    'toml',
    extensions: ['toml', 'ini', 'cfg', 'conf'],
    names: ['ini', 'cfg'],
    lineComments: ['#', ';'],
    strings: [StringRule('"""', multiline: true), StringRule("'''", multiline: true, escape: false), StringRule('"'), StringRule("'", escape: false)],
    literals: _w('true false'),
    keys: KeyStyle.equals,
  ),
  Language(
    'html',
    names: ['htm', 'xhtml', 'vue', 'svelte'],
    extensions: ['html', 'htm', 'xhtml', 'vue', 'svelte'],
    blockComment: ('<!--', '-->'),
    strings: [StringRule('"', escape: false), StringRule("'", escape: false)],
    markup: true,
  ),
  Language(
    'xml',
    names: ['svg', 'xsd', 'xsl', 'plist'],
    extensions: ['xml', 'svg', 'xsd', 'xsl', 'plist', 'csproj', 'xaml'],
    blockComment: ('<!--', '-->'),
    strings: [StringRule('"', escape: false), StringRule("'", escape: false)],
    markup: true,
  ),
  Language(
    'css',
    names: ['scss', 'less'],
    extensions: ['css', 'scss', 'less'],
    blockComment: ('/*', '*/'),
    lineComments: [],
    strings: _cStrings,
    keywords: _w('important'),
    metaStart: '@',
    keys: KeyStyle.colon,
  ),
  Language(
    'sql',
    names: ['postgres', 'mysql', 'sqlite', 'psql'],
    extensions: ['sql'],
    lineComments: ['--'],
    blockComment: ('/*', '*/'),
    strings: [StringRule("'"), StringRule('"')],
    keywords: _w('add all alter and as asc between by case check column constraint create cross database default delete desc distinct drop else end exists foreign from full group having in index inner insert into is join key left like limit not null offset on or order outer primary references right select set table then union unique update values view when where with'),
    literals: _w('true false null'),
    types: _w('int integer bigint smallint text varchar char boolean date timestamp numeric decimal real float double blob'),
    ignoreCase: true,
  ),
  Language(
    'markdown',
    names: ['md'],
    extensions: ['md', 'markdown'],
  ),
  Language(
    'diff',
    names: ['patch'],
    extensions: ['diff', 'patch'],
    diff: true,
  ),
  Language(
    'lua',
    extensions: ['lua'],
    lineComments: ['--'],
    blockComment: ('--[[', ']]'),
    keywords: _w('and break do else elseif end for function goto if in local not or repeat return then until while'),
    literals: _w('true false nil'),
  ),
  Language(
    'ruby',
    names: ['rb'],
    extensions: ['rb', 'rake', 'gemspec'],
    lineComments: ['#'],
    keywords: _w('alias and begin break case class def defined do else elsif end ensure for if in module next not or redo rescue retry return self super then undef unless until when while yield'),
    literals: _w('true false nil'),
    capitalIsType: true,
  ),
];

final Map<String, Language> _byName = () {
  final map = <String, Language>{};
  for (final language in languages) {
    map[language.id] = language;
    for (final name in language.names) {
      map.putIfAbsent(name, () => language);
    }
  }
  return map;
}();

final Map<String, Language> _byExtension = () {
  final map = <String, Language>{};
  for (final language in languages) {
    for (final extension in language.extensions) {
      map.putIfAbsent(extension, () => language);
    }
  }
  return map;
}();

const Map<String, String> _byFileName = {
  'dockerfile': 'shell',
  'makefile': 'shell',
  'cargo.lock': 'toml',
  '.bashrc': 'shell',
  '.zshrc': 'shell',
  '.gitignore': 'shell',
  'pubspec.lock': 'yaml',
};

/// The language a fence's info string names (```` ```rust ````, ```` ```{.py} ````, ```` ```language-ts title="x" ````), or null.
Language? languageForInfo(String? info) {
  if (info == null) return null;
  var word = info.trim().split(RegExp(r'[\s,]+')).first.toLowerCase();
  word = word.replaceAll(RegExp(r'^[{.]+|[}]+$'), '');
  if (word.startsWith('language-')) word = word.substring(9);
  if (word.startsWith('lang-')) word = word.substring(5);
  return word.isEmpty ? null : _byName[word];
}

/// The language of a file named [path], by its name or its last extension, or null.
Language? languageForPath(String? path) {
  if (path == null || path.isEmpty) return null;
  final name = path.split(RegExp(r'[\\/]')).last.toLowerCase();
  final byName = _byFileName[name];
  if (byName != null) return _byName[byName];
  final cut = name.lastIndexOf('.');
  if (cut <= 0) return null;
  return _byExtension[name.substring(cut + 1)];
}

/// A guess from what [text] starts with (a `#!` line, an XML or HTML prolog, a JSON document, a unified diff), or null.
Language? sniffLanguage(String text) {
  final head = text.length > 2000 ? text.substring(0, 2000) : text;
  final trimmed = head.trimLeft();
  if (trimmed.isEmpty) return null;
  if (trimmed.startsWith('#!')) {
    final line = trimmed.split('\n').first;
    for (final (word, id) in const [('python', 'python'), ('node', 'javascript'), ('deno', 'typescript'), ('ruby', 'ruby'), ('pwsh', 'powershell'), ('sh', 'shell')]) {
      if (RegExp('\\b$word\\d*\\b').hasMatch(line) || (id == 'shell' && line.contains('sh'))) return _byName[id];
    }
    return null;
  }
  if (trimmed.startsWith('<?xml')) return _byName['xml'];
  final lower = trimmed.toLowerCase();
  if (lower.startsWith('<!doctype html') || lower.startsWith('<html')) return _byName['html'];
  if (RegExp(r'^(diff --git |--- .*\n\+\+\+ |@@ -\d)').hasMatch(trimmed)) return _byName['diff'];
  if ((trimmed.startsWith('{') || trimmed.startsWith('[')) && RegExp(r'^[\[{]\s*("|[\]}\[{]|\d|true|false|null)').hasMatch(trimmed)) {
    return _byName['json'];
  }
  return null;
}

/// What a line of code left open, to carry to the next line: 0 for nothing, 1 for a block comment, 2 + n for the language's string
/// rule n, which runs on.
typedef LexState = int;

bool _isDigit(int c) => c >= 0x30 && c <= 0x39;
bool _isNameStart(int c) => (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) || c == 0x5f || c == 0x24 || c > 0x7f;
bool _isNameChar(int c) => _isNameStart(c) || _isDigit(c);

/// The spans of [line] in [language] that are not plain, in order, and the state the line leaves. [state] is what the line before it
/// left.
(List<Span>, LexState) lexLine(Language language, String line, [LexState state = 0]) {
  final spans = <Span>[];
  void add(int start, int end, Tok tok) {
    if (end <= start || tok == Tok.plain) return;
    if (spans.isNotEmpty && spans.last.tok == tok && spans.last.end == start) {
      spans[spans.length - 1] = Span(spans.last.start, end, tok);
    } else {
      spans.add(Span(start, end, tok));
    }
  }

  final n = line.length;
  if (language.diff) {
    final tok = line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ') || line.startsWith('index ')
        ? Tok.meta
        : line.startsWith('@@')
        ? Tok.type
        : line.startsWith('+')
        ? Tok.inserted
        : line.startsWith('-')
        ? Tok.deleted
        : Tok.plain;
    add(0, n, tok);
    return (spans, 0);
  }

  var i = 0;
  // A string or comment that runs on from the line before.
  int? scanString(int from, StringRule rule) {
    var at = from;
    while (at < n) {
      if (rule.escape && line.codeUnitAt(at) == 0x5c) {
        at += 2;
        continue;
      }
      if (line.startsWith(rule.close, at)) return at + rule.close.length;
      at++;
    }
    return null;
  }

  final block = language.blockComment;
  if (state == 1 && block != null) {
    final end = line.indexOf(block.$2);
    if (end < 0) {
      add(0, n, Tok.comment);
      return (spans, 1);
    }
    i = end + block.$2.length;
    add(0, i, Tok.comment);
    state = 0;
  } else if (state >= 2 && state - 2 < language.strings.length) {
    final rule = language.strings[state - 2];
    final end = scanString(0, rule);
    if (end == null) {
      add(0, n, Tok.string);
      return (spans, state);
    }
    i = end;
    add(0, i, Tok.string);
    state = 0;
  } else {
    state = 0;
  }

  outer:
  while (i < n) {
    final c = line.codeUnitAt(i);
    if (c == 0x20 || c == 0x09) {
      i++;
      continue;
    }
    for (final marker in language.lineComments) {
      if (line.startsWith(marker, i)) {
        // A `#` that is part of a shell word or `$#` is not a comment.
        if (marker == '#' && i > 0 && language.id == 'shell' && !_isSpace(line.codeUnitAt(i - 1))) break;
        add(i, n, Tok.comment);
        i = n;
        continue outer;
      }
    }
    if (block != null && line.startsWith(block.$1, i)) {
      final end = line.indexOf(block.$2, i + block.$1.length);
      if (end < 0) {
        add(i, n, Tok.comment);
        return (spans, 1);
      }
      add(i, end + block.$2.length, Tok.comment);
      i = end + block.$2.length;
      continue;
    }
    for (var r = 0; r < language.strings.length; r++) {
      final rule = language.strings[r];
      if (!line.startsWith(rule.open, i)) continue;
      // A lone `'` after a name is a lifetime or an apostrophe, not a string: only Rust and a few others hit this.
      if (rule.open == "'" && language.id == 'rust' && !_rustChar(line, i)) break;
      final end = scanString(i + rule.open.length, rule);
      if (end == null) {
        add(i, n, Tok.string);
        return (spans, rule.multiline ? 2 + r : 0);
      }
      var tok = Tok.string;
      if (language.jsonKeys) {
        var after = end;
        while (after < n && _isSpace(line.codeUnitAt(after))) {
          after++;
        }
        if (after < n && line.codeUnitAt(after) == 0x3a) tok = Tok.type;
      }
      add(i, end, tok);
      i = end;
      continue outer;
    }
    if (language.markup && c == 0x3c) {
      var at = i + 1;
      if (at < n && line.codeUnitAt(at) == 0x2f) at++;
      final nameStart = at;
      while (at < n && (_isNameChar(line.codeUnitAt(at)) || line.codeUnitAt(at) == 0x2d || line.codeUnitAt(at) == 0x3a)) {
        at++;
      }
      if (at > nameStart) {
        add(nameStart, at, Tok.keyword);
        i = at;
        continue;
      }
    }
    if (_isDigit(c) || (c == 0x2e && i + 1 < n && _isDigit(line.codeUnitAt(i + 1)) && (i == 0 || !_isNameChar(line.codeUnitAt(i - 1))))) {
      if (i > 0 && _isNameChar(line.codeUnitAt(i - 1))) {
        i++;
        continue;
      }
      var at = i + 1;
      while (at < n) {
        final d = line.codeUnitAt(at);
        if (_isNameChar(d) || (d == 0x2e && at + 1 < n && _isDigit(line.codeUnitAt(at + 1)))) {
          at++;
        } else {
          break;
        }
      }
      add(i, at, Tok.number);
      i = at;
      continue;
    }
    final meta = language.metaStart;
    final isMeta = meta != null && line.startsWith(meta, i) && i + meta.length < n && _isNameStart(line.codeUnitAt(i + meta.length));
    if (_isNameStart(c) || isMeta) {
      var at = i + (isMeta ? meta.length : 0);
      while (at < n && (_isNameChar(line.codeUnitAt(at)) || (language.keys != KeyStyle.none && line.codeUnitAt(at) == 0x2d))) {
        at++;
      }
      final word = line.substring(i, at);
      Tok tok = Tok.plain;
      if (isMeta && !language.literals.contains(word)) {
        tok = Tok.meta;
      } else if (language.keys != KeyStyle.none && _followedByKey(line, at, language.keys)) {
        tok = Tok.type;
      } else if (language._hasWord(language.keywords, word)) {
        tok = Tok.keyword;
      } else if (language._hasWord(language.literals, word)) {
        tok = Tok.literal;
      } else if (language._hasWord(language.types, word) || (language.capitalIsType && _isUpper(c) && word.length > 1)) {
        tok = Tok.type;
      }
      add(i, at, tok);
      i = at;
      continue;
    }
    i++;
  }
  return (spans, state);
}

bool _isSpace(int c) => c == 0x20 || c == 0x09;
bool _isUpper(int c) => c >= 0x41 && c <= 0x5a;

/// Whether the `'` at [at] opens a character literal (`'a'`, `'\n'`) rather than a lifetime (`'a`).
bool _rustChar(String line, int at) {
  final n = line.length;
  if (at + 2 < n && line.codeUnitAt(at + 1) == 0x5c) return true;
  return at + 2 < n && line.codeUnitAt(at + 2) == 0x27;
}

bool _followedByKey(String line, int at, KeyStyle style) {
  var i = at;
  while (i < line.length && _isSpace(line.codeUnitAt(i))) {
    i++;
  }
  if (i >= line.length) return false;
  final c = line.codeUnitAt(i);
  if (style == KeyStyle.equals) return c == 0x3d;
  return c == 0x3a && (i + 1 >= line.length || _isSpace(line.codeUnitAt(i + 1)));
}
