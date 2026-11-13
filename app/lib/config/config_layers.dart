import 'dart:convert';

import 'package:toml/toml.dart';
import 'package:yaml/yaml.dart';

/// The formats canoa.cfg may be written in, chosen as anycfg chooses: by a file's extension when it names one (`.toml`,
/// `.yaml`, `.yml`, `.json`), and for `.cfg` or no name by the first line (`#!yaml`, `#!json`, `#!toml`), none meaning TOML.
enum ConfigFormat {
  toml,
  yaml,
  json;

  static ConfigFormat? named(String word) => switch (word.replaceFirst(RegExp(r'^\.+'), '').toLowerCase()) {
    'toml' => toml,
    'yaml' || 'yml' => yaml,
    'json' => json,
    _ => null,
  };
}

/// A canoa.cfg read: its format, whether its first line names it (written back the same way), and its tables.
typedef ConfigText = ({ConfigFormat format, bool marked, Map<String, dynamic> data});

const _readsFormats = 'toml, yaml, json';

/// Reads [text] as the file called [name] (or one with no name). Throws [FormatException] naming no value.
ConfigText decodeConfig(String text, {String? name}) {
  if (text.startsWith('\u{feff}')) text = text.substring(1);
  final extension = _extensionOf(name);
  ConfigFormat? format;
  var marked = false;
  var body = text;
  if (extension != null && extension != 'cfg') {
    format = ConfigFormat.named(extension);
    if (format == null) throw FormatException('canoa.cfg: the extension .$extension is not one this app reads; it reads $_readsFormats');
  } else {
    final lineEnd = text.indexOf('\n');
    final first = lineEnd < 0 ? text : text.substring(0, lineEnd);
    if (first.startsWith('#!')) {
      final words = first.substring(2).trim().split(RegExp(r'\s+')).where((word) => word.isNotEmpty).toList();
      final last = words.isEmpty ? '' : words.last.split(RegExp(r'[/\\]')).last;
      format = ConfigFormat.named(last);
      if (format == null) {
        throw const FormatException('canoa.cfg: the first line names a format this app does not read; it reads $_readsFormats');
      }
      marked = true;
      body = lineEnd < 0 ? '' : text.substring(lineEnd + 1);
    } else {
      format = ConfigFormat.toml;
    }
  }
  final Object? raw;
  try {
    raw = switch (format) {
      ConfigFormat.toml => TomlDocument.parse(body).toMap(),
      ConfigFormat.yaml => body.trim().isEmpty ? <String, dynamic>{} : loadYaml(body),
      ConfigFormat.json => _json(body),
    };
  } on YamlException catch (error) {
    // The exception's own text quotes the line; only its position is told.
    throw FormatException('canoa.cfg: not valid YAML${error.span == null ? '' : ' at line ${error.span!.start.line + 1}'}');
  } on FormatException {
    rethrow;
  } on Object {
    throw FormatException('canoa.cfg: not valid ${format.name.toUpperCase()}');
  }
  final data = _plain(raw, true);
  if (data is! Map<String, dynamic>) throw const FormatException('canoa.cfg: the top level is not a table of settings');
  return (format: format, marked: marked, data: data);
}

/// JSON's own message quotes the text around the error, which may be a secret.
Object? _json(String body) {
  try {
    return jsonDecode(body);
  } on FormatException {
    throw const FormatException('canoa.cfg: not valid JSON');
  }
}

String? _extensionOf(String? name) {
  if (name == null) return null;
  final base = name.split(RegExp(r'[/\\]')).last;
  final dot = base.lastIndexOf('.');
  if (dot <= 0 || dot == base.length - 1) return null;
  return base.substring(dot + 1).toLowerCase();
}

/// Plain maps and lists. YAML and JSON have a null that TOML lacks: a key holding one is not set, one in a list is refused.
Object? _plain(Object? value, bool root) {
  switch (value) {
    case final Map<dynamic, dynamic> map:
      final out = <String, dynamic>{};
      for (final MapEntry(:key, :value) in map.entries) {
        final inner = _plain(value, false);
        if (inner == null) continue;
        out['$key'] = inner;
      }
      return out;
    case final List<dynamic> list:
      return [
        for (final item in list)
          _plain(item, false) ?? (throw const FormatException('canoa.cfg: a list holds a null, which has no place in canoa.cfg')),
      ];
    case null:
      return root ? <String, dynamic>{} : null;
    default:
      return value;
  }
}

/// [data] written as the format and with the marker of a file read as [format] and [marked].
String encodeConfig(Map<String, dynamic> data, ConfigFormat format, {required bool marked}) {
  final marker = marked ? '#!${format.name}\n' : '';
  return switch (format) {
    ConfigFormat.toml => '$marker${TomlDocument.fromMap(data)}',
    ConfigFormat.json => '$marker${const JsonEncoder.withIndent('  ').convert(data)}\n',
    ConfigFormat.yaml => '$marker${_yamlLines(data, 0).join('\n')}\n',
  };
}

final _plainKey = RegExp(r'^[A-Za-z_][A-Za-z0-9_-]*$');
const _reserved = {'true', 'false', 'null', 'yes', 'no', 'on', 'off', 'y', 'n'};

String _yamlKey(String key) => _plainKey.hasMatch(key) && !_reserved.contains(key.toLowerCase()) ? key : jsonEncode(key);

String _yamlScalar(Object? value) => switch (value) {
  final String text => jsonEncode(text),
  final double number when number.isNaN => '.nan',
  final double number when number.isInfinite => number.isNegative ? '-.inf' : '.inf',
  _ => '$value',
};

List<String> _yamlLines(Map<dynamic, dynamic> map, int indent) {
  final pad = ' ' * indent;
  final lines = <String>[];
  for (final MapEntry(:key, :value) in map.entries) {
    final name = _yamlKey('$key');
    switch (value) {
      case final Map<dynamic, dynamic> inner when inner.isEmpty:
        lines.add('$pad$name: {}');
      case final Map<dynamic, dynamic> inner:
        lines.add('$pad$name:');
        lines.addAll(_yamlLines(inner, indent + 2));
      case final List<dynamic> list when list.isEmpty:
        lines.add('$pad$name: []');
      case final List<dynamic> list:
        lines.add('$pad$name:');
        for (final item in list) {
          if (item is Map && item.isNotEmpty) {
            final inner = _yamlLines(item, indent + 4);
            lines.add('$pad  - ${inner.first.substring(indent + 4)}');
            lines.addAll(inner.skip(1));
          } else if (item is Map) {
            lines.add('$pad  - {}');
          } else if (item is List) {
            lines.add('$pad  - ${jsonEncode(item)}');
          } else {
            lines.add('$pad  - ${_yamlScalar(item)}');
          }
        }
      default:
        lines.add('$pad$name: ${_yamlScalar(value)}');
    }
  }
  return lines;
}

/// The tables below which each key is one entry that a higher layer replaces whole. `*` is any key. The host merges by the
/// same list (`ENTRIES` in canoa-host).
const configEntryPaths = <List<String>>[
  ['host', 'agents', '*'],
  ['host', 'providers', '*'],
  ['host', 'tools', '*'],
  ['host', 'profiles', '*'],
  ['host', 'accounts', '*'],
  ['agents', '*'],
  ['profile', 'mcp', '*'],
  ['profile', 'skills', '*'],
  ['profile', 'env', '*'],
  ['overlays', '*'],
  ['catalogs', '*'],
];

bool _isEntry(List<String> path) => configEntryPaths.any(
  (pattern) => pattern.length == path.length && [for (var i = 0; i < path.length; i++) pattern[i] == '*' || pattern[i] == path[i]].every((same) => same),
);

Object? _copy(Object? value) => switch (value) {
  final Map<dynamic, dynamic> map => {for (final MapEntry(:key, :value) in map.entries) '$key': _copy(value)},
  final List<dynamic> list => [for (final item in list) _copy(item)],
  _ => value,
};

bool _same(Object? a, Object? b) {
  if (a is Map && b is Map) {
    return a.length == b.length && a.keys.every((key) => b.containsKey(key) && _same(a[key], b[key]));
  }
  if (a is List && b is List) {
    return a.length == b.length && [for (var i = 0; i < a.length; i++) _same(a[i], b[i])].every((same) => same);
  }
  return a == b;
}

/// [above] merged over [below], key by key: a key above replaces the same key below, tables merge, and the entries of
/// [configEntryPaths] and every list are replaced whole.
Map<String, dynamic> mergeConfig(Map<String, dynamic> below, Map<String, dynamic> above) {
  Map<String, dynamic> tables(List<String> path, Map<String, dynamic> below, Map<String, dynamic> above) {
    final out = _copy(below)! as Map<String, dynamic>;
    for (final MapEntry(:key, :value) in above.entries) {
      final at = [...path, key];
      final lower = out[key];
      out[key] = value is Map<String, dynamic> && lower is Map<String, dynamic> && !_isEntry(at) ? tables(at, lower, value) : _copy(value);
    }
    return out;
  }

  return tables(const [], below, above);
}

/// The person's own file after a change: [merged], what the app now holds, less every key that the layers [lower] already give
/// the same and the person's file [own] did not have, and every one [defaults] gives where no layer does. So a value from the installed or the system's file is never copied
/// into it, and a key the person set stays.
Map<String, dynamic> ownDocument(
    Map<String, dynamic> merged, Map<String, dynamic> lower, Map<String, dynamic> own, Map<String, dynamic> defaults) {
  Map<String, dynamic> tables(
      List<String> path, Map<String, dynamic> merged, Map<String, dynamic>? lower, Map<String, dynamic>? own, Map<String, dynamic>? defaults) {
    final out = <String, dynamic>{};
    for (final MapEntry(:key, :value) in merged.entries) {
      final at = [...path, key];
      final below = lower?[key];
      final had = own?[key];
      final byDefault = defaults?[key];
      final held = own?.containsKey(key) ?? false;
      if (value is Map<String, dynamic> && !_isEntry(at)) {
        final inner = tables(at, value, below is Map<String, dynamic> ? below : null, had is Map<String, dynamic> ? had : null,
            byDefault is Map<String, dynamic> ? byDefault : null);
        if (inner.isNotEmpty || held) out[key] = inner;
      } else if (held ||
          ((lower?.containsKey(key) ?? false) ? !_same(below, value) : !(defaults?.containsKey(key) ?? false) || !_same(byDefault, value))) {
        out[key] = _copy(value);
      }
    }
    return out;
  }

  return tables(const [], merged, lower, own, defaults);
}

/// Where a value of the merged configuration comes from.
enum ConfigOrigin {
  installed('the installed file'),
  system('the system file'),
  user('your user file'),
  listed('a file CANOA_CONFIG lists'),
  own('your file');

  const ConfigOrigin(this.words);

  /// How Settings names it.
  final String words;
}

/// One layer of canoa.cfg, with where it came from.
typedef ConfigLayer = ({ConfigOrigin origin, Map<String, dynamic> data});

/// The highest of [layers] (lowest first) that holds [path], an entry counting as one value, or null when none does.
ConfigOrigin? originOf(List<ConfigLayer> layers, List<String> path) {
  var cut = path;
  for (var length = 1; length <= path.length; length++) {
    if (_isEntry(path.sublist(0, length))) {
      cut = path.sublist(0, length);
      break;
    }
  }
  for (final layer in layers.reversed) {
    Object? at = layer.data;
    for (final key in cut) {
      if (at is Map<String, dynamic> && at.containsKey(key)) {
        at = at[key];
      } else {
        at = _missing;
        break;
      }
    }
    if (!identical(at, _missing)) return layer.origin;
  }
  return null;
}

final Object _missing = Object();

/// [data] without the key at [path], whatever it held, and without a table that leaves empty.
Map<String, dynamic> withoutKey(Map<String, dynamic> data, List<String> path) {
  final out = _copy(data)! as Map<String, dynamic>;
  final chain = <Map<String, dynamic>>[out];
  for (var i = 0; i < path.length - 1; i++) {
    final next = chain.last[path[i]];
    if (next is! Map<String, dynamic>) return out;
    chain.add(next);
  }
  chain.last.remove(path.last);
  for (var i = chain.length - 1; i > 0 && chain[i].isEmpty; i--) {
    chain[i - 1].remove(path[i - 1]);
  }
  return out;
}

/// `--config` and `CANOA_CONFIG`: a list of files split at `;` on Windows and `:` elsewhere, with no quoting; a leading `+`
/// puts them over the installed, system and user files, and without it they are the only ones read.
({bool merge, List<String> entries}) parseConfigList(String value, {required bool windows}) {
  final plus = value.startsWith('+');
  final separator = windows ? ';' : ':';
  final entries = (plus ? value.substring(1) : value).split(separator);
  for (var i = 0; i < entries.length; i++) {
    if (entries[i].isEmpty) throw FormatException('canoa.cfg entry ${i + 1} is empty');
  }
  return (merge: plus, entries: entries);
}

/// A file canoa.cfg may be read from, and what layer it is.
typedef ConfigFile = ({ConfigOrigin origin, String path});

/// The files canoa.cfg is read from, lowest layer first, and the one the app writes. The host finds them the same way
/// (`Layers` in canoa-host); each of the two is tested against the same cases.
final class ConfigFiles {
  const ConfigFiles({required this.files, required this.own, required this.windows, this.spec});

  /// Whether paths are Windows paths.
  final bool windows;

  /// The files, lowest first, whether or not they exist.
  final List<ConfigFile> files;

  /// The person's own file, the last: the user's, or the last one listed. The only file the app writes.
  final String own;

  /// The `CANOA_CONFIG` value, when it named files.
  final String? spec;

  /// Finds the files from [env], for a program in [programDir]. [exists] says whether a listed file is there: one that is not
  /// is refused, naming its entry, and so is an empty entry. Throws [FormatException].
  static ConfigFiles resolve({
    required Map<String, String> env,
    required bool windows,
    required String programDir,
    required bool Function(String path) exists,
    String? explicit,
  }) {
    final sep = windows ? r'\' : '/';
    String join(String dir, String name) => dir.endsWith(sep) ? '$dir$name' : '$dir$sep$name';
    String? var_(String name) => (env[name] ?? '').isEmpty ? null : env[name];
    final home = windows ? var_('USERPROFILE') : var_('HOME');
    String? user;
    if (windows) {
      final base = var_('APPDATA') ?? (home == null ? null : join(join(home, 'AppData'), 'Roaming'));
      user = base == null ? null : join(join(base, 'Canoa'), 'canoa.cfg');
    } else {
      final xdg = var_('XDG_CONFIG_HOME');
      final base = xdg != null && xdg.startsWith('/') ? xdg : (home == null ? null : join(home, '.config'));
      user = base == null ? null : join(join(base, 'Canoa'), 'canoa.cfg');
    }
    final system = windows
        ? join(join(var_('PROGRAMDATA') ?? r'C:\ProgramData', 'Canoa'), 'canoa.cfg')
        : '/etc/Canoa/canoa.cfg';
    final installedDir = var_('CANOA_INSTALLED_DIR') ?? programDir;
    final spec = (explicit ?? var_('CANOA_CONFIG'));
    final lower = <ConfigFile>[
      (origin: ConfigOrigin.installed, path: join(installedDir, 'canoa.cfg')),
      (origin: ConfigOrigin.system, path: system),
    ];
    final files = <ConfigFile>[];
    String own;
    if (spec == null || spec.isEmpty) {
      if (user == null) throw const FormatException('no canoa.cfg: give CANOA_CONFIG');
      files
        ..addAll(lower)
        ..add((origin: ConfigOrigin.own, path: user));
      own = user;
    } else {
      final list = parseConfigList(spec, windows: windows);
      for (var i = 0; i < list.entries.length; i++) {
        if (!exists(list.entries[i])) {
          throw FormatException('canoa.cfg entry ${i + 1} (${list.entries[i]}) is not a file');
        }
      }
      if (list.merge) {
        files.addAll(lower);
        if (user != null) files.add((origin: ConfigOrigin.user, path: user));
      }
      for (var i = 0; i < list.entries.length; i++) {
        files.add((origin: i == list.entries.length - 1 ? ConfigOrigin.own : ConfigOrigin.listed, path: list.entries[i]));
      }
      own = list.entries.last;
    }
    // A file named twice is read once, at its highest place.
    final kept = <ConfigFile>[];
    for (final file in files.reversed) {
      if (!kept.any((other) => other.path == file.path)) kept.add(file);
    }
    return ConfigFiles(files: kept.reversed.toList(), own: own, windows: windows, spec: spec == null || spec.isEmpty ? null : spec);
  }

  /// The arguments that make `canoa` read these same files: none by default (it finds them as the app did), the value of
  /// `CANOA_CONFIG` when that named them.
  List<String> get hostArgs => spec == null ? const [] : ['--config', spec!];

  /// What `canoa check --config` takes to check the file [candidate] as the person's own, in place of the one the app writes.
  String checkSpec(String candidate) {
    final value = spec;
    if (value == null) return '+$candidate';
    final plus = value.startsWith('+');
    final entries = parseConfigList(value, windows: windows).entries;
    return '${plus ? '+' : ''}${[...entries.take(entries.length - 1), candidate].join(windows ? ';' : ':')}';
  }
}
