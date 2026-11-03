/// Profiles: the environment, initial context, skills, MCP servers and defaults an agent is launched with, in layers.
/// canoa-host reads the same schema (`canoa_host::profile`); `crates/canoa-host/tests/vectors/profile.json` holds both.
library;

import 'package:toml/toml.dart';

/// The longest skill, MCP server, header or overlay name.
const int maxProfileName = 64;

/// The longest environment variable name.
const int maxEnvName = 128;

/// The largest file a `from_file` reference reads, in bytes.
const int maxSecretFile = 1024 * 1024;

/// Environment variables an overlay or a project may not set, compared without case: they decide what a process runs or
/// loads, where an agent's own requests go, where a harness keeps its configuration, or which certificates are trusted.
const reservedEnv = [
  'PATH', 'PATHEXT', 'HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'TMP',
  'TEMP', 'TMPDIR', 'COMSPEC', 'SHELL', 'BASH_ENV', 'ENV', 'PROMPT_COMMAND', 'NODE_OPTIONS', 'NODE_PATH',
  'NODE_EXTRA_CA_CERTS', 'PERL5OPT', 'PERL5LIB', 'RUBYOPT', 'RUBYLIB', 'JAVA_TOOL_OPTIONS', '_JAVA_OPTIONS',
  'JDK_JAVA_OPTIONS', 'SSL_CERT_FILE', 'SSL_CERT_DIR', 'REQUESTS_CA_BUNDLE', 'CURL_CA_BUNDLE', 'CLAUDE_CONFIG_DIR',
  'CLAUDE_CODE_EXECUTABLE', 'CODEX_HOME', 'CODEX_CONFIG', 'GEMINI_CLI_HOME', 'GEMINI_SYSTEM_MD', 'PI_CODING_AGENT_DIR',
  'PI_ACP_PI_COMMAND', 'GROK_HOME', 'OPENCODE_CONFIG', 'OPENCODE_CONFIG_DIR', 'OPENCODE_CONFIG_CONTENT',
];

/// Prefixes of [reservedEnv]'s kind, compared without case.
const reservedEnvPrefixes = ['LD_', 'DYLD_', 'PYTHON', 'GIT_', 'NPM_CONFIG_', 'CANOA_', 'COQUI_'];

/// Suffixes of [reservedEnv]'s kind, compared without case.
const reservedEnvSuffixes = ['_BASE_URL', '_PROXY'];

/// Whether an overlay or a project may not set [name].
bool isReservedEnv(String name) {
  final upper = name.toUpperCase();
  return reservedEnv.contains(upper) || reservedEnvPrefixes.any(upper.startsWith) ||
      reservedEnvSuffixes.any(upper.endsWith);
}

/// Where a layer comes from, lowest first.
enum LayerKind {
  builtIn('built-in'),
  overlay('overlay'),
  user('user'),
  project('project'),
  agent('agent'),
  session('session');

  const LayerKind(this.id);

  /// The kind as the view writes it.
  final String id;

  /// Whether the layer comes from elsewhere, so its paths must stay inside its own directory.
  bool get confined => this == overlay || this == project;

  static LayerKind? named(String id) => values.where((kind) => kind.id == id).firstOrNull;
}

/// One layer's name in the view: `user`, or `overlay:team` for a named one.
final class LayerId {
  const LayerId(this.kind, [this.name = '']);
  final LayerKind kind;
  final String name;

  @override
  String toString() => name.isEmpty ? kind.id : '${kind.id}:$name';

  @override
  bool operator ==(Object other) => other is LayerId && other.kind == kind && other.name == name;

  @override
  int get hashCode => Object.hash(kind, name);
}

/// A value an environment variable or a header takes. References are resolved only when a session starts.
sealed class EnvValue {
  const EnvValue();
  Map<String, Object?> toJson();
}

/// Written in the profile, for what is not secret. Its text is never in [toString].
final class EnvLiteral extends EnvValue {
  const EnvLiteral(this.value);
  final String value;
  @override
  Map<String, Object?> toJson() => {'value': value};
  @override
  String toString() => 'EnvLiteral(..)';
}

/// The value of this process's environment variable [name].
final class EnvFromEnv extends EnvValue {
  const EnvFromEnv(this.name);
  final String name;
  @override
  Map<String, Object?> toJson() => {'from_env': name};
  @override
  String toString() => 'EnvFromEnv($name)';
}

/// The contents of a file, relative to its layer's directory, without one final line break.
final class EnvFromFile extends EnvValue {
  const EnvFromFile(this.path);
  final String path;
  @override
  Map<String, Object?> toJson() => {'from_file': path};
  @override
  String toString() => 'EnvFromFile($path)';
}

/// The initial context: [text], or a [file] relative to its layer's directory. Exactly one is set.
final class ProfileContext {
  const ProfileContext.text(String this.text) : file = null;
  const ProfileContext.file(String this.file) : text = null;
  final String? text, file;
  Map<String, Object?> toJson() => text != null ? {'text': text} : {'file': file};
}

/// A skill: a directory holding a `SKILL.md`, relative to its layer's directory.
final class Skill {
  const Skill(this.path);
  final String path;
  Map<String, Object?> toJson() => {'path': path};
}

/// An MCP server, in the forms ACP's `mcpServers` takes.
sealed class McpServer {
  const McpServer();
  Map<String, Object?> toJson();
}

/// A program the agent starts, speaking MCP on its standard streams.
final class McpStdio extends McpServer {
  const McpStdio({required this.command, this.args = const [], this.env = const {}});
  final String command;
  final List<String> args;
  final Map<String, EnvValue> env;
  @override
  Map<String, Object?> toJson() => {
    'type': 'stdio',
    'command': command,
    'args': args,
    'env': {for (final MapEntry(:key, :value) in env.entries) key: value.toJson()},
  };
}

/// Streamable HTTP, or with [sse] HTTP with server-sent events.
final class McpHttp extends McpServer {
  const McpHttp({required this.url, this.headers = const {}, this.sse = false});
  final String url;
  final Map<String, EnvValue> headers;
  final bool sse;
  @override
  Map<String, Object?> toJson() => {
    'type': sse ? 'sse' : 'http',
    'url': url,
    'headers': {for (final MapEntry(:key, :value) in headers.entries) key: value.toJson()},
  };
}

/// Why a profile document was refused, or a reference could not be resolved. Never quotes a value.
final class ProfileException implements Exception {
  const ProfileException(this.code, {this.key, this.line});

  /// `parse`, `name`, `path`, `form`, `value` or `resolve`.
  final String code;

  /// The key, such as `env.GITHUB_TOKEN`; for a name, the table holding it. None for `parse`.
  final String? key;

  /// The 1-based line of a parse error, when the parser gives one.
  final int? line;

  @override
  String toString() => switch (code) {
    'parse' => 'The profile is not TOML of a profile\'s shape${line == null ? '' : ' at line $line'}',
    'name' => 'Profile $key: a name has characters or a length the schema refuses',
    'path' => 'Profile $key: a path is empty, or leaves the directory of a layer that comes from elsewhere',
    'form' => 'Profile $key: takes exactly one of its forms',
    'value' => 'Profile $key: a value is empty, holds a control character, or is not an HTTP URL',
    'reserved' => 'Profile $key: a layer from elsewhere may not set this variable',
    _ => 'Profile $key: the reference cannot be resolved',
  };
}

/// One profile document: what one layer sets and removes. In [env], [skills] and [mcp], null removes a lower layer's.
final class Profile {
  const Profile({
    this.model,
    this.effort,
    this.mode,
    this.context,
    this.unset = const [],
    this.shareEnv = const [],
    this.env = const {},
    this.skills = const {},
    this.mcp = const {},
  });

  final String? model, effort, mode;
  final ProfileContext? context;

  /// `model`, `effort`, `mode` or `context`: values of lower layers this one takes back.
  final List<String> unset;

  /// Variables of this process that overlays and projects may read with `from_env`; only in the person's own layers.
  final List<String> shareEnv;
  final Map<String, EnvValue?> env;
  final Map<String, Skill?> skills;
  final Map<String, McpServer?> mcp;

  static const scalars = ['model', 'effort', 'mode', 'context'];

  bool get isEmpty =>
      model == null && effort == null && mode == null && context == null && unset.isEmpty && shareEnv.isEmpty && env.isEmpty &&
      skills.isEmpty && mcp.isEmpty;

  /// Parses a profile document for a layer of [kind]. Touches no file.
  static Profile parse(String text, LayerKind kind) {
    final Map<String, dynamic> map;
    try {
      map = TomlDocument.parse(text.startsWith('\u{feff}') ? text.substring(1) : text).toMap();
    } on Object {
      throw const ProfileException('parse');
    }
    return fromMap(map, kind);
  }

  /// Reads a profile from a TOML table already parsed, such as Canoa.cfg's `[profile]`.
  static Profile fromMap(Map<dynamic, dynamic> map, LayerKind kind) {
    _shape(map);
    return _checked(map, kind.confined);
  }
}

const _top = {'model', 'effort', 'mode', 'context', 'unset', 'share_env', 'env', 'skills', 'mcp'};
const _envKeys = {'value', 'from_env', 'from_file', 'remove'};
const _mcpKeys = {'type', 'command', 'args', 'env', 'url', 'headers', 'remove'};

Never _parseError() => throw const ProfileException('parse');

void _keys(Map<dynamic, dynamic> map, Set<String> allowed) {
  if (map.keys.any((key) => key is! String || !allowed.contains(key))) _parseError();
}

void _optional<T>(Map<dynamic, dynamic> map, String key) {
  if (map.containsKey(key) && map[key] is! T) _parseError();
}

Map<dynamic, dynamic> _tableOf(Object? value) => value is Map ? value : _parseError();

/// A key no profile has, or a value of the wrong type, anywhere in the document: what serde refuses before any rule.
void _shape(Map<dynamic, dynamic> map) {
  _keys(map, _top);
  for (final key in ['model', 'effort', 'mode']) {
    _optional<String>(map, key);
  }
  if (map.containsKey('context')) {
    final context = _tableOf(map['context']);
    _keys(context, {'text', 'file'});
    _optional<String>(context, 'text');
    _optional<String>(context, 'file');
  }
  for (final key in ['unset', 'share_env']) {
    if (!map.containsKey(key)) continue;
    final list = map[key];
    if (list is! List || list.any((item) => item is! String)) _parseError();
  }
  void values(Object? table) {
    for (final entry in _tableOf(table).values) {
      final env = _tableOf(entry);
      _keys(env, _envKeys);
      for (final key in ['value', 'from_env', 'from_file']) {
        _optional<String>(env, key);
      }
      _optional<bool>(env, 'remove');
    }
  }
  if (map.containsKey('env')) values(map['env']);
  if (map.containsKey('skills')) {
    for (final entry in _tableOf(map['skills']).values) {
      final skill = _tableOf(entry);
      _keys(skill, {'path', 'remove'});
      _optional<String>(skill, 'path');
      _optional<bool>(skill, 'remove');
    }
  }
  if (map.containsKey('mcp')) {
    for (final entry in _tableOf(map['mcp']).values) {
      final server = _tableOf(entry);
      _keys(server, _mcpKeys);
      for (final key in ['type', 'command', 'url']) {
        _optional<String>(server, key);
      }
      _optional<bool>(server, 'remove');
      if (server.containsKey('args')) {
        final args = server['args'];
        if (args is! List || args.any((arg) => arg is! String)) _parseError();
      }
      if (server.containsKey('env')) values(server['env']);
      if (server.containsKey('headers')) values(server['headers']);
    }
  }
}

Never _invalid(String code, String key) => throw ProfileException(code, key: key);

/// Not empty, with no control character.
bool _plain(String text) => text.isNotEmpty && !text.runes.any(_isControl);

/// Unicode's control characters (Cc), as Rust's `char::is_control`.
bool _isControl(int rune) => rune < 0x20 || (rune >= 0x7f && rune < 0xa0);

/// Unicode's White_Space, as Rust's `char::is_whitespace`.
bool _isSpace(int rune) =>
    (rune >= 0x09 && rune <= 0x0d) || rune == 0x20 || rune == 0x85 || rune == 0xa0 || rune == 0x1680 ||
    (rune >= 0x2000 && rune <= 0x200a) || rune == 0x2028 || rune == 0x2029 || rune == 0x202f || rune == 0x205f ||
    rune == 0x3000;

final _entryName = RegExp(r'^[A-Za-z0-9_-]{1,64}$');
final _envName = RegExp(r'^[A-Za-z_][A-Za-z0-9_]{0,127}$');

bool isEntryName(String name) => _entryName.hasMatch(name);
bool isEnvName(String name) => _envName.hasMatch(name);

/// Judged on the text, the same on every platform. A confined path is relative and stays below its directory: no root,
/// drive, home or `..`, and no `:`, which on Windows names a drive or a file's stream.
bool pathAllowed(String path, {required bool confined}) {
  if (path.isEmpty || path.contains('\u0000')) return false;
  if (!confined) return true;
  return !path.startsWith('/') && !path.startsWith('\\') && !path.startsWith('~') && !path.contains(':') &&
      !path.split(RegExp(r'[/\\]')).contains('..');
}

String _path(String key, String path, bool confined) =>
    pathAllowed(path, confined: confined) ? path : _invalid('path', key);

Profile _checked(Map<dynamic, dynamic> map, bool confined) {
  final scalar = <String, String>{};
  for (final key in ['model', 'effort', 'mode']) {
    if (map[key] case final String value) {
      if (!_plain(value)) _invalid('value', key);
      scalar[key] = value;
    }
  }
  ProfileContext? context;
  if (map['context'] case final Map<dynamic, dynamic> raw) {
    context = switch ((raw['text'], raw['file'])) {
      (final String text, null) =>
        text.isEmpty || text.contains('\u0000') ? _invalid('value', 'context') : ProfileContext.text(text),
      (null, final String file) => ProfileContext.file(_path('context', file, confined)),
      _ => _invalid('form', 'context'),
    };
  }
  final unset = <String>[];
  for (final name in (map['unset'] as List<dynamic>?) ?? const []) {
    if (!Profile.scalars.contains(name)) _invalid('form', 'unset');
    if (scalar.containsKey(name) || (name == 'context' && context != null)) _invalid('form', name as String);
    if (!unset.contains(name)) unset.add(name as String);
  }
  final shareEnv = <String>[];
  for (final name in ((map['share_env'] as List<dynamic>?) ?? const []).cast<String>()) {
    if (confined) _invalid('form', 'share_env');
    if (!isEnvName(name)) _invalid('name', 'share_env');
    if (!shareEnv.contains(name)) shareEnv.add(name);
  }
  final env = <String, EnvValue?>{};
  for (final MapEntry(:key, :value) in ((map['env'] as Map<dynamic, dynamic>?) ?? const {}).entries) {
    final name = key as String;
    if (!isEnvName(name)) _invalid('name', 'env');
    final raw = value as Map<dynamic, dynamic>;
    env[name] = raw.containsKey('remove') ? _removal('env.$name', raw) : _reserved(
      'env.$name', name, _envValue('env.$name', raw, confined), confined);
  }
  final skills = <String, Skill?>{};
  for (final MapEntry(:key, :value) in ((map['skills'] as Map<dynamic, dynamic>?) ?? const {}).entries) {
    final name = key as String;
    if (!isEntryName(name)) _invalid('name', 'skills');
    final raw = value as Map<dynamic, dynamic>;
    skills[name] = raw.containsKey('remove')
        ? _removal('skills.$name', raw)
        : raw['path'] is String
        ? Skill(_path('skills.$name', raw['path'] as String, confined))
        : _invalid('form', 'skills.$name');
  }
  final mcp = <String, McpServer?>{};
  for (final MapEntry(:key, :value) in ((map['mcp'] as Map<dynamic, dynamic>?) ?? const {}).entries) {
    final name = key as String;
    if (!isEntryName(name)) _invalid('name', 'mcp');
    final raw = value as Map<dynamic, dynamic>;
    mcp[name] = raw.containsKey('remove') ? _removal('mcp.$name', raw) : _mcpServer('mcp.$name', raw, confined);
  }
  return Profile(
    model: scalar['model'],
    effort: scalar['effort'],
    mode: scalar['mode'],
    context: context,
    unset: List.unmodifiable(unset),
    shareEnv: List.unmodifiable(shareEnv),
    env: Map.unmodifiable(env),
    skills: Map.unmodifiable(skills),
    mcp: Map.unmodifiable(mcp),
  );
}

/// `remove = true` alone.
Null _removal(String key, Map<dynamic, dynamic> raw) =>
    raw['remove'] == true && raw.length == 1 ? null : _invalid('form', key);

EnvValue _envValue(String key, Map<dynamic, dynamic> raw, bool confined) =>
    switch ((raw['value'], raw['from_env'], raw['from_file'])) {
      (final String value, null, null) => value.contains('\u0000') ? _invalid('value', key) : EnvLiteral(value),
      (null, final String name, null) => isEnvName(name) ? EnvFromEnv(name) : _invalid('name', key),
      (null, null, final String path) => EnvFromFile(_path(key, path, confined)),
      _ => _invalid('form', key),
    };

McpServer _mcpServer(String key, Map<dynamic, dynamic> raw, bool confined) {
  switch (raw['type']) {
    case 'stdio':
      if (raw.containsKey('url') || raw.containsKey('headers')) _invalid('form', key);
      final command = raw['command'];
      if (command is! String) _invalid('form', key);
      final args = [for (final arg in (raw['args'] as List<dynamic>?) ?? const []) arg as String];
      if (!_plain(command) || args.any((arg) => arg.contains('\u0000'))) _invalid('value', key);
      return McpStdio(command: command, args: List.unmodifiable(args), env: _entries('$key.env', raw['env'], true, confined));
    case final String type when type == 'http' || type == 'sse':
      if (raw.containsKey('command') || raw.containsKey('args') || raw.containsKey('env')) _invalid('form', key);
      final url = raw['url'];
      if (url is! String) _invalid('form', key);
      if (!(url.startsWith('https://') || url.startsWith('http://')) ||
          url.runes.any((rune) => _isControl(rune) || _isSpace(rune))) {
        _invalid('value', key);
      }
      return McpHttp(url: url, headers: _entries('$key.headers', raw['headers'], false, confined), sse: type == 'sse');
    default:
      _invalid('form', key);
  }
}

/// A confined layer's [value] for [name] unless [name] is reserved.
EnvValue _reserved(String key, String name, EnvValue value, bool confined) =>
    confined && isReservedEnv(name) ? _invalid('reserved', key) : value;

/// A server's `env` ([variables]) or `headers`; a confined layer's variables may not be reserved ones.
Map<String, EnvValue> _entries(String key, Object? table, bool variables, bool confined) {
  final names = variables ? isEnvName : isEntryName;
  final out = <String, EnvValue>{};
  for (final MapEntry(key: name, :value) in ((table as Map<dynamic, dynamic>?) ?? const {}).entries) {
    if (!names(name as String)) _invalid('name', key);
    final raw = value as Map<dynamic, dynamic>;
    if (raw.containsKey('remove')) _invalid('form', '$key.$name');
    final entry = _envValue('$key.$name', raw, confined);
    out[name] = variables ? _reserved('$key.$name', name, entry, confined) : entry;
  }
  return Map.unmodifiable(out);
}

/// One layer: a profile, where it comes from, and the directory its relative paths are read from.
final class Layer {
  const Layer(this.id, this.profile, {this.dir});
  final LayerId id;
  final Profile profile;

  /// Null reads relative paths from nowhere, and resolving one fails.
  final String? dir;
}

/// A value and the layer it came from.
final class Sourced<T> {
  const Sourced(this.value, this.layer, this.dir);
  final T value;
  final LayerId layer;
  final String? dir;
}

/// An entry a layer removed: [key] (`model`, `env.NAME`, ...), the [layer] that removed it, the layer that [was] its.
final class Removed {
  const Removed(this.key, this.layer, this.was);
  final String key;
  final LayerId layer, was;
}

/// An entry of an overlay or a project left out because it [reads] a variable no layer of the person's own shares.
final class Refused {
  const Refused(this.key, this.layer, this.reads);
  final String key;
  final LayerId layer;
  final String reads;
}

/// The profile every layer adds up to, each value with its layer.
final class EffectiveProfile {
  Sourced<String>? model, effort, mode;
  Sourced<ProfileContext>? context;
  final env = <String, Sourced<EnvValue>>{};
  final skills = <String, Sourced<Skill>>{};
  final mcp = <String, Sourced<McpServer>>{};
  final removed = <Removed>[];

  /// The variables overlays and projects may read, each with the first layer that shared it.
  final shareEnv = <String, LayerId>{};

  /// What overlays and projects set that reads a variable not shared, and was left out.
  final refused = <Refused>[];

  /// The view the settings show and the vectors hold: every value with its layer, references as written.
  Map<String, Object?> view() {
    Map<String, Object?> one(Object? value, LayerId layer) => {'value': value, 'layer': '$layer'};
    List<Map<String, Object?>> entries<T>(Map<String, Sourced<T>> map, Object? Function(T) json) => [
      for (final MapEntry(:key, :value) in map.entries) {'name': key, 'value': json(value.value), 'layer': '${value.layer}'},
    ];
    return {
      if (model case final model?) 'model': one(model.value, model.layer),
      if (effort case final effort?) 'effort': one(effort.value, effort.layer),
      if (mode case final mode?) 'mode': one(mode.value, mode.layer),
      if (context case final context?) 'context': one(context.value.toJson(), context.layer),
      'env': entries(env, (value) => value.toJson()),
      'skills': entries(skills, (skill) => skill.toJson()),
      'mcp': entries(mcp, (server) => server.toJson()),
      'removed': [
        for (final removed in this.removed) {'key': removed.key, 'layer': '${removed.layer}', 'was': '${removed.was}'},
      ],
      'share_env': [for (final MapEntry(:key, :value) in shareEnv.entries) {'name': key, 'layer': '$value'}],
      'refused': [
        for (final refused in this.refused) {'key': refused.key, 'layer': '${refused.layer}', 'reads': refused.reads},
      ],
    };
  }

  /// The environment variables, each reference resolved: what a session starts with, and nothing else may hold.
  /// [lookup] reads this process's environment; [readFile] a file's text, or null when it cannot, or when [within] is
  /// given and the file, its links resolved, is outside that directory.
  Future<Map<String, Secret>> environment({
    required String? Function(String name) lookup,
    required Future<String?> Function(String path, {String? within}) readFile,
  }) async => {
    for (final MapEntry(:key, :value) in env.entries)
      key: await resolveValue('env.$key', value.value, value.dir,
          confined: value.layer.kind.confined, lookup: lookup, readFile: readFile),
  };
}

/// A resolved value; [toString] never shows it.
final class Secret {
  const Secret(this._value);
  final String _value;

  /// The value, for the process being started and nothing else.
  String expose() => _value;

  @override
  String toString() => '(hidden)';
}

/// Resolves one value: a literal as written, a variable through [lookup], a file from [dir] without one final line break.
/// A [confined] layer's file must stay inside [dir] once its links are resolved.
Future<Secret> resolveValue(
  String key,
  EnvValue value,
  String? dir, {
  bool confined = false,
  required String? Function(String name) lookup,
  required Future<String?> Function(String path, {String? within}) readFile,
}) async {
  switch (value) {
    case EnvLiteral(:final value):
      return Secret(value);
    case EnvFromEnv(:final name):
      final found = lookup(name);
      if (found == null) throw ProfileException('resolve', key: key);
      return Secret(found);
    case EnvFromFile(:final path):
      final absolute = isAbsolutePath(path);
      if ((!absolute || confined) && dir == null) throw ProfileException('resolve', key: key);
      final text = await readFile(absolute ? path : '$dir/$path', within: confined ? dir : null);
      if (text == null) throw ProfileException('resolve', key: key);
      if (!text.endsWith('\n')) return Secret(text);
      final trimmed = text.substring(0, text.length - 1);
      return Secret(trimmed.endsWith('\r') ? trimmed.substring(0, trimmed.length - 1) : trimmed);
  }
}

/// Whether [path] is absolute on some platform: a root, a backslash root or a drive.
bool isAbsolutePath(String path) =>
    path.startsWith('/') || path.startsWith('\\') || RegExp(r'^[A-Za-z]:[/\\]').hasMatch(path);

/// Merges [layers], lowest first. A later layer's value replaces a lower one's; a named entry is replaced whole and keeps
/// its place; an entry removed and set again later comes last. An overlay's or a project's entry that reads a variable
/// no other layer lists in `share_env` is left out, and the lower layer's entry stays.
EffectiveProfile mergeProfiles(List<Layer> layers) {
  final out = EffectiveProfile();
  for (final layer in layers.where((layer) => !layer.id.kind.confined)) {
    for (final name in layer.profile.shareEnv) {
      out.shareEnv.putIfAbsent(name, () => layer.id);
    }
  }
  String? unshared(Object value) => switch (value) {
    EnvFromEnv(:final name) when !out.shareEnv.containsKey(name) => name,
    McpStdio(:final env) => env.values.map(unshared).nonNulls.firstOrNull,
    McpHttp(:final headers) => headers.values.map(unshared).nonNulls.firstOrNull,
    _ => null,
  };
  for (final layer in layers) {
    final profile = layer.profile;
    Sourced<T> sourced<T>(T value) => Sourced(value, layer.id, layer.dir);
    for (final name in profile.unset) {
      final LayerId? was;
      switch (name) {
        case 'model':
          was = out.model?.layer;
          out.model = null;
        case 'effort':
          was = out.effort?.layer;
          out.effort = null;
        case 'mode':
          was = out.mode?.layer;
          out.mode = null;
        default:
          was = out.context?.layer;
          out.context = null;
      }
      if (was != null) out.removed.add(Removed(name, layer.id, was));
    }
    if (profile.model case final model?) out.model = sourced(model);
    if (profile.effort case final effort?) out.effort = sourced(effort);
    if (profile.mode case final mode?) out.mode = sourced(mode);
    if (profile.context case final context?) out.context = sourced(context);
    void entries<T extends Object>(Map<String, Sourced<T>> into, Map<String, T?> changes, String table) {
      for (final MapEntry(:key, :value) in changes.entries) {
        if (value != null) {
          final reads = layer.id.kind.confined ? unshared(value) : null;
          if (reads != null) {
            out.refused.add(Refused('$table.$key', layer.id, reads));
            continue;
          }
          into[key] = sourced(value);
        } else if (into.remove(key) case final was?) {
          out.removed.add(Removed('$table.$key', layer.id, was.layer));
        }
      }
    }
    entries(out.env, profile.env, 'env');
    entries(out.skills, profile.skills, 'skills');
    entries(out.mcp, profile.mcp, 'mcp');
  }
  return out;
}

/// Where a named overlay comes from: a git repository at a ref, or a directory on this computer.
sealed class OverlaySource {
  const OverlaySource();

  /// Parses `<repository>@<ref>[#<path>]`, as dotagents writes it: the ref follows the last `@`, the path the last `#`.
  /// Throws [ProfileException] with key `overlays` for what canoa-host refuses too, among it a plaintext `http` or
  /// `git` transport at anything but a full commit id, and another host's share.
  static GitOverlay parseGit(String source) {
    Never refused() => throw const ProfileException('value', key: 'overlays');
    var rest = source;
    var path = '';
    final hash = source.lastIndexOf('#');
    if (hash >= 0) {
      path = source.substring(hash + 1);
      rest = source.substring(0, hash);
      if (path.isEmpty || path.startsWith('/') || path.contains(RegExp(r'[\\:\u0000]')) ||
          path.split('/').contains('..')) {
        refused();
      }
    }
    final at = rest.lastIndexOf('@');
    if (at < 0) refused();
    final repo = rest.substring(0, at);
    final reference = rest.substring(at + 1);
    if (repo.isEmpty || repo.startsWith('-') || repo.contains('::') ||
        repo.runes.any((rune) => _isControl(rune) || _isSpace(rune))) {
      refused();
    }
    final scheme = repo.indexOf('://');
    if (scheme >= 0) {
      final name = repo.substring(0, scheme);
      if (!const {'https', 'http', 'ssh', 'git', 'file'}.contains(name) ||
          (name == 'file' && !repo.substring(scheme + 3).startsWith('/')) ||
          ((name == 'http' || name == 'git') && !isFullCommitId(reference))) {
        refused();
      }
    }
    if (repo.startsWith(r'\\') || repo.startsWith('//') || !_gitRef(reference)) refused();
    while (path.endsWith('/')) {
      path = path.substring(0, path.length - 1);
    }
    return GitOverlay(repo: repo, reference: reference, path: path);
  }
}

/// A whole commit id, SHA-1 or SHA-256, in lower-case hex: what a plaintext transport needs, since only the id checks
/// what arrives.
bool isFullCommitId(String reference) =>
    (reference.length == 40 || reference.length == 64) && RegExp(r'^[0-9a-f]+$').hasMatch(reference);

/// A git repository at a ref, and the overlay's directory inside it (empty for its root).
final class GitOverlay extends OverlaySource {
  const GitOverlay({required this.repo, required this.reference, this.path = ''});
  final String repo, reference, path;

  /// The source as Canoa.cfg writes it.
  String get spec => '$repo@$reference${path.isEmpty ? '' : '#$path'}';
}

/// A directory on this computer, read in place.
final class DirOverlay extends OverlaySource {
  const DirOverlay(this.dir);
  final String dir;
}

bool _gitRef(String reference) =>
    reference.isNotEmpty &&
    !reference.startsWith('-') && !reference.startsWith('/') && !reference.startsWith('.') &&
    !reference.endsWith('/') && !reference.endsWith('.') && !reference.endsWith('.lock') &&
    !reference.contains('..') && !reference.contains('//') && !reference.contains('@{') &&
    !reference.runes.any((rune) =>
        _isControl(rune) || _isSpace(rune) || r':~^?*[\'.contains(String.fromCharCode(rune)));
