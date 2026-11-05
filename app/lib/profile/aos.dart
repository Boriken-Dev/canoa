/// Overlays in the Agent Overlay Specification (AOS) v0 format, edition `0.1.0-draft`
/// (https://github.com/Boriken-Dev/agent-overlay-spec): reading `overlay.cfg` and env files, validating a manifest, overlay
/// names, source strings, tree digests, the defaults conventional files contribute, composition order, the environment
/// an overlay builds with its substitution, and an icon's header. canoa has the same (`canoa_host::aos`), and both
/// run the specification's vectors in `crates/canoa-host/tests/vectors/aos/`.
///
/// Canoa reads TOML and JSON; YAML is refused as an unsupported format. It runs no env script, no setup and no package
/// manager: those declarations are read and validated, and reported as not applied. Nothing here reads a file, runs
/// anything or consults the environment except through the callbacks a caller passes.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:toml/toml.dart';

/// A refusal, by the class the specification's conformance vectors name it, and what it is about: a variable, a path, a
/// format or an overlay, never a value.
final class AosException implements Exception {
  const AosException(this.code, [this.subject = '']);
  final String code;
  final String subject;
  @override
  String toString() => subject.isEmpty ? code : '$code: $subject';
}

/// What a reader reads at most: settings with the specification's recommended defaults and no floor.
final class AosLimits {
  const AosLimits({this.maxBytes = 1024 * 1024, this.maxDepth = 64});
  final int maxBytes, maxDepth;
}

// ------------------------------------------------------------------------------------------------------------------
// Names and paths.
// ------------------------------------------------------------------------------------------------------------------

const _reservedNames = {'default', 'overlay', 'system', 'user', 'project', 'project-root'};

/// Path components that are never part of an overlay.
const excludedNames = {
  '.git', '.hg', '.svn', '__pycache__', '.mypy_cache', '.pytest_cache', '.ruff_cache', '.tox', '.venv', 'node_modules',
};

String _asciiLower(String text) => String.fromCharCodes(text.codeUnits.map((c) => c >= 0x41 && c <= 0x5a ? c + 32 : c));
String _asciiUpper(String text) => String.fromCharCodes(text.codeUnits.map((c) => c >= 0x61 && c <= 0x7a ? c - 32 : c));
bool _equalsIgnoreAsciiCase(String a, String b) => _asciiLower(a) == _asciiLower(b);

bool _isDeviceStem(String stem, {required bool aliases}) {
  final lower = _asciiLower(stem);
  if (const {'con', 'prn', 'aux', 'nul'}.contains(lower)) return true;
  if (aliases && (lower == r'conin$' || lower == r'conout$')) return true;
  final runes = lower.runes.toList();
  if (runes.length != 4) return false;
  final head = String.fromCharCodes(runes.take(3));
  final digit = runes[3];
  final ascii = digit >= 0x30 && digit <= 0x39;
  return (head == 'com' || head == 'lpt') &&
      (ascii && (aliases || digit != 0x30) || aliases && (digit == 0xb9 || digit == 0xb2 || digit == 0xb3));
}

final _overlayNamePattern = RegExp(r'^[A-Za-z](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?$');

/// Whether [name] is a valid overlay identity.
bool validOverlayName(String name) =>
    _overlayNamePattern.hasMatch(name) && !_reservedNames.contains(_asciiLower(name)) &&
    !_isDeviceStem(name.split('.').first, aliases: false);

/// An identity as it is compared: ASCII lowercase, with `_` and `.` made `-`.
String normalizeName(String name) => _asciiLower(name).replaceAll(RegExp('[_.]'), '-');

String _variableStem(String identity) => _asciiUpper(normalizeName(identity)).replaceAll(RegExp('[^A-Z0-9]'), '_');

/// `<NAME>_OVERLAY_ROOT` for an overlay identity.
String rootVariable(String identity) => '${_variableStem(identity)}_OVERLAY_ROOT';

/// `<NAME>_OVERLAY_STATE` for an overlay identity.
String stateVariable(String identity) => '${_variableStem(identity)}_OVERLAY_STATE';

/// Whether a variable belongs to the consumer: `AOS_*`, `*_OVERLAY_ROOT` or `*_OVERLAY_STATE`, in any ASCII case.
bool isReservedVariable(String name) {
  final upper = _asciiUpper(name);
  return upper.startsWith('AOS_') || upper.endsWith('_OVERLAY_ROOT') || upper.endsWith('_OVERLAY_STATE');
}

final _envNamePattern = RegExp(r'^[A-Za-z_][A-Za-z0-9_]*$');

/// `[A-Za-z_][A-Za-z0-9_]*`.
bool isAosEnvName(String name) => _envNamePattern.hasMatch(name);

const _sensitiveNames = {
  'PATH', 'PATHEXT', 'HOME', 'USERPROFILE', 'SHELL', 'COMSPEC', 'SYSTEMROOT', 'BASH_ENV', 'ENV', 'ZDOTDIR',
  'PROMPT_COMMAND', 'IFS', 'EDITOR', 'VISUAL', 'PAGER', 'LESSOPEN', 'LESSCLOSE', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
  'APPDATA', 'LOCALAPPDATA', 'TMPDIR', 'TEMP', 'TMP', 'PSMODULEPATH', 'CLASSPATH', 'JAVA_TOOL_OPTIONS', '_JAVA_OPTIONS',
  'JDK_JAVA_OPTIONS', 'PERL5OPT', 'PERL5LIB', 'RUBYOPT', 'RUBYLIB', 'RUSTFLAGS', 'GOPROXY', 'GONOSUMDB', 'GONOSUMCHECK',
  'GOPRIVATE', 'GOFLAGS', 'SSH_ASKPASS', 'SSLKEYLOGFILE', 'REQUESTS_CA_BUNDLE', 'CURL_CA_BUNDLE',
};
const _sensitivePrefixes = [
  'LD_', 'DYLD_', 'GIT_', 'PYTHON', 'NODE_', 'NPM_CONFIG_', 'PIP_', 'UV_', 'CARGO_', 'RUSTC_', 'DOTNET_', 'COR_',
  'CORECLR_', 'SSL_',
];

/// The specification's sensitive variables, compared without ASCII case.
bool isSensitive(String name) {
  final upper = _asciiUpper(name);
  return _sensitiveNames.contains(upper) || _sensitivePrefixes.any(upper.startsWith) || upper.endsWith('_PROXY') ||
      upper.endsWith('_BASE_URL');
}

bool _forbiddenInPath(int c) =>
    '/\\:*?"<>|'.codeUnits.contains(c) || c <= 0x1f || (c >= 0x7f && c <= 0x9f) || (c >= 0x200c && c <= 0x200f) ||
    (c >= 0x202a && c <= 0x202e) || (c >= 0x2066 && c <= 0x206f) || c == 0xfeff;

/// A relative path as the specification defines it.
bool isRelativePath(String path) {
  if (path.isEmpty || RegExp('~[0-9]').hasMatch(path)) return false;
  return path.split('/').every((component) {
    final lower = _asciiLower(component);
    return component.isNotEmpty && !component.endsWith('.') && !component.endsWith(' ') &&
        !component.runes.any(_forbiddenInPath) && !excludedNames.contains(lower) &&
        !_isDeviceStem(lower.split('.').first, aliases: true) && !lower.endsWith('.pyc');
  });
}

/// Where an overlay comes from, as a source string names it.
enum SourceKind { git, url, directory }

/// A parsed source string, `[git+]<location>[@<ref>][#<path>]`.
final class AosSource {
  const AosSource(this.kind, this.location, this.reference, this.path);
  final SourceKind kind;
  final String location;
  final String? reference, path;
}

bool _gitLocation(String location) =>
    location.endsWith('.git') || location.startsWith('git@') || location.startsWith('ssh://') ||
    location.startsWith('git://');

/// Parses a source string as the specification's distribution section says; throws `invalid-source`.
AosSource parseSource(String text) {
  Never refused(String why) => throw AosException('invalid-source', why);
  var raw = text.trim();
  if (raw.isEmpty) refused('empty');
  final explicit = raw.startsWith('git+');
  if (explicit) raw = raw.substring(4);
  var body = raw;
  String? path;
  final hash = raw.indexOf('#');
  if (hash >= 0) {
    body = raw.substring(0, hash);
    var tail = raw.substring(hash + 1).trim();
    while (tail.startsWith('/')) {
      tail = tail.substring(1);
    }
    while (tail.endsWith('/')) {
      tail = tail.substring(0, tail.length - 1);
    }
    path = tail.isEmpty ? null : tail;
  }
  var location = body.trim();
  String? reference;
  final at = location.lastIndexOf('@');
  if (at >= 0) {
    final head = location.substring(0, at);
    final tail = location.substring(at + 1);
    if (head.isNotEmpty && tail.isNotEmpty && !tail.contains(RegExp(r'[/:\\]')) && !head.endsWith('://') &&
        !head.endsWith(':') && (explicit || _gitLocation(head))) {
      location = head;
      reference = tail;
    }
  }
  if (path != null && !isRelativePath(path)) refused('path');
  if (explicit || _gitLocation(location)) {
    if (location.startsWith('-') || (reference?.startsWith('-') ?? false)) refused('option-like');
    if (location.contains('::')) refused('git remote helper');
    var host = '';
    final scheme = location.indexOf('://');
    if (scheme >= 0) {
      if (!const {'https', 'http', 'ssh', 'git', 'file'}.contains(_asciiLower(location.substring(0, scheme)))) {
        refused('scheme');
      }
      host = location.substring(scheme + 3).split('/').first.split('@').last;
    } else if (location.contains('@')) {
      host = location.split(':').first.split('@').last;
    }
    if (host.startsWith('-')) refused('option-like host');
    return AosSource(SourceKind.git, location, reference, path);
  }
  return AosSource(location.contains('://') ? SourceKind.url : SourceKind.directory, location, null, path);
}

// ------------------------------------------------------------------------------------------------------------------
// Reading.
// ------------------------------------------------------------------------------------------------------------------

AosException _error(String code) => AosException(code);

(String, List<int>) _selectFormat(List<int> data) {
  var bytes = data;
  if (bytes.length >= 3 && bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf) bytes = bytes.sublist(3);
  if (bytes.length < 2 || bytes[0] != 0x23 || bytes[1] != 0x21) return ('toml', bytes);
  final end = bytes.indexOf(0x0a);
  final line = end < 0 ? bytes.sublist(2) : bytes.sublist(2, end);
  final payload = end < 0 ? <int>[] : bytes.sublist(end + 1);
  final String text;
  try {
    text = utf8.decode(line);
  } on FormatException {
    throw _error('invalid-utf8');
  }
  final words = text.split(RegExp('[ \t\f\r]+')).where((word) => word.isNotEmpty);
  var word = words.isEmpty ? '' : words.last;
  word = word.split(RegExp(r'[/\\]')).last;
  while (word.startsWith('.')) {
    word = word.substring(1);
  }
  if (word.isEmpty) throw _error('no-format-named');
  return (_asciiLower(word), payload);
}

/// Reads an AOS document (`overlay.cfg`, or an `aos` env file) into the data model. TOML is the default and JSON is read
/// after a `#!json` line; YAML and every other format are refused by name.
Map<String, Object> readDocument(List<int> bytes, {AosLimits limits = const AosLimits()}) {
  if (bytes.length > limits.maxBytes) throw _error('too-large');
  final (format, payload) = _selectFormat(bytes);
  if (format != 'toml' && format != 'json') {
    throw AosException('unsupported-format', RegExp(r'^[A-Za-z0-9._-]+$').hasMatch(format) ? format : '(not shown)');
  }
  return _parse(format, payload, limits.maxDepth);
}

/// Reads [bytes] as [format], `toml` or `json`, chosen by something other than a format line, as a catalog's file name
/// chooses it: one leading byte-order mark is dropped and the rest is parsed whole, with the data model's refusals.
Map<String, Object> readDocumentAs(String format, List<int> bytes, {AosLimits limits = const AosLimits()}) {
  if (bytes.length > limits.maxBytes) throw _error('too-large');
  if (format != 'toml' && format != 'json') throw AosException('unsupported-format', format);
  final hasMark = bytes.length >= 3 && bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf;
  return _parse(format, hasMark ? bytes.sublist(3) : bytes, limits.maxDepth);
}

Map<String, Object> _parse(String format, List<int> payload, int maxDepth) {
  final String text;
  try {
    text = utf8.decode(payload);
  } on FormatException {
    throw _error('invalid-utf8');
  }
  // Checked on the bytes: the decoder drops a leading byte-order mark.
  if (payload.length >= 3 && payload[0] == 0xef && payload[1] == 0xbb && payload[2] == 0xbf) throw _error('parse-error');
  if (text.replaceAll(RegExp('[ \t\r\n]'), '').isEmpty) return {};
  final Object? value;
  if (format == 'toml') {
    final Map<String, dynamic> map;
    try {
      map = TomlDocument.parse(text).toMap();
    } on Object {
      throw _error('parse-error');
    }
    value = _fromToml(map, 1, maxDepth);
  } else {
    value = _Json(text, maxDepth).document();
  }
  if (value is! Map<String, Object?>) throw _error('not-a-mapping');
  _checkModel(value, 1, maxDepth);
  return value.cast<String, Object>();
}

Object _fromToml(Object? value, int depth, int maxDepth) => switch (value) {
  final String text => text,
  final bool flag => flag,
  final int number => number,
  final BigInt _ => throw _error('integer-range'),
  final double number => number.isFinite ? number : throw _error('non-finite-float'),
  final Map<dynamic, dynamic> map => depth > maxDepth
      ? throw _error('too-deep')
      : <String, Object?>{for (final MapEntry(:key, :value) in map.entries) '$key': _fromToml(value, depth + 1, maxDepth)},
  final Iterable<dynamic> items =>
    depth > maxDepth ? throw _error('too-deep') : [for (final item in items) _fromToml(item, depth + 1, maxDepth)],
  _ => throw _error('datetime-value'),
};

void _checkModel(Object? value, int depth, int maxDepth) {
  switch (value) {
    case null:
      throw _error('null-value');
    case final List<Object?> items:
      if (depth > maxDepth) throw _error('too-deep');
      for (final item in items) {
        _checkModel(item, depth + 1, maxDepth);
      }
    case final Map<String, Object?> map:
      if (depth > maxDepth) throw _error('too-deep');
      for (final item in map.values) {
        _checkModel(item, depth + 1, maxDepth);
      }
    case final double number when !number.isFinite:
      throw _error('non-finite-float');
  }
}

/// RFC 8259 JSON with the data model's refusals; `null` is read, and refused after.
final class _Json {
  _Json(this.text, this.maxDepth);
  final String text;
  final int maxDepth;
  var at = 0;

  Object? document() {
    final value = _value(1);
    _space();
    if (at != text.length) throw _error('parse-error');
    return value;
  }

  int? get _peek => at < text.length ? text.codeUnitAt(at) : null;

  void _space() {
    while (const {0x20, 0x09, 0x0a, 0x0d}.contains(_peek)) {
      at++;
    }
  }

  Object? _literal(String word, Object? value) {
    if (!text.startsWith(word, at)) throw _error('parse-error');
    at += word.length;
    return value;
  }

  Object? _value(int depth) {
    _space();
    switch (_peek) {
      case 0x7b:
        if (depth > maxDepth) throw _error('too-deep');
        at++;
        final map = <String, Object?>{};
        _space();
        if (_peek == 0x7d) {
          at++;
          return map;
        }
        while (true) {
          _space();
          if (_peek != 0x22) throw _error('parse-error');
          final key = _string();
          _space();
          if (_peek != 0x3a) throw _error('parse-error');
          at++;
          final value = _value(depth + 1);
          if (map.containsKey(key)) throw AosException('duplicate-key', key);
          map[key] = value;
          _space();
          switch (_peek) {
            case 0x2c:
              at++;
            case 0x7d:
              at++;
              return map;
            default:
              throw _error('parse-error');
          }
        }
      case 0x5b:
        if (depth > maxDepth) throw _error('too-deep');
        at++;
        final items = <Object?>[];
        _space();
        if (_peek == 0x5d) {
          at++;
          return items;
        }
        while (true) {
          items.add(_value(depth + 1));
          _space();
          switch (_peek) {
            case 0x2c:
              at++;
            case 0x5d:
              at++;
              return items;
            default:
              throw _error('parse-error');
          }
        }
      case 0x22:
        return _string();
      case 0x74:
        return _literal('true', true);
      case 0x66:
        return _literal('false', false);
      case 0x6e:
        return _literal('null', null);
      case final int c when c == 0x2d || (c >= 0x30 && c <= 0x39):
        return _number();
      default:
        throw _error('parse-error');
    }
  }

  int _digits() {
    final start = at;
    while (at < text.length && _isDigit(text.codeUnitAt(at))) {
      at++;
    }
    return at - start;
  }

  Object _number() {
    final start = at;
    if (_peek == 0x2d) at++;
    if (_peek == 0x30) {
      at++;
    } else if (_peek case final int c when c >= 0x31 && c <= 0x39) {
      _digits();
    } else {
      throw _error('parse-error');
    }
    var float = false;
    if (_peek == 0x2e) {
      at++;
      if (_digits() == 0) throw _error('parse-error');
      float = true;
    }
    if (_peek == 0x65 || _peek == 0x45) {
      at++;
      if (_peek == 0x2b || _peek == 0x2d) at++;
      if (_digits() == 0) throw _error('parse-error');
      float = true;
    }
    final literal = text.substring(start, at);
    if (float) {
      final number = double.parse(literal);
      if (!number.isFinite) throw _error('non-finite-float');
      return number;
    }
    final big = BigInt.parse(literal);
    if (big < BigInt.from(-0x8000000000000000) || big > BigInt.parse('9223372036854775807')) {
      throw _error('integer-range');
    }
    return big.toInt();
  }

  int _hex4() {
    if (at + 4 > text.length) throw _error('parse-error');
    final digits = text.substring(at, at + 4);
    if (!RegExp(r'^[0-9A-Fa-f]{4}$').hasMatch(digits)) throw _error('parse-error');
    at += 4;
    return int.parse(digits, radix: 16);
  }

  String _string() {
    at++;
    final out = StringBuffer();
    while (true) {
      final c = _peek;
      if (c == null) throw _error('parse-error');
      at++;
      if (c == 0x22) break;
      if (c < 0x20) throw _error('parse-error');
      if (c != 0x5c) {
        out.writeCharCode(c);
        continue;
      }
      final escape = _peek;
      if (escape == null) throw _error('parse-error');
      at++;
      switch (escape) {
        case 0x22 || 0x5c || 0x2f:
          out.writeCharCode(escape);
        case 0x62:
          out.writeCharCode(0x08);
        case 0x66:
          out.writeCharCode(0x0c);
        case 0x6e:
          out.writeCharCode(0x0a);
        case 0x72:
          out.writeCharCode(0x0d);
        case 0x74:
          out.writeCharCode(0x09);
        case 0x75:
          final high = _hex4();
          if (high >= 0xd800 && high < 0xdc00) {
            if (!text.startsWith(r'\u', at)) throw _error('invalid-unicode');
            at += 2;
            final low = _hex4();
            if (low < 0xdc00 || low >= 0xe000) throw _error('invalid-unicode');
            out.writeCharCode(0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00));
          } else if (high >= 0xdc00 && high < 0xe000) {
            throw _error('invalid-unicode');
          } else {
            out.writeCharCode(high);
          }
        default:
          throw _error('parse-error');
      }
    }
    return out.toString();
  }
}

/// One line of a `dotenv` file: the variable, its text, and whether single quotes keep it as written.
final class DotenvLine {
  const DotenvLine(this.name, this.text, this.literal);
  final String name, text;
  final bool literal;
}

(String, String)? _dotenvAssignment(String text) {
  final match = RegExp(r'^([A-Za-z0-9_]*)').firstMatch(text)!;
  final name = match[1]!;
  if (!isAosEnvName(name)) return null;
  final rest = text.substring(name.length).replaceFirst(RegExp('^[ \t]*'), '');
  if (!rest.startsWith('=')) return null;
  return (name, rest.substring(1));
}

/// Reads a `dotenv` file's lines, in order; nothing is substituted yet.
List<DotenvLine> parseDotenv(List<int> data) {
  var bytes = data;
  if (bytes.length >= 3 && bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf) bytes = bytes.sublist(3);
  final String text;
  try {
    text = utf8.decode(bytes);
  } on FormatException {
    throw _error('invalid-utf8');
  }
  final out = <DotenvLine>[];
  for (var line in text.split('\n')) {
    if (line.endsWith('\r')) line = line.substring(0, line.length - 1);
    final stripped = line.replaceAll(RegExp(r'^[ \t]+|[ \t]+$'), '');
    if (stripped.isEmpty || stripped.startsWith('#')) continue;
    final trimmed = line.replaceFirst(RegExp('^[ \t]+'), '');
    (String, String)? found;
    if (trimmed.startsWith('export') && trimmed.length > 6 && (trimmed[6] == ' ' || trimmed[6] == '\t')) {
      found = _dotenvAssignment(trimmed.substring(6).replaceFirst(RegExp('^[ \t]+'), ''));
    }
    found ??= _dotenvAssignment(trimmed);
    if (found == null) throw _error('dotenv-syntax');
    final (name, raw) = found;
    var value = raw.replaceAll(RegExp(r'^[ \t]+|[ \t]+$'), '');
    var literal = false;
    final runes = value.runes.toList();
    if (runes.length >= 2 && runes.first == runes.last && (runes.first == 0x22 || runes.first == 0x27)) {
      literal = runes.first == 0x27;
      value = String.fromCharCodes(runes.sublist(1, runes.length - 1));
    }
    if (out.any((line) => _equalsIgnoreAsciiCase(line.name, name))) throw AosException('duplicate-key', name);
    out.add(DotenvLine(name, value, literal));
  }
  return out;
}

/// Reads a `dotenv` file into [env], each line substituted against what the earlier lines left; returns what each set.
Map<String, String> readDotenv(List<int> bytes, AosEnvironment env) {
  final out = <String, String>{};
  for (final line in parseDotenv(bytes)) {
    final value = line.literal ? line.text : substitute(line.text, env.lookup);
    env.set(line.name, value);
    out[line.name] = value;
  }
  return out;
}

// ------------------------------------------------------------------------------------------------------------------
// Substitution.
// ------------------------------------------------------------------------------------------------------------------

/// What a placeholder reads: the value, null when the variable is unset; throws for one it may not read.
typedef AosLookup = String? Function(String name);

/// What one substitution may cost: settings with defaults and no floor, so text from elsewhere cannot exhaust memory, the
/// stack or the processor.
final class SubstLimits {
  const SubstLimits({this.maxValue = 1024 * 1024, this.maxDepth = 32, this.maxSteps = 10000000});

  /// The longest value one substitution gives, in UTF-16 units.
  final int maxValue;

  /// How deep expressions may nest inside the words of others.
  final int maxDepth;

  /// How many steps pattern matching may take in one substitution.
  final int maxSteps;
}

final class _Run {
  _Run(this.lookup, this.limits);
  final AosLookup lookup;
  final SubstLimits limits;
  var depth = 0, steps = 0;

  void step(int cost) {
    steps += cost;
    if (steps > limits.maxSteps) throw const AosException('too-complex', 'a pattern');
  }

  void fits(int length) {
    if (length > limits.maxValue) throw const AosException('too-large', 'a substituted value');
  }
}

/// [text] with every placeholder replaced, reading variables through [lookup]. Past [limits]: `too-large`, `too-deep`
/// and `too-complex`.
String substitute(String text, AosLookup lookup, {SubstLimits limits = const SubstLimits()}) =>
    _substitute(text.runes.toList(), _Run(lookup, limits));

String _substitute(List<int> text, _Run run) {
  if (run.depth > run.limits.maxDepth) throw const AosException('too-deep', 'an expression');
  run.depth++;
  try {
    return _substituteInner(text, run);
  } finally {
    run.depth--;
  }
}

String _substituteInner(List<int> text, _Run run) {
  final out = StringBuffer();
  var index = 0;
  while (index < text.length) {
    if (text[index] != 0x24) {
      out.writeCharCode(text[index]);
      index++;
      continue;
    }
    final next = index + 1 < text.length ? text[index + 1] : null;
    if (next == 0x24) {
      out.write(r'$');
      index += 2;
      continue;
    }
    if (next == 0x7b) {
      final close = _closingBrace(text, index + 2);
      if (close != null) {
        final found = _expression(text.sublist(index + 2, close), run);
        if (found != null) {
          out.write(found);
          run.fits(out.length);
          index = close + 1;
          continue;
        }
      }
    }
    out.write(r'$');
    index++;
  }
  run.fits(out.length);
  return out.toString();
}

int? _closingBrace(List<int> text, int start) {
  var depth = 0;
  var index = start;
  while (index < text.length) {
    if (text[index] == 0x24 && index + 1 < text.length && text[index + 1] == 0x7b) {
      depth++;
      index += 2;
      continue;
    }
    if (text[index] == 0x7d) {
      if (depth == 0) return index;
      depth--;
    }
    index++;
  }
  return null;
}

bool _isNameStart(int c) => (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) || c == 0x5f;
bool _isDigit(int c) => c >= 0x30 && c <= 0x39;

int _nameAt(List<int> text) {
  if (text.isEmpty || !_isNameStart(text.first)) return 0;
  var length = 1;
  while (length < text.length && (_isNameStart(text[length]) || _isDigit(text[length]))) {
    length++;
  }
  return length;
}

String _required(String name, _Run run) => run.lookup(name) ?? (throw AosException('undefined-variable', name));

bool _startsWith(List<int> text, String prefix) {
  final runes = prefix.runes.toList();
  if (text.length < runes.length) return false;
  for (var i = 0; i < runes.length; i++) {
    if (text[i] != runes[i]) return false;
  }
  return true;
}

String? _expression(List<int> body, _Run run) {
  if (body.isNotEmpty && body.first == 0x23) {
    final length = _nameAt(body.sublist(1));
    if (length > 0 && length + 1 == body.length) {
      return '${_required(String.fromCharCodes(body.sublist(1)), run).runes.length}';
    }
  }
  final length = _nameAt(body);
  if (length == 0) return null;
  final key = String.fromCharCodes(body.sublist(0, length));
  final rest = body.sublist(length);
  if (rest.isEmpty) return _required(key, run);
  for (final operator in const [':-', ':=', ':+', '-', '=', '+']) {
    if (!_startsWith(rest, operator)) continue;
    final word = rest.sublist(operator.length);
    final value = run.lookup(key);
    final present = value != null && !(operator.startsWith(':') && value.isEmpty);
    if (operator.endsWith('+')) return present ? _substitute(word, run) : '';
    return present ? value : _substitute(word, run);
  }
  switch (rest.first) {
    case 0x3a:
      final spec = _sliceSpec(rest.sublist(1));
      if (spec == null) return null;
      return _slice(_required(key, run), spec.$1, spec.$2);
    case 0x2f:
      final spec = rest.sublist(1);
      final kind = spec.isNotEmpty && (spec.first == 0x2f || spec.first == 0x23 || spec.first == 0x25) ? spec.first : null;
      final (pattern, replacement) = _splitPattern(spec.sublist(kind == null ? 0 : 1));
      final value = _required(key, run);
      return _replace(value, kind, pattern, replacement, run);
    case 0x23 || 0x25:
      final doubled = rest.length > 1 && rest[1] == rest[0];
      final value = _required(key, run);
      return _trim(value, rest[0], doubled, rest.sublist(doubled ? 2 : 1), run);
    default:
      final operator = String.fromCharCodes(rest);
      if (!const {'^', '^^', ',', ',,'}.contains(operator)) return null;
      final value = _required(key, run);
      if (value.isEmpty) return value;
      final runes = value.runes.toList();
      final first = String.fromCharCode(runes.first);
      final others = String.fromCharCodes(runes.skip(1));
      return switch (operator) {
        '^^' => _upper(value),
        ',,' => _lower(value),
        '^' => _upper(first) + others,
        _ => _lower(first) + others,
      };
  }
}

/// Unicode's unconditional full uppercase mappings (SpecialCasing.txt) that `toUpperCase` leaves out.
const _specialUpper = <int, List<int>>{
  0xdf: [0x53, 0x53], 0x149: [0x2bc, 0x4e], 0x1f0: [0x4a, 0x30c], 0x390: [0x399, 0x308, 0x301],
  0x3b0: [0x3a5, 0x308, 0x301], 0x587: [0x535, 0x552], 0x1e96: [0x48, 0x331], 0x1e97: [0x54, 0x308],
  0x1e98: [0x57, 0x30a], 0x1e99: [0x59, 0x30a], 0x1e9a: [0x41, 0x2be], 0x1f50: [0x3a5, 0x313],
  0x1f52: [0x3a5, 0x313, 0x300], 0x1f54: [0x3a5, 0x313, 0x301], 0x1f56: [0x3a5, 0x313, 0x342],
  0x1fb2: [0x1fba, 0x399], 0x1fb3: [0x391, 0x399], 0x1fb4: [0x386, 0x399], 0x1fb6: [0x391, 0x342],
  0x1fb7: [0x391, 0x342, 0x399], 0x1fbc: [0x391, 0x399], 0x1fc2: [0x1fca, 0x399], 0x1fc3: [0x397, 0x399],
  0x1fc4: [0x389, 0x399], 0x1fc6: [0x397, 0x342], 0x1fc7: [0x397, 0x342, 0x399], 0x1fcc: [0x397, 0x399],
  0x1fd2: [0x399, 0x308, 0x300], 0x1fd3: [0x399, 0x308, 0x301], 0x1fd6: [0x399, 0x342],
  0x1fd7: [0x399, 0x308, 0x342], 0x1fe2: [0x3a5, 0x308, 0x300], 0x1fe3: [0x3a5, 0x308, 0x301],
  0x1fe4: [0x3a1, 0x313], 0x1fe6: [0x3a5, 0x342], 0x1fe7: [0x3a5, 0x308, 0x342], 0x1ff2: [0x1ffa, 0x399],
  0x1ff3: [0x3a9, 0x399], 0x1ff4: [0x38f, 0x399], 0x1ff6: [0x3a9, 0x342], 0x1ff7: [0x3a9, 0x342, 0x399],
  0x1ffc: [0x3a9, 0x399], 0xfb00: [0x46, 0x46], 0xfb01: [0x46, 0x49], 0xfb02: [0x46, 0x4c],
  0xfb03: [0x46, 0x46, 0x49], 0xfb04: [0x46, 0x46, 0x4c], 0xfb05: [0x53, 0x54], 0xfb06: [0x53, 0x54],
  0xfb13: [0x544, 0x546], 0xfb14: [0x544, 0x535], 0xfb15: [0x544, 0x53b], 0xfb16: [0x54e, 0x546],
  0xfb17: [0x544, 0x53d],
};

/// Unicode's default full uppercase, as Rust's `to_uppercase` gives it.
String _upper(String text) {
  final out = StringBuffer();
  for (final rune in text.runes) {
    final special = _specialUpper[rune];
    if (special != null) {
      out.write(String.fromCharCodes(special));
    } else if (rune >= 0x1f80 && rune <= 0x1faf) {
      // Greek with ypogegrammeni: the capital without it, then a capital iota.
      out.write(String.fromCharCodes([const [0x1f08, 0x1f28, 0x1f68][(rune - 0x1f80) ~/ 16] + (rune & 0x7), 0x399]));
    } else {
      out.write(String.fromCharCode(rune).toUpperCase());
    }
  }
  return out.toString();
}

final _letter = RegExp(r'\p{L}', unicode: true);

/// Unicode's default full lowercase, as Rust's `to_lowercase` gives it: `İ` keeps its dot, and a capital sigma that ends
/// a word is final.
String _lower(String text) {
  final runes = text.runes.toList();
  final out = StringBuffer();
  for (var i = 0; i < runes.length; i++) {
    final rune = runes[i];
    if (rune == 0x130) {
      out.write(String.fromCharCodes(const [0x69, 0x307]));
    } else if (rune == 0x3a3 &&
        i > 0 &&
        _letter.hasMatch(String.fromCharCode(runes[i - 1])) &&
        (i + 1 == runes.length || !_letter.hasMatch(String.fromCharCode(runes[i + 1])))) {
      out.writeCharCode(0x3c2);
    } else {
      out.write(String.fromCharCode(rune).toLowerCase());
    }
  }
  return out.toString();
}

bool _isSpace(int c) => c == 0x20 || c == 0x09 || c == 0x0a || c == 0x0d || c == 0x0b || c == 0x0c;

/// The largest number a slice spec reads: a 64-bit int's maximum natively, 2^53 - 1 on the web, whose ints are doubles and where
/// no literal past 2^53 compiles.
final int _maxNumber = identical(1, 1.0) ? 9007199254740991 : int.parse('9223372036854775807');

int _number(List<int> digits, bool negative) {
  final max = _maxNumber;
  var value = 0;
  for (final digit in digits) {
    value = value > (max - 9) ~/ 10 ? max : value * 10 + (digit - 0x30);
  }
  return negative ? -value : value;
}

(int, int?)? _sliceSpec(List<int> text) {
  var at = 0;
  int skip() {
    final start = at;
    while (at < text.length && _isSpace(text[at])) {
      at++;
    }
    return at - start;
  }

  List<int> digits() {
    final start = at;
    while (at < text.length && _isDigit(text[at])) {
      at++;
    }
    return text.sublist(start, at);
  }

  final spaces = skip();
  final int offset;
  if (at < text.length && text[at] == 0x28) {
    at++;
    skip();
    final negative = at < text.length && text[at] == 0x2d;
    if (negative) at++;
    final found = digits();
    if (found.isEmpty) return null;
    offset = _number(found, negative);
    skip();
    if (at >= text.length || text[at] != 0x29) return null;
    at++;
  } else if (at < text.length && _isDigit(text[at])) {
    offset = _number(digits(), false);
  } else if (at < text.length && text[at] == 0x2d && spaces > 0) {
    at++;
    final found = digits();
    if (found.isEmpty) return null;
    offset = _number(found, true);
  } else {
    return null;
  }
  int? length;
  if (at < text.length && text[at] == 0x3a) {
    at++;
    skip();
    final negative = at < text.length && text[at] == 0x2d;
    if (negative) at++;
    final found = digits();
    if (found.isEmpty) return null;
    length = _number(found, negative);
  }
  skip();
  return at == text.length ? (offset, length) : null;
}

String _slice(String value, int offset, int? length) {
  final runes = value.runes.toList();
  final size = runes.length;
  var start = offset;
  if (start < 0) {
    start += size;
    if (start < 0) return '';
  }
  final end = length == null ? size : (length >= 0 ? start + length : size + length);
  if (end <= start) return '';
  final from = start < size ? start : size;
  final to = end < size ? end : size;
  return to <= from ? '' : String.fromCharCodes(runes.sublist(from, to));
}

(List<int>, List<int>) _splitPattern(List<int> spec) {
  var index = 0;
  while (index < spec.length) {
    if (spec[index] == 0x5c) {
      index += 2;
      continue;
    }
    if (spec[index] == 0x2f) return (spec.sublist(0, index), spec.sublist(index + 1));
    index++;
  }
  return (spec, const []);
}

/// A pattern token: a star, any one character, one character, or a class of ranges.
final class _Token {
  const _Token.star() : kind = 0, char = 0, negate = false, members = const [];
  const _Token.any() : kind = 1, char = 0, negate = false, members = const [];
  const _Token.char(this.char) : kind = 2, negate = false, members = const [];
  const _Token.klass(this.negate, this.members) : kind = 3, char = 0;
  final int kind, char;
  final bool negate;
  final List<(int, int)> members;

  bool get isStar => kind == 0;
  bool matches(int c) => switch (kind) {
    0 || 1 => true,
    2 => c == char,
    _ => members.any((range) => c >= range.$1 && c <= range.$2) != negate,
  };
}

List<_Token> _compile(List<int> pattern) {
  Never invalid() => throw AosException('invalid-pattern', String.fromCharCodes(pattern));
  final out = <_Token>[];
  var index = 0;
  while (index < pattern.length) {
    final c = pattern[index];
    if (c == 0x5c && index + 1 < pattern.length) {
      out.add(_Token.char(pattern[index + 1]));
      index += 2;
      continue;
    }
    if (c == 0x2a) {
      out.add(const _Token.star());
    } else if (c == 0x3f) {
      out.add(const _Token.any());
    } else if (c == 0x5b && pattern.indexOf(0x5d, index + 2) >= 0) {
      final close = pattern.indexOf(0x5d, index + 2);
      final body = pattern.sublist(index + 1, close);
      final negate = body.isNotEmpty && (body.first == 0x21 || body.first == 0x5e);
      final items = body.sublist(negate ? 1 : 0);
      if (items.isEmpty) invalid();
      final members = <(int, int)>[];
      var at = 0;
      while (at < items.length) {
        if (at + 2 < items.length && items[at + 1] == 0x2d) {
          if (items[at + 2] < items[at]) invalid();
          members.add((items[at], items[at + 2]));
          at += 3;
        } else {
          members.add((items[at], items[at]));
          at++;
        }
      }
      out.add(_Token.klass(negate, members));
      index = close + 1;
      continue;
    } else {
      out.add(_Token.char(c));
    }
    index++;
  }
  return out;
}

bool _whole(List<_Token> tokens, List<int> text, _Run run) {
  var t = 0, s = 0, steps = 0;
  int? starAt, starFrom;
  while (s < text.length) {
    if (++steps == 4096) {
      run.step(steps);
      steps = 0;
    }
    if (t < tokens.length && !tokens[t].isStar && tokens[t].matches(text[s])) {
      t++;
      s++;
    } else if (t < tokens.length && tokens[t].isStar) {
      starAt = t;
      starFrom = s;
      t++;
    } else if (starAt != null) {
      t = starAt + 1;
      starFrom = starFrom! + 1;
      s = starFrom;
    } else {
      run.step(steps);
      return false;
    }
  }
  run.step(steps + 1);
  return tokens.skip(t).every((token) => token.isStar);
}

List<int> _ends(List<_Token> tokens, List<int> text, int start, _Run run) => [
  for (var end = start; end <= text.length; end++)
    if (_whole(tokens, text.sublist(start, end), run)) end,
];

String _replace(String value, int? kind, List<int> pattern, List<int> replacement, _Run run) {
  if (pattern.isEmpty) return value;
  final tokens = _compile(pattern);
  final text = value.runes.toList();
  final with_ = String.fromCharCodes(replacement);
  if (kind == 0x23) {
    final found = _ends(tokens, text, 0, run);
    return found.isEmpty ? value : with_ + String.fromCharCodes(text.sublist(found.last));
  }
  if (kind == 0x25) {
    for (var start = 0; start <= text.length; start++) {
      if (_whole(tokens, text.sublist(start), run)) return String.fromCharCodes(text.sublist(0, start)) + with_;
    }
    return value;
  }
  final out = StringBuffer();
  var position = 0;
  while (true) {
    (int, int)? match;
    for (var start = position; start < text.length; start++) {
      final found = _ends(tokens, text, start, run);
      if (found.isNotEmpty && found.last > start) {
        match = (start, found.last);
        break;
      }
    }
    if (match == null) break;
    out
      ..write(String.fromCharCodes(text.sublist(position, match.$1)))
      ..write(with_);
    run.fits(out.length);
    position = match.$2;
    if (kind != 0x2f) break;
  }
  out.write(String.fromCharCodes(text.sublist(position)));
  return out.toString();
}

String _trim(String value, int operator, bool longest, List<int> pattern, _Run run) {
  final tokens = _compile(pattern);
  final text = value.runes.toList();
  if (operator == 0x23) {
    final found = _ends(tokens, text, 0, run);
    if (found.isEmpty) return value;
    return String.fromCharCodes(text.sublist(longest ? found.last : found.first));
  }
  final starts = [
    for (var start = 0; start <= text.length; start++)
      if (_whole(tokens, text.sublist(start), run)) start,
  ];
  if (starts.isEmpty) return value;
  return String.fromCharCodes(text.sublist(0, longest ? starts.first : starts.last));
}

// ------------------------------------------------------------------------------------------------------------------
// Validation.
// ------------------------------------------------------------------------------------------------------------------

/// A problem with a manifest: a class and the JSON pointer of the offending value.
final class AosIssue {
  const AosIssue(this.code, this.at);
  final String code, at;
  @override
  String toString() => '$at $code';
}

bool _isInt(Object? value) => value is int;
bool _isSchemaInt(Object? value) => value is int || (value is double && value == value.truncateToDouble());

final _toolName = RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]*$');
final _lowerName = RegExp(r'^[a-z][a-z0-9-]*$');
final _harnessId = RegExp(r'^[a-z0-9][a-z0-9-]*$');
final _digestForm = RegExp(r'^sha256:[0-9a-fA-F]{64}$');
final _versionForm = RegExp(r'^[0-9]{1,9}(?:\.[0-9]{1,9}){0,2}$');
final _environmentName = RegExp(r'^[A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?$');
final _scriptPathForm = RegExp(r'^[A-Za-z0-9_+-][A-Za-z0-9._+-]*(?:/[A-Za-z0-9_+-][A-Za-z0-9._+-]*)*$');
final _commandName = RegExp(r'^[A-Za-z0-9][A-Za-z0-9._+-]*$');
final _headerName = RegExp(r"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$");
final _providerName = RegExp(r'^[a-z][a-z0-9_-]*$');
final _tag = RegExp(r'^[a-z0-9]+(?:-[a-z0-9]+)*$');

bool _isText(Object? value) => value is String && !value.contains('\u0000');
bool _isScriptPath(String text) => _scriptPathForm.hasMatch(text) && isRelativePath(text);

bool _isMcpUrl(String text) {
  final lower = _asciiLower(text);
  final String rest;
  if (lower.startsWith('https://')) {
    rest = text.substring(8);
  } else if (lower.startsWith('http://')) {
    rest = text.substring(7);
  } else {
    return false;
  }
  final end = rest.indexOf(RegExp(r'[/?]'));
  final host = end < 0 ? rest : rest.substring(0, end);
  final tail = end < 0 ? '' : rest.substring(end);
  final space = RegExp(r'\s');
  return host.isNotEmpty && !host.contains(RegExp('[#@]')) && !host.contains(space) && !tail.contains('#') &&
      !tail.contains(space);
}

/// One line of display text: not empty, with no control character, line or paragraph separator, or bidirectional
/// embedding, override or isolate.
bool isDisplayLine(String text) =>
    text.isNotEmpty &&
    !text.runes.any((c) =>
        c <= 0x1f || (c >= 0x7f && c <= 0x9f) || c == 0x2028 || c == 0x2029 || (c >= 0x202a && c <= 0x202e) ||
        (c >= 0x2066 && c <= 0x2069));

bool _isWebUrl(String text) {
  if (!text.startsWith('https://')) return false;
  final rest = text.substring(8);
  bool bad(int c) => c <= 0x20 || (c >= 0x7f && c <= 0x9f);
  final end = rest.indexOf(RegExp('[/?#]'));
  final host = end < 0 ? rest : rest.substring(0, end);
  final tail = end < 0 ? '' : rest.substring(end);
  return host.isNotEmpty && !host.runes.any((c) => bad(c) || c == 0x40 || c == 0x5c) && !tail.runes.any(bad);
}

/// A relative path ending in `.png` or `.webp`, in any ASCII case.
bool isIconPath(String path) {
  final lower = _asciiLower(path);
  return isRelativePath(path) && (lower.endsWith('.png') || lower.endsWith('.webp'));
}

/// The fields of `[about]`, the informational table only a store reads.
const aboutFields = {'summary', 'icon', 'homepage', 'repository', 'license', 'authors', 'tags'};

/// Whether one `[about]` field is valid on its own; a field `[about]` does not define is not. A store leaves out each
/// field this refuses and shows the rest.
bool aboutFieldValid(String field, Object? value) {
  if (!aboutFields.contains(field)) return false;
  final check = _Check()..about(<String, Object?>{field: value}, '');
  return check.issues.isEmpty;
}

/// Whether [text] is an https URL as `[about]` and catalogs take one: a host, and no user information, white space or
/// control character.
bool isWebUrl(String text) => _isWebUrl(text);

bool _isLicense(String text) {
  if (!RegExp(r'^[A-Za-z0-9.+:() -]+$').hasMatch(text)) return false;
  final tokens = text.replaceAll('(', ' ( ').replaceAll(')', ' ) ').split(' ').where((t) => t.isNotEmpty).toList();
  bool id(String token) {
    final body = token.endsWith('+') ? token.substring(0, token.length - 1) : token;
    return body.isNotEmpty && !const {'AND', 'OR', 'WITH'}.contains(body) && RegExp(r'^[A-Za-z0-9.:-]+$').hasMatch(body);
  }

  var at = 0;
  String? peek() => at < tokens.length ? tokens[at] : null;
  late bool Function() or;
  bool withExpression() {
    bool primary;
    if (peek() == '(') {
      at++;
      final inner = or();
      final closed = peek() == ')';
      at++;
      primary = inner && closed;
    } else if (peek() case final token? when id(token)) {
      at++;
      primary = true;
    } else {
      primary = false;
    }
    if (primary && peek() == 'WITH') {
      at++;
      final exception = peek() != null && id(peek()!) && !peek()!.endsWith('+');
      at++;
      return exception;
    }
    return primary;
  }

  bool and() {
    if (!withExpression()) return false;
    while (peek() == 'AND') {
      at++;
      if (!withExpression()) return false;
    }
    return true;
  }

  or = () {
    if (!and()) return false;
    while (peek() == 'OR') {
      at++;
      if (!and()) return false;
    }
    return true;
  };
  return or() && at == tokens.length;
}

bool _deepEqual(Object? a, Object? b) {
  if (a is Map && b is Map) {
    return a.length == b.length && a.keys.every((key) => b.containsKey(key) && _deepEqual(a[key], b[key]));
  }
  if (a is List && b is List) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (!_deepEqual(a[i], b[i])) return false;
    }
    return true;
  }
  return a.runtimeType == b.runtimeType && a == b;
}

final class _Check {
  final issues = <AosIssue>[];

  void schema(String at) => issues.add(AosIssue('schema', at));
  void push(String code, String at) => issues.add(AosIssue(code, at));

  Map<String, Object?>? object(Object? value, String at, Set<String> allowed) {
    if (value is! Map<String, Object?>) {
      schema(at);
      return null;
    }
    if (value.keys.any((key) => !allowed.contains(key))) schema(at);
    return value;
  }

  List<Object?> array(Object? value, String at) {
    if (value is List<Object?>) return value;
    schema(at);
    return const [];
  }

  void stringWhere(Object? value, String at, bool Function(String) rule) {
    if (value is! String || !rule(value)) schema(at);
  }

  void stringsWhere(Object? value, String at, bool Function(String) rule) {
    for (final (index, item) in array(value, at).indexed) {
      stringWhere(item, '$at/$index', rule);
    }
  }

  void boolean(Object? value, String at) {
    if (value is! bool) schema(at);
  }

  void osList(Object? value, String at) => stringsWhere(value, at, _lowerName.hasMatch);

  void timeout(Object? value, String at) {
    if (!_isInt(value) || (value as int) < 0 || value > 2147483647) schema(at);
  }

  void texts(Object? value, String at) {
    for (final (index, item) in array(value, at).indexed) {
      if (!_isText(item)) schema('$at/$index');
    }
  }

  void unique(List<Object?> items, String at) {
    for (var i = 0; i < items.length; i++) {
      for (var j = 0; j < i; j++) {
        if (_deepEqual(items[i], items[j])) {
          schema(at);
          return;
        }
      }
    }
  }

  void envValue(Object? value, String at) {
    if (value is String) {
      if (value.contains('\u0000')) schema(at);
      return;
    }
    if (value is! Map<String, Object?>) {
      schema(at);
      return;
    }
    final sources = [for (final key in const ['value', 'path', 'secret', 'provider']) if (value.containsKey(key)) key];
    final ok = switch (sources) {
      ['value'] => value.keys.every(const {'value', 'if_unset'}.contains) && _isText(value['value']),
      ['path'] => value.keys.every(const {'path', 'if_unset'}.contains) &&
          value['path'] is String && isRelativePath(value['path']! as String),
      ['secret'] => value.keys.every(const {'secret', 'if_unset', 'optional'}.contains) &&
          value['secret'] is String && (value['secret']! as String).isNotEmpty &&
          (!value.containsKey('optional') || value['optional'] is bool),
      ['provider'] => () {
        final provider = value['provider'] is String ? value['provider']! as String : '';
        final key = value['key'];
        final keyOk = key is String && key.isNotEmpty && !key.contains('\u0000') &&
            switch (provider) {
              'file' => isRelativePath(key),
              'env' => isAosEnvName(key),
              _ => true,
            };
        final fieldsOk = value.entries.every((entry) => switch (entry.key) {
          'provider' || 'key' => true,
          'if_unset' || 'optional' => entry.value is bool,
          _ => entry.value is String || entry.value is bool || _isSchemaInt(entry.value),
        });
        return keyOk && _providerName.hasMatch(provider) && fieldsOk;
      }(),
      _ => false,
    };
    if (!ok || (value.containsKey('if_unset') && value['if_unset'] is! bool)) schema(at);
  }

  void envMap(Object? value, String at) {
    if (value is! Map<String, Object?>) {
      schema(at);
      return;
    }
    for (final MapEntry(key: name, value: entry) in value.entries) {
      if (!isAosEnvName(name) || isReservedVariable(name)) schema(at);
      if (isReservedVariable(name)) push('reserved-variable', '$at/$name');
      envValue(entry, '$at/$name');
    }
    for (final name in value.keys) {
      if (value.keys.any((other) => other != name && _equalsIgnoreAsciiCase(other, name))) {
        push('env-name-case-duplicate', '$at/$name');
      }
    }
  }

  void script(Object? value, String at, {required bool network}) {
    final map = object(value, at, {'interpreter', 'path', 'args', 'timeout', 'os', if (network) 'network'});
    if (map == null) return;
    if (!map.containsKey('interpreter') || !map.containsKey('path')) schema(at);
    for (final MapEntry(:key, value: item) in map.entries) {
      final here = '$at/$key';
      switch (key) {
        case 'interpreter':
          stringWhere(item, here, _lowerName.hasMatch);
        case 'path':
          stringWhere(item, here, isRelativePath);
        case 'args':
          texts(item, here);
        case 'timeout':
          timeout(item, here);
        case 'network':
          boolean(item, here);
        case 'os':
          osList(item, here);
      }
    }
  }

  void runtime(Object? value, String at) {
    final map = object(value, at, {'os', 'commands', 'python', 'node'});
    if (map == null) return;
    for (final MapEntry(:key, value: item) in map.entries) {
      final here = '$at/$key';
      switch (key) {
        case 'os':
          osList(item, here);
        case 'commands':
          for (final (index, entry) in array(item, here).indexed) {
            final there = '$here/$index';
            if (entry is String) {
              if (!_commandName.hasMatch(entry)) schema(there);
            } else if (object(entry, there, {'name', 'os'}) case final entry?) {
              if (entry['name'] is! String || !_commandName.hasMatch(entry['name']! as String)) schema(there);
              if (entry.containsKey('os')) osList(entry['os'], '$there/os');
            }
          }
        case 'python' || 'node':
          final python = key == 'python';
          for (final (index, raw) in array(item, here).indexed) {
            final there = '$here/$index';
            final variant = object(raw, there, python
                ? {'version', 'requirements', 'lock', 'lock_digest', 'environment', 'os'}
                : {'version', 'package', 'environment', 'os'});
            if (variant == null) continue;
            if (python &&
                (variant.containsKey('requirements') && variant.containsKey('lock') ||
                    variant.containsKey('lock_digest') && !variant.containsKey('lock'))) {
              schema(there);
            }
            for (final MapEntry(key: field, value: value) in variant.entries) {
              final at = '$there/$field';
              switch (field) {
                case 'version':
                  stringWhere(value, at, _versionForm.hasMatch);
                case 'requirements' || 'lock':
                  stringWhere(value, at, isRelativePath);
                case 'lock_digest':
                  stringWhere(value, at, _digestForm.hasMatch);
                case 'environment':
                  stringWhere(value, at, _environmentName.hasMatch);
                case 'package':
                  stringWhere(value, at, (path) =>
                      isRelativePath(path) && (path == 'package.json' || path.endsWith('/package.json')));
                case 'os':
                  osList(value, at);
              }
            }
          }
      }
    }
  }

  void scripts(Object? value, String at) {
    if (value is! Map<String, Object?>) {
      schema(at);
      return;
    }
    for (final MapEntry(key: name, value: target) in value.entries) {
      if (!RegExp(r'^[A-Za-z0-9][A-Za-z0-9._-]*$').hasMatch(name) || _asciiLower(name).endsWith('.exe')) schema(at);
      final here = '$at/$name';
      switch (target) {
        case false:
          break;
        case final String path:
          final dot = path.lastIndexOf('.');
          final known = dot >= 0 && const {'py', 'sh', 'bash', 'ps1', 'js', 'mjs', 'cjs'}.contains(path.substring(dot + 1));
          if (!known || !_isScriptPath(path)) schema(here);
        case Map<String, Object?>():
          final entry = object(target, here, {'path', 'interpreter', 'os'});
          if (entry != null) {
            if (!entry.containsKey('path') || !entry.containsKey('interpreter')) schema(here);
            if (entry.containsKey('path')) stringWhere(entry['path'], '$here/path', _isScriptPath);
            if (entry.containsKey('interpreter')) stringWhere(entry['interpreter'], '$here/interpreter', _lowerName.hasMatch);
            if (entry.containsKey('os')) osList(entry['os'], '$here/os');
          }
        default:
          schema(here);
      }
    }
  }

  void harness(Object? value, String at) {
    if (value is! Map<String, Object?>) {
      schema(at);
      return;
    }
    for (final MapEntry(key: id, value: hints) in value.entries) {
      if (!_harnessId.hasMatch(id)) schema(at);
      final here = '$at/$id';
      if (hints is! Map<String, Object?>) {
        schema(here);
        continue;
      }
      for (final MapEntry(:key, value: item) in hints.entries) {
        final there = '$here/$key';
        switch (key) {
          case 'context':
            stringsWhere(item, there, isRelativePath);
          case 'skills':
            boolean(item, there);
          case 'model' || 'effort':
            stringWhere(item, there, (text) => text.isNotEmpty);
        }
      }
    }
  }

  void about(Object? value, String at) {
    final map = object(value, at, aboutFields);
    if (map == null) return;
    for (final MapEntry(:key, value: item) in map.entries) {
      final here = '$at/$key';
      switch (key) {
        case 'summary':
          stringWhere(item, here, isDisplayLine);
        case 'icon':
          stringWhere(item, here, isIconPath);
        case 'homepage' || 'repository':
          stringWhere(item, here, _isWebUrl);
        case 'license':
          stringWhere(item, here, _isLicense);
        case 'authors':
          stringsWhere(item, here, isDisplayLine);
        case 'tags':
          stringsWhere(item, here, (tag) => tag.length <= 32 && _tag.hasMatch(tag));
          if (item is List<Object?>) unique(item, here);
      }
    }
  }

  void mcp(Object? value, String at) {
    final map = object(value, at, {'servers', 'skills'});
    if (map == null) return;
    final names = <Object?>[];
    if (map.containsKey('servers')) {
      final here = '$at/servers';
      for (final (index, server) in array(map['servers'], here).indexed) {
        final there = '$here/$index';
        this.server(server, there);
        final name = server is Map ? server['name'] : null;
        if (names.contains(name)) push('mcp-duplicate-server', '$there/name');
        names.add(name);
      }
    }
    if (map.containsKey('skills')) {
      final here = '$at/skills';
      final linked = <Object?>[];
      for (final (index, link) in array(map['skills'], here).indexed) {
        final there = '$here/$index';
        final entry = object(link, there, {'name', 'server'});
        if (entry == null) continue;
        for (final field in const ['name', 'server']) {
          if (entry.containsKey(field)) {
            stringWhere(entry[field], '$there/$field', _toolName.hasMatch);
          } else {
            schema(there);
          }
        }
        if (linked.contains(entry['name'])) push('mcp-duplicate-skill', '$there/name');
        linked.add(entry['name']);
        if (!names.contains(entry['server'])) push('mcp-unknown-server', '$there/server');
      }
    }
  }

  void server(Object? value, String at) {
    final map = object(value, at,
        {'name', 'transport', 'timeout', 'command', 'args', 'env', 'env_passthrough', 'url', 'headers_env'});
    if (map == null) return;
    switch (map['transport']) {
      case 'stdio':
        if (!map.containsKey('command') || map.containsKey('url') || map.containsKey('headers_env')) schema(at);
      case 'streamable-http':
        if (!map.containsKey('url') || const ['command', 'args', 'env', 'env_passthrough'].any(map.containsKey)) {
          schema(at);
        }
      case String():
        schema('$at/transport');
      default:
        schema(at);
    }
    if (!map.containsKey('name')) schema(at);
    for (final MapEntry(:key, value: item) in map.entries) {
      final here = '$at/$key';
      switch (key) {
        case 'name':
          stringWhere(item, here, _toolName.hasMatch);
        case 'timeout':
          if (item is! num || item < 0.001 || item > 2147483647) schema(here);
        case 'command':
          if (!_isText(item) || item == '') {
            schema(here);
          } else if ((item! as String).contains(RegExp(r'[/\\]')) && !isRelativePath(item as String)) {
            push('mcp-command', here);
          }
        case 'args':
          texts(item, here);
        case 'env':
          envMap(item, here);
        case 'env_passthrough':
          stringsWhere(item, here, isAosEnvName);
          if (item is List<Object?>) unique(item, here);
        case 'url':
          if (item is! String || !_isMcpUrl(item)) schema(here);
        case 'headers_env':
          if (item is Map<String, Object?>) {
            for (final MapEntry(key: header, value: variable) in item.entries) {
              if (!_headerName.hasMatch(header)) schema(here);
              stringWhere(variable, '$here/$header', isAosEnvName);
            }
          } else {
            schema(here);
          }
      }
    }
  }
}

/// The problems with a manifest's data model, sorted by pointer; empty when it is valid. With [files], every file a field
/// names must be among them, and `env` may not set a variable a default path addition names.
List<AosIssue> validate(Map<String, Object?> doc, {Set<String>? files}) {
  final check = _Check();
  for (final MapEntry(:key, :value) in doc.entries) {
    final at = '/$key';
    switch (key) {
      case 'name' || 'description':
        if (value is! String) check.schema(at);
      case 'requires':
        for (final (index, item) in check.array(value, at).indexed) {
          final here = '$at/$index';
          if (item is String && _overlayNamePattern.hasMatch(item)) {
            if (!validOverlayName(item)) check.push('reserved-name', here);
          } else {
            check.schema(here);
          }
        }
      case 'routing':
        check.stringsWhere(value, at, (_) => true);
      case 'rules' || 'context':
        check.stringsWhere(value, at, isRelativePath);
      case 'priority':
        if (!_isInt(value)) check.schema(at);
      case 'defaults':
        final map = check.object(value, at,
            {'bin', 'lib', 'env', 'env_scripts', 'setup', 'deps', 'context', 'skills', 'rules'});
        for (final MapEntry(key: name, value: flag) in (map ?? const {}).entries) {
          check.boolean(flag, '$at/$name');
        }
      case 'skills':
        check.stringsWhere(value, at, _toolName.hasMatch);
        if (value is List<Object?>) check.unique(value, at);
      case 'env':
        check.envMap(value, at);
      case 'env_files':
        for (final (index, item) in check.array(value, at).indexed) {
          final here = '$at/$index';
          if (item is String) {
            if (!isRelativePath(item)) check.schema(here);
          } else if (check.object(item, here, {'path', 'format', 'os'}) case final map?) {
            if (map.containsKey('path')) {
              check.stringWhere(map['path'], '$here/path', isRelativePath);
            } else {
              check.schema(here);
            }
            if (map.containsKey('format')) {
              check.stringWhere(map['format'], '$here/format', (format) => format == 'aos' || format == 'dotenv');
            }
            if (map.containsKey('os')) check.osList(map['os'], '$here/os');
          }
        }
      case 'paths':
        for (final (index, item) in check.array(value, at).indexed) {
          final here = '$at/$index';
          final map = check.object(item, here, {'variable', 'path', 'position', 'os'});
          if (map == null) continue;
          if (!map.containsKey('variable') || !map.containsKey('path')) check.schema(here);
          for (final MapEntry(key: field, value: item) in map.entries) {
            final there = '$here/$field';
            switch (field) {
              case 'variable':
                check.stringWhere(item, there, (name) => isAosEnvName(name) && !isReservedVariable(name));
                if (item is String) {
                  if (isReservedVariable(item)) check.push('reserved-variable', there);
                  final env = doc['env'];
                  if (env is Map && env.keys.any((key) => _equalsIgnoreAsciiCase('$key', item))) {
                    check.push('env-path-conflict', there);
                  }
                }
              case 'path':
                check.stringWhere(item, there, isRelativePath);
              case 'position':
                check.stringWhere(item, there, (position) => position == 'prepend' || position == 'append');
              case 'os':
                check.osList(item, there);
            }
          }
        }
      case 'env_scripts' || 'setup':
        for (final (index, item) in check.array(value, at).indexed) {
          check.script(item, '$at/$index', network: key == 'setup');
        }
      case 'runtime':
        check.runtime(value, at);
      case 'scripts':
        check.scripts(value, at);
      case 'mcp':
        check.mcp(value, at);
      case 'harness':
        check.harness(value, at);
      case 'about':
        check.about(value, at);
    }
  }
  if (files != null) {
    for (final (at, path) in _namedFiles(doc)) {
      if (!files.contains(path)) check.push('missing-file', at);
    }
    if (doc['env'] case final Map<String, Object?> env) {
      final added = [for (final addition in defaultPaths(doc, files)) addition.variable];
      for (final name in env.keys) {
        if (added.any((variable) => _equalsIgnoreAsciiCase(variable, name))) check.push('env-path-conflict', '/env/$name');
      }
    }
    if (doc['about'] case {'icon': final String icon} when isIconPath(icon) && !files.contains(icon)) {
      check.push('missing-file', '/about/icon');
    }
    if (doc['skills'] case final List<Object?> skills) {
      for (final (index, name) in skills.indexed) {
        if (name is String && !files.contains('skills/$name/SKILL.md')) check.push('missing-skill', '/skills/$index');
      }
    }
    if (doc['mcp'] case {'skills': final List<Object?> links}) {
      for (final (index, link) in links.indexed) {
        if (link case {'name': final String name} when !files.contains('skills/$name/SKILL.md')) {
          check.push('missing-skill', '/mcp/skills/$index/name');
        }
      }
    }
  }
  final issues = check.issues..sort((a, b) => a.at == b.at ? a.code.compareTo(b.code) : a.at.compareTo(b.at));
  return issues;
}

/// Whether a manifest with these problems still applies: problems under `/about` alone never stop it.
bool applies(List<AosIssue> issues) => issues.every((issue) => issue.at == '/about' || issue.at.startsWith('/about/'));

List<(String, String)> _namedFiles(Map<String, Object?> doc) {
  final out = <(String, String)>[];
  void each(String key, String? field) {
    for (final (index, item) in ((doc[key] is List ? doc[key]! as List : const [])).indexed) {
      if (item is String && field == null) {
        out.add(('/$key/$index', item));
      } else if (item is Map && item[field ?? 'path'] is String) {
        out.add(('/$key/$index/${field ?? 'path'}', item[field ?? 'path'] as String));
      }
    }
  }

  each('rules', null);
  each('context', null);
  each('env_files', null);
  each('env_scripts', 'path');
  each('setup', 'path');
  if (doc['runtime'] case final Map<String, Object?> runtime) {
    for (final (index, variant) in ((runtime['python'] is List ? runtime['python']! as List : const [])).indexed) {
      for (final field in const ['requirements', 'lock']) {
        if (variant is Map && variant[field] is String) out.add(('/runtime/python/$index/$field', variant[field] as String));
      }
    }
    for (final (index, variant) in ((runtime['node'] is List ? runtime['node']! as List : const [])).indexed) {
      if (variant is Map && variant['package'] is String) {
        out.add(('/runtime/node/$index/package', variant['package'] as String));
      }
    }
  }
  if (doc['scripts'] case final Map<String, Object?> scripts) {
    for (final MapEntry(key: name, value: target) in scripts.entries) {
      if (target is String) out.add(('/scripts/$name', target));
      if (target is Map && target['path'] is String) out.add(('/scripts/$name/path', target['path'] as String));
    }
  }
  if (doc['harness'] case final Map<String, Object?> harness) {
    for (final MapEntry(key: id, value: hints) in harness.entries) {
      if (hints is Map && hints['context'] is List) {
        for (final (index, path) in (hints['context'] as List).indexed) {
          if (path is String) out.add(('/harness/$id/context/$index', path));
        }
      }
    }
  }
  return out;
}

// ------------------------------------------------------------------------------------------------------------------
// The tree digest.
// ------------------------------------------------------------------------------------------------------------------

/// What an entry of a tree is.
enum AosEntryKind { file, dir, symlink, special }

/// One entry of a tree: each name from the root as its bytes, its kind, and a file's bytes.
final class AosTreeEntry {
  const AosTreeEntry(this.components, this.kind, [this.data = const []]);

  /// A file of `/`-separated [path] holding [data].
  AosTreeEntry.file(String path, this.data)
      : components = [for (final name in path.split('/')) utf8.encode(name)],
        kind = AosEntryKind.file;

  final List<List<int>> components;
  final AosEntryKind kind;
  final List<int> data;

  /// Whether the entry is under an excluded name, or a `.pyc` file: not part of an overlay.
  bool get isExcluded => _excluded(components, kind);

  /// The `/`-separated path, its bad UTF-8 replaced.
  String get path => components.map((name) => utf8.decode(name, allowMalformed: true)).join('/');
}

bool _excluded(List<List<int>> components, AosEntryKind kind) {
  bool same(List<int> name, String text) {
    final bytes = utf8.encode(text);
    if (bytes.length != name.length) return false;
    for (var i = 0; i < bytes.length; i++) {
      if (bytes[i] != name[i]) return false;
    }
    return true;
  }

  bool endsWithPyc(List<int> name) {
    const pyc = [0x2e, 0x70, 0x79, 0x63];
    if (name.length < 4) return false;
    for (var i = 0; i < 4; i++) {
      if (name[name.length - 4 + i] != pyc[i]) return false;
    }
    return true;
  }

  return components.any((name) => excludedNames.any((excluded) => same(name, excluded))) ||
      kind != AosEntryKind.dir && components.isNotEmpty && endsWithPyc(components.last);
}

/// A tree digest and the lines it is the digest of.
final class AosTreeDigest {
  const AosTreeDigest(this.digest, this.lines);

  /// `sha256:` and 64 lowercase hexadecimal digits.
  final String digest;
  final List<List<int>> lines;
}

/// `sha256:` and the lowercase hexadecimal SHA-256 of [data].
String sha256Digest(List<int> data) => 'sha256:${sha256.convert(data)}';

/// Whether two digests are the same, the hexadecimal digits compared without case; false unless both are well formed.
bool sameDigest(String a, String b) =>
    _digestForm.hasMatch(a) && _digestForm.hasMatch(b) && _asciiLower(a) == _asciiLower(b);

int compareBytes(List<int> a, List<int> b) {
  final length = a.length < b.length ? a.length : b.length;
  for (var i = 0; i < length; i++) {
    if (a[i] != b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

/// The tree digest of [entries]; throws `symlink`, `special-file`, `non-utf8-name`, `invalid-name` or `case-collision`.
AosTreeDigest digestEntries(Iterable<AosTreeEntry> entries) {
  final lines = <(List<int>, List<int>)>[];
  for (final entry in entries) {
    if (entry.kind == AosEntryKind.dir || entry.isExcluded) continue;
    if (entry.kind == AosEntryKind.symlink) throw AosException('symlink', entry.path);
    if (entry.kind == AosEntryKind.special) throw AosException('special-file', entry.path);
    final names = <String>[];
    for (final name in entry.components) {
      try {
        names.add(utf8.decode(name));
      } on FormatException {
        throw AosException('non-utf8-name', entry.path);
      }
    }
    final path = names.join('/');
    if (!isRelativePath(path)) throw AosException('invalid-name', entry.path);
    final pathBytes = utf8.encode(path);
    lines.add((pathBytes, [...pathBytes, 0, ...ascii.encode(sha256Digest(entry.data)), 0x0a]));
  }
  final folded = <String, String>{};
  for (final (pathBytes, _) in lines) {
    final path = utf8.decode(pathBytes);
    final other = folded.putIfAbsent(_asciiLower(path), () => path);
    if (other != path) throw AosException('case-collision', path);
  }
  lines.sort((a, b) => compareBytes(a.$1, b.$1));
  final all = [for (final (_, line) in lines) line];
  return AosTreeDigest(sha256Digest([for (final line in all) ...line]), all);
}

// ------------------------------------------------------------------------------------------------------------------
// Defaults, composition and the environment.
// ------------------------------------------------------------------------------------------------------------------

bool _defaultOn(Map<String, Object?> manifest, String key) =>
    !(manifest['defaults'] is Map && (manifest['defaults']! as Map)[key] == false);

Iterable<String> _members(Set<String> files) => files.where(isRelativePath);

List<String> _strings(Object? value) => [if (value is List) for (final item in value) if (item is String) item];

List<String>? _osOf(Object? entry) => entry is Map && entry.containsKey('os') ? _strings(entry['os']) : null;

/// Whether an entry with [os] applies on [current].
bool osMatches(List<String>? os, String current) =>
    os == null || os.contains(current) ||
    os.contains('unix') && current != 'windows' && const {'macos', 'linux', 'freebsd', 'openbsd', 'netbsd'}.contains(current);

List<String> _merged(List<String> defaults, List<String> declared) =>
    [...defaults.where((path) => !declared.contains(path)), ...declared];

/// An entry of `paths`, declared or by default.
final class PathAddition {
  const PathAddition(this.variable, this.path, {this.append = false, this.os});
  final String variable, path;
  final bool append;
  final List<String>? os;
}

/// The `bin` and `lib` defaults that are on, present and not replaced by a declared entry.
List<PathAddition> defaultPaths(Map<String, Object?> manifest, Set<String> files) {
  final declared = [
    for (final entry in (manifest['paths'] is List ? manifest['paths']! as List : const []))
      if (entry is Map && entry['path'] is String) entry['path'] as String,
  ];
  return [
    for (final (key, variable, directory) in const [
      ('bin', 'PATH', 'bin'), ('lib', 'PYTHONPATH', 'lib/python'), ('lib', 'NODE_PATH', 'lib/node')])
      if (_defaultOn(manifest, key) && _members(files).any((path) => path.startsWith('$directory/')) &&
          !declared.contains(directory))
        PathAddition(variable, directory),
  ];
}

/// The defaults `bin` and `lib`, then `paths`.
List<PathAddition> pathsOf(Map<String, Object?> manifest, Set<String> files) => [
  ...defaultPaths(manifest, files),
  for (final entry in (manifest['paths'] is List ? manifest['paths']! as List : const []))
    if (entry is Map)
      PathAddition('${entry['variable'] ?? ''}', '${entry['path'] ?? ''}',
          append: entry['position'] == 'append', os: _osOf(entry)),
];

List<String> _byBytes(Iterable<String> paths) => paths.toList()..sort((a, b) => compareBytes(utf8.encode(a), utf8.encode(b)));

/// Every `rules/<name>.md` directly in `rules/`, by path bytes, then `rules`.
List<String> rulesOf(Map<String, Object?> manifest, Set<String> files) => _merged(
  _defaultOn(manifest, 'rules') ? _byBytes(_members(files).where((path) => RegExp(r'^rules/[^/]+\.md$').hasMatch(path))) : [],
  _strings(manifest['rules']),
);

/// The root `CONTEXT.md`, then `context`.
List<String> contextOf(Map<String, Object?> manifest, Set<String> files) => _merged(
  _defaultOn(manifest, 'context') && _members(files).contains('CONTEXT.md') ? ['CONTEXT.md'] : [],
  _strings(manifest['context']),
);

/// Every `skills/<name>/SKILL.md` with a skill name, and the skills `skills` lists, by name bytes.
List<String> skillsOf(Map<String, Object?> manifest, Set<String> files) {
  final found = {..._strings(manifest['skills'])};
  if (_defaultOn(manifest, 'skills')) {
    for (final path in _members(files)) {
      final parts = path.split('/');
      if (parts.length == 3 && parts[0] == 'skills' && parts[2] == 'SKILL.md' && _toolName.hasMatch(parts[1])) {
        found.add(parts[1]);
      }
    }
  }
  return _byBytes(found);
}

/// An entry of `env_files`, declared or by default.
final class EnvFile {
  const EnvFile(this.path, {this.dotenv = false, this.os});
  final String path;
  final bool dotenv;
  final List<String>? os;
}

/// The root file `env`, read as `dotenv`, then `env_files`.
List<EnvFile> envFilesOf(Map<String, Object?> manifest, Set<String> files) {
  final declared = [
    for (final entry in (manifest['env_files'] is List ? manifest['env_files']! as List : const []))
      entry is String
          ? EnvFile(entry)
          : EnvFile('${(entry as Map)['path'] ?? ''}', dotenv: entry['format'] == 'dotenv', os: _osOf(entry)),
  ];
  return [
    if (_defaultOn(manifest, 'env') && _members(files).contains('env') && !declared.any((entry) => entry.path == 'env'))
      const EnvFile('env', dotenv: true),
    ...declared,
  ];
}

/// The environment a build starts from, read one variable at a time: its value and whether a placeholder may read it.
typedef AosBase = (String, bool)? Function(String name);

/// An environment being built: variables in the order first set, over a base. On Windows names compare without ASCII
/// case and setting one keeps the name it has. A secret, or a base variable marked unreadable, is never read by a
/// placeholder.
final class AosEnvironment {
  AosEnvironment({this.windows = false, this.base});
  final bool windows;
  final AosBase? base;
  final values = <String, String>{};
  final _secret = <String>{};
  var _extra = <(String, String)>[];

  AosEnvironment copy() => AosEnvironment(windows: windows, base: base)
    ..values.addAll(values)
    .._secret.addAll(_secret)
    .._extra = _extra;

  String? _key(String name) {
    if (values.containsKey(name)) return name;
    if (windows) return values.keys.where((key) => _equalsIgnoreAsciiCase(key, name)).firstOrNull;
    return null;
  }

  /// A variable's value, set or from the base.
  String? get(String name) {
    final key = _key(name);
    return key != null ? values[key] : base?.call(name)?.$1;
  }

  /// Whether a placeholder may not read the variable.
  bool isSecret(String name) {
    final key = _key(name);
    if (key != null) return _secret.contains(key);
    final found = base?.call(name);
    return found != null && !found.$2;
  }

  /// What a placeholder reads; throws `secret-placeholder` for a variable that holds a secret.
  String? lookup(String name) {
    for (final (extra, value) in _extra) {
      if (extra == name || windows && _equalsIgnoreAsciiCase(extra, name)) return value;
    }
    if (isSecret(name)) throw AosException('secret-placeholder', name);
    return get(name);
  }

  /// Sets a variable; [secret] marks a value no placeholder may read.
  void set(String name, String value, {bool secret = false}) {
    final key = _key(name) ?? name;
    values[key] = value;
    if (secret) {
      _secret.add(key);
    } else {
      _secret.remove(key);
    }
  }
}

/// What the consumer has and grants: the inputs of resolving secret references.
final class AosFacts {
  AosFacts(this.os, {Map<String, List<String>>? grants, Map<String, String>? hostEnv, Map<String, String>? secrets,
      this.secretFiles, this.providers = const ['env']})
      : grants = grants ?? {},
        hostEnv = hostEnv ?? {},
        secrets = secrets ?? {};
  final String os;
  final Map<String, List<String>> grants;
  final Map<String, String> hostEnv, secrets;
  final Map<String, String>? secretFiles;
  final List<String> providers;
}

/// One overlay to apply.
final class AosOverlay {
  const AosOverlay({required this.identity, required this.manifest, required this.files, required this.root,
      this.state, required this.read});
  final String identity, root;
  final String? state;
  final Map<String, Object?> manifest;
  final Set<String> files;
  final List<int>? Function(String path) read;
}

/// A weakening, reported with an identifier and a subject, never a value.
final class AosWeakening implements Comparable<AosWeakening> {
  const AosWeakening(this.id, this.subject);
  final String id, subject;
  @override
  int compareTo(AosWeakening other) => id == other.id ? subject.compareTo(other.subject) : id.compareTo(other.id);
  @override
  bool operator ==(Object other) => other is AosWeakening && other.id == id && other.subject == subject;
  @override
  int get hashCode => Object.hash(id, subject);
}

String _resolvePath(String root, String relative, bool windows) {
  final separator = windows ? r'\' : '/';
  return '${root.replaceFirst(RegExp(r'[/\\]+$'), '')}$separator${relative.replaceAll('/', separator)}';
}

String _referenceName(Map<dynamic, dynamic> entry) =>
    entry['secret'] is String ? 'secret:${entry['secret']}' : '${entry['provider'] ?? ''}:${entry['key'] ?? ''}';

bool _isReference(Map<dynamic, dynamic> entry) => entry.containsKey('provider') || entry.containsKey('secret');

bool _granted(Map<dynamic, dynamic> entry, AosFacts facts) {
  final (provider, key) =
      entry['secret'] is String ? ('secret', entry['secret'] as String) : ('${entry['provider'] ?? ''}', '${entry['key'] ?? ''}');
  return facts.grants[provider]?.contains(key) ?? false;
}

String? _secretValue(Map<dynamic, dynamic> entry, AosFacts facts, bool windows) {
  final provider = entry['provider'];
  final key = '${entry['key'] ?? ''}';
  if (provider == 'file' && !isRelativePath(key)) throw AosException('invalid-reference', key);
  if (!_granted(entry, facts)) return null;
  if (entry['secret'] is String) return facts.secrets[entry['secret']];
  if (!facts.providers.contains(provider)) return null;
  switch (provider) {
    case 'env':
      final host = AosEnvironment(windows: windows);
      facts.hostEnv.forEach(host.set);
      return host.get(key);
    case 'file':
      final value = facts.secretFiles?[key];
      if (value == null) return null;
      if (value.endsWith('\r\n')) return value.substring(0, value.length - 2);
      if (value.endsWith('\n')) return value.substring(0, value.length - 1);
      return value;
  }
  return null;
}

AosException _as(AosException error, String name) => AosException(error.code, name);

void _applyMapping(AosEnvironment env, Map<String, Object?> mapping, AosFacts facts, String root,
    List<AosWeakening> weakenings) {
  final windows = env.windows;
  final before = env.copy();
  for (final MapEntry(key: name, value: raw) in mapping.entries) {
    if (isReservedVariable(name)) throw AosException('reserved-variable', name);
    final entry = raw is String ? <String, Object?>{'value': raw} : raw! as Map<String, Object?>;
    if (entry['if_unset'] == true && before.get(name) != null) continue;
    var secret = false;
    final String value;
    if (entry['value'] case final String text) {
      try {
        value = substitute(text, before.lookup);
      } on AosException catch (error) {
        throw _as(error, name);
      }
    } else if (entry['path'] case final String path) {
      value = _resolvePath(root, path, windows);
    } else {
      final found = _secretValue(entry, facts, windows);
      if (found == null) {
        if (entry['optional'] == true) continue;
        throw AosException('unresolved-reference', name);
      }
      secret = true;
      weakenings.add(entry['provider'] == 'env'
          ? AosWeakening('env-passthrough', '${entry['key']}')
          : AosWeakening('secret-read', _referenceName(entry)));
      value = found;
    }
    if (isSensitive(name)) weakenings.add(AosWeakening('env-sensitive', name));
    env.set(name, value, secret: secret);
  }
}

void _applyPaths(AosEnvironment env, List<PathAddition> paths, String root, String os, List<AosWeakening> weakenings) {
  final windows = env.windows;
  final separator = windows ? ';' : ':';
  final grouped = <String, (String, List<PathAddition>)>{};
  for (final entry in paths.where((entry) => osMatches(entry.os, os))) {
    final key = windows ? _asciiUpper(entry.variable) : entry.variable;
    grouped.putIfAbsent(key, () => (entry.variable, [])).$2.add(entry);
  }
  for (final (variable, entries) in grouped.values) {
    if (isSensitive(variable) && !const {'PATH', 'PYTHONPATH', 'NODE_PATH'}.contains(_asciiUpper(variable))) {
      weakenings.add(AosWeakening('env-sensitive', variable));
    }
    final prepend = <String>[], append = <String>[];
    final seen = <String>{};
    for (final entry in entries) {
      final resolved = _resolvePath(root, entry.path, windows);
      if (!seen.add(resolved)) continue;
      (entry.append ? append : prepend).add(resolved);
    }
    final kept = (env.get(variable) ?? '').split(separator).where((part) => part.isNotEmpty && !seen.contains(part));
    final secret = env.isSecret(variable);
    env.set(variable, [...prepend, ...kept, ...append].join(separator), secret: secret);
  }
}

/// Applies one overlay to [env]: its env files that apply on this OS, then `env`, then `paths`, each with its defaults
/// first, every text value substituted as it is set. Env scripts are not run. Throws the applying and substitution
/// classes, the subject being the variable being set or the env file.
void applyOverlay(AosEnvironment env, AosOverlay overlay, AosFacts facts, List<AosWeakening> weakenings) {
  env._extra = [
    ('AOS_OVERLAY_ROOT', overlay.root),
    ('AOS_OVERLAY_NAME', normalizeName(overlay.identity)),
    if (overlay.state case final state?) ('AOS_OVERLAY_STATE', state),
  ];
  try {
    _applyInner(env, overlay, facts, weakenings);
  } finally {
    env._extra = [];
  }
}

void _applyInner(AosEnvironment env, AosOverlay overlay, AosFacts facts, List<AosWeakening> weakenings) {
  final manifest = overlay.manifest;
  final paths = pathsOf(manifest, overlay.files);
  final listed = {for (final entry in paths) _asciiUpper(entry.variable)};
  final own = manifest['env'] is Map ? (manifest['env']! as Map).cast<String, Object?>() : const <String, Object?>{};
  for (final name in own.keys) {
    if (listed.contains(_asciiUpper(name))) throw AosException('env-path-conflict', name);
  }
  for (final file in envFilesOf(manifest, overlay.files)) {
    if (!osMatches(file.os, facts.os)) continue;
    final bytes = overlay.read(file.path) ?? (throw AosException('missing-file', file.path));
    if (file.dotenv) {
      final List<DotenvLine> lines;
      try {
        lines = parseDotenv(bytes);
      } on AosException {
        throw AosException('invalid-env-file', file.path);
      }
      for (final line in lines) {
        if (listed.contains(_asciiUpper(line.name))) throw AosException('env-path-conflict', line.name);
      }
      for (final line in lines) {
        if (isReservedVariable(line.name)) throw AosException('reserved-variable', line.name);
        final String value;
        try {
          value = line.literal ? line.text : substitute(line.text, env.lookup);
        } on AosException catch (error) {
          throw _as(error, line.name);
        }
        if (isSensitive(line.name)) weakenings.add(AosWeakening('env-sensitive', line.name));
        env.set(line.name, value);
      }
    } else {
      final Map<String, Object> mapping;
      try {
        mapping = readDocument(bytes);
      } on AosException {
        throw AosException('invalid-env-file', file.path);
      }
      if (validate({'env': mapping}).isNotEmpty) throw AosException('invalid-env-file', file.path);
      for (final name in mapping.keys) {
        if (listed.contains(_asciiUpper(name))) throw AosException('env-path-conflict', name);
      }
      _applyMapping(env, mapping, facts, overlay.root, weakenings);
    }
  }
  _applyMapping(env, own, facts, overlay.root, weakenings);
  _applyPaths(env, paths, overlay.root, facts.os, weakenings);
}

/// Whether plain `http` to [url] goes to another computer: its host is neither `localhost`, `[::1]` nor an IPv4 address in
/// `127.0.0.0/8`, read as an address, so `127.0.0.1.example.org` is another computer.
bool isInsecureUrl(String url) {
  if (url.length < 7 || _asciiLower(url.substring(0, 7)) != 'http://') return false;
  final authority = url.substring(7).split(RegExp('[/?#]')).first;
  final host = _asciiLower(authority.startsWith('[')
      ? authority.substring(0, authority.contains(']') ? authority.indexOf(']') + 1 : authority.length)
      : authority.split(':').first);
  bool loopbackAddress() {
    final parts = host.split('.');
    if (parts.length != 4) return false;
    for (final part in parts) {
      if (!RegExp(r'^(0|[1-9][0-9]{0,2})$').hasMatch(part) || int.parse(part) > 255) return false;
    }
    return parts.first == '127';
  }

  return host != 'localhost' && host != '[::1]' && !loopbackAddress();
}

/// What an overlay's MCP servers and optional references read that is not granted, sorted; granted reads and plain
/// `http` to another host are added to [weakenings].
List<String> grantsReport(Map<String, Object?> manifest, AosFacts facts, List<AosWeakening> weakenings) {
  final ungranted = <String>{};
  final hostGrants = facts.grants['env'] ?? const [];
  final servers = manifest['mcp'] is Map && (manifest['mcp']! as Map)['servers'] is List
      ? (manifest['mcp']! as Map)['servers'] as List
      : const [];
  for (final server in servers.whereType<Map<dynamic, dynamic>>()) {
    for (final value in (server['env'] is Map ? (server['env'] as Map).values : const [])) {
      if (value is! Map || !_isReference(value)) continue;
      if (!_granted(value, facts)) {
        ungranted.add(_referenceName(value));
      } else {
        weakenings.add(value['provider'] == 'env'
            ? AosWeakening('env-passthrough', '${value['key']}')
            : AosWeakening('secret-read', _referenceName(value)));
      }
    }
    final headers = server['headers_env'] is Map ? [for (final v in (server['headers_env'] as Map).values) '$v'] : <String>[];
    for (final name in [..._strings(server['env_passthrough']), ...headers]) {
      if (hostGrants.contains(name)) {
        weakenings.add(AosWeakening('env-passthrough', name));
      } else {
        ungranted.add('env:$name');
      }
    }
    if (server['url'] case final String url when isInsecureUrl(url)) {
      weakenings.add(AosWeakening('insecure-transport', '${server['name'] ?? ''}'));
    }
  }
  for (final entry in (manifest['env'] is Map ? (manifest['env']! as Map).values : const [])) {
    if (entry is Map && _isReference(entry) && entry['optional'] == true && !_granted(entry, facts)) {
      ungranted.add(_referenceName(entry));
    }
  }
  return _byBytes(ungranted);
}

/// A cycle in `requires`, as its chain.
final class AosCycle implements Exception {
  const AosCycle(this.chain);
  final List<String> chain;
  @override
  String toString() => 'cycle: ${chain.join(' -> ')}';
}

/// Resolves `requires`: the visiting order and each missing requirement as (the overlay that requires it, the name).
/// [overlays] maps a normalized identity to its manifest. Throws [AosCycle].
(List<String>, List<(String, String)>) resolveRequires(Map<String, Map<String, Object?>> overlays, List<String> requested) {
  final order = <String>[];
  final missing = <(String, String)>[];
  void visit(String name, List<String> chain) {
    if (order.contains(name)) return;
    final at = chain.indexOf(name);
    if (at >= 0) throw AosCycle([...chain.sublist(at), name]);
    final manifest = overlays[name];
    if (manifest == null) {
      missing.add((chain.isEmpty ? '' : chain.last, name));
      return;
    }
    for (final required in _strings(manifest['requires'])) {
      visit(normalizeName(required), [...chain, name]);
    }
    order.add(name);
  }

  for (final name in requested) {
    visit(normalizeName(name), []);
  }
  return (order, missing);
}

/// The composition order: ascending `(priority, name, identity)`, strings by code point.
List<String> compositionOrder(Map<String, Map<String, Object?>> overlays, List<String> active) {
  (int, List<int>, List<int>) key(String identity) {
    final manifest = overlays[identity] ?? const {};
    final priority = manifest['priority'] is int ? manifest['priority']! as int : 500;
    final name = manifest['name'] is String && (manifest['name']! as String).isNotEmpty ? manifest['name']! as String : identity;
    return (priority, utf8.encode(name), utf8.encode(identity));
  }

  return [...active]..sort((a, b) {
    final (pa, na, ia) = key(a);
    final (pb, nb, ib) = key(b);
    if (pa != pb) return pa.compareTo(pb);
    final byName = compareBytes(na, nb);
    return byName != 0 ? byName : compareBytes(ia, ib);
  });
}

// ------------------------------------------------------------------------------------------------------------------
// Icons.
// ------------------------------------------------------------------------------------------------------------------

/// An icon's limits: settings with the specification's recommended defaults and no floor.
final class IconLimits {
  const IconLimits({this.maxBytes = 512 * 1024, this.maxWidth = 1024, this.maxHeight = 1024});
  final int maxBytes, maxWidth, maxHeight;
}

/// What an icon's header says: its format and dimensions.
final class IconHeader {
  const IconHeader(this.format, this.width, this.height);
  final String format;
  final int width, height;
}

/// Checks an icon named [name] holding [bytes] before anything decodes it: its extension, size, signature and the
/// dimensions its header gives. Throws `icon-format`, `icon-too-large`, `icon-mismatch`, `icon-unreadable` or
/// `icon-dimensions`. SVG and every format but PNG and WebP are refused.
IconHeader checkIcon(String name, List<int> bytes, {IconLimits limits = const IconLimits()}) {
  final lower = _asciiLower(name);
  final String named;
  if (lower.endsWith('.png')) {
    named = 'png';
  } else if (lower.endsWith('.webp')) {
    named = 'webp';
  } else {
    throw const AosException('icon-format');
  }
  if (bytes.length > limits.maxBytes) throw const AosException('icon-too-large');
  bool at(int offset, List<int> expected) {
    if (bytes.length < offset + expected.length) return false;
    for (var i = 0; i < expected.length; i++) {
      if (bytes[offset + i] != expected[i]) return false;
    }
    return true;
  }

  const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  final isPng = at(0, png);
  final isWebp = at(0, ascii.encode('RIFF')) && at(8, ascii.encode('WEBP'));
  if (!isPng && !isWebp) throw const AosException('icon-format');
  final found = isPng ? 'png' : 'webp';
  if (found != named) throw const AosException('icon-mismatch');
  const unreadable = AosException('icon-unreadable');
  final data = ByteData.sublistView(Uint8List.fromList(bytes));
  int width, height;
  if (isPng) {
    if (bytes.length < 24 || data.getUint32(8) != 13 || !at(12, ascii.encode('IHDR'))) throw unreadable;
    width = data.getUint32(16);
    height = data.getUint32(20);
  } else {
    if (bytes.length < 20 || data.getUint32(4, Endian.little) + 8 > bytes.length) throw unreadable;
    final size = data.getUint32(16, Endian.little);
    if (20 + size > bytes.length) throw unreadable;
    if (at(12, ascii.encode('VP8 '))) {
      if (size < 10 || !at(23, const [0x9d, 0x01, 0x2a])) throw unreadable;
      width = data.getUint16(26, Endian.little) & 0x3fff;
      height = data.getUint16(28, Endian.little) & 0x3fff;
    } else if (at(12, ascii.encode('VP8L'))) {
      if (size < 5 || bytes[20] != 0x2f) throw unreadable;
      final bits = data.getUint32(21, Endian.little);
      width = (bits & 0x3fff) + 1;
      height = ((bits >> 14) & 0x3fff) + 1;
    } else if (at(12, ascii.encode('VP8X'))) {
      if (size < 10) throw unreadable;
      width = (bytes[24] | bytes[25] << 8 | bytes[26] << 16) + 1;
      height = (bytes[27] | bytes[28] << 8 | bytes[29] << 16) + 1;
    } else {
      throw unreadable;
    }
  }
  if (width == 0 || height == 0) throw unreadable;
  if (width > limits.maxWidth || height > limits.maxHeight) throw const AosException('icon-dimensions');
  return IconHeader(found, width, height);
}
