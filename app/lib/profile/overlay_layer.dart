/// An AOS overlay as a profile layer: what Canoa applies of it, what it reports as not applied yet, and its
/// environment, built when a session starts. canoa-host has the same (`canoa_host::profile::overlay_layer`), held to the
/// `overlays` cases of `crates/canoa-host/tests/vectors/profile.json`.
library;

import 'dart:convert';

import 'aos.dart';
import 'profile.dart';

/// An AOS overlay as Canoa applies it: what building its environment needs, and what Canoa does not apply.
final class AosLayer {
  const AosLayer({required this.identity, required this.root, required this.os, required this.digest,
      required this.pinned, required this.manifest, required this.files, required this.texts, required this.references,
      required this.notApplied, this.about = const {}});

  /// The name Canoa.cfg gives it, its identity.
  final String identity;

  /// Its root, absolute, as processes see it.
  final String root;

  /// The OS its environment is built for, as `os` keys name it.
  final String os;
  final String digest;
  final bool pinned;

  /// Its manifest as Canoa applies it: the `bin` and `lib` defaults off, and no header sent over plain `http` to another
  /// host.
  final Map<String, Object?> manifest;
  final Set<String> files;

  /// Its env files' bytes, by relative path.
  final Map<String, List<int>> texts;

  /// Each secret reference of its environment: the variable, the reference and whether it is optional.
  final List<(String, String, bool)> references;

  /// What it declares that Canoa does not apply yet, each as `<field>` or `<field>: <detail>`.
  final List<String> notApplied;

  /// The valid `summary` and `icon` of its `[about]`, for Settings to show; nothing else reads them.
  final Map<String, String> about;
}

/// Why an overlay is left out whole. Never quotes a value.
final class OverlayRefusal implements Exception {
  const OverlayRefusal(this.code, this.subject);

  /// `name`, `digest`, `manifest`, `invalid`, `env-file` or `reserved`.
  final String code;
  final String subject;

  @override
  String toString() => '${switch (code) {
    'name' => 'Its name is not a valid overlay identity',
    'digest' => 'Its files cannot be digested',
    'manifest' => 'Its overlay.cfg cannot be read',
    'invalid' => 'Its overlay.cfg is not valid',
    'env-file' => 'An env file of it cannot be read',
    _ => 'It sets a variable a layer from elsewhere may not set',
  }} ($subject)';
}

/// An AOS overlay's report in the view: what Canoa does not apply, what is not granted, and its weakenings.
final class OverlayReport {
  const OverlayReport(this.layer, this.digest, this.notApplied, this.ungranted, this.unresolved, this.weakenings);
  final LayerId layer;
  final String digest;
  final List<String> notApplied, ungranted, unresolved;
  final List<AosWeakening> weakenings;

  Map<String, Object?> toJson() => {
    'layer': '$layer',
    'digest': digest,
    'not_applied': notApplied,
    'ungranted': ungranted,
    'unresolved': unresolved,
    'weakenings': [for (final w in weakenings) [w.id, w.subject]],
  };
}

/// A mapping's entries by key bytes: mapping order carries no meaning in AOS, so every order Canoa shows is this one.
List<MapEntry<String, Object?>> _sorted(Object? value) => value is Map
    ? ([for (final MapEntry(:key, :value) in value.entries) MapEntry('$key', value)]
      ..sort((a, b) => compareBytes(utf8.encode(a.key), utf8.encode(b.key))))
    : const [];

(String, bool)? _referenceOf(Map<dynamic, dynamic> entry) {
  final optional = entry['optional'] == true;
  if (entry['secret'] is String) return ('secret:${entry['secret']}', optional);
  if (entry['provider'] is! String) return null;
  return ('${entry['provider']}:${entry['key'] ?? ''}', optional);
}

EnvValue _envValue(Object? value) => switch (value) {
  final String text => EnvText(text),
  {'value': final String text} => EnvText(text),
  {'path': final String path} => EnvPath(path),
  final Map<dynamic, dynamic> entry => () {
    final (reference, optional) = _referenceOf(entry) ?? ('', false);
    return EnvReference(reference, optional: optional);
  }(),
  _ => const EnvText(''),
};

String _resolvePath(String root, String relative, bool windows) {
  final separator = windows ? r'\' : '/';
  return '${root.replaceFirst(RegExp(r'[/\\]+$'), '')}$separator${relative.replaceAll('/', separator)}';
}

const _defaultScripts = ['.py', '.js', '.mjs', '.cjs', '.ps1', '.bash', '.sh'];

/// Makes the layer of an AOS overlay from its [tree]: its root variable, environment, skills, MCP servers, context, and
/// the hints for [harness]. The `bin` and `lib` defaults, which change `PATH`, `PYTHONPATH` and `NODE_PATH`, are not
/// applied and are reported; so is everything Canoa does not run. Throws [OverlayRefusal].
Layer overlayLayer({
  required String identity,
  required String root,
  required String os,
  required List<AosTreeEntry> tree,
  String? harness,
  bool pinned = false,
}) {
  if (!validOverlayName(identity)) throw OverlayRefusal('name', identity);
  final AosTreeDigest digest;
  try {
    digest = digestEntries(tree);
  } on AosException catch (error) {
    throw OverlayRefusal('digest', '$error');
  }
  final members = {
    for (final entry in tree)
      if (entry.kind == AosEntryKind.file && !entry.isExcluded) entry.path: entry.data,
  };
  final files = members.keys.toSet();
  final Map<String, Object?> manifest;
  try {
    manifest = members['overlay.cfg'] == null ? {} : readDocument(members['overlay.cfg']!);
  } on AosException catch (error) {
    throw OverlayRefusal('manifest', '$error');
  }
  final issues = validate(manifest, files: files);
  // `about` is store metadata: a problem in it never stops an overlay, and Canoa applies nothing of it.
  for (final issue in issues) {
    if (!applies([issue])) throw OverlayRefusal('invalid', '${issue.at} ${issue.code}');
  }
  final windows = os == 'windows';
  final notApplied = <String>[];
  final texts = <String, List<int>>{};
  final references = <(String, String, bool)>[];
  final env = <String, EnvValue>{rootVariable(identity): EnvLiteral(root)};
  void set(String name, EnvValue value) {
    if (value is EnvReference) references.add((name, value.reference, value.optional));
    env[name] = value;
  }

  for (final file in envFilesOf(manifest, files)) {
    if (!osMatches(file.os, os)) continue;
    final bytes = members[file.path] ?? (throw OverlayRefusal('env-file', '${file.path}: missing-file'));
    OverlayRefusal invalid(String code) => OverlayRefusal('env-file', '${file.path}: $code');
    if (file.dotenv) {
      final List<DotenvLine> lines;
      try {
        lines = parseDotenv(bytes);
      } on AosException catch (error) {
        throw invalid(error.code);
      }
      for (final line in lines) {
        set(line.name, line.literal ? EnvLiteral(line.text) : EnvText(line.text));
      }
    } else {
      final Map<String, Object> mapping;
      try {
        mapping = readDocument(bytes);
      } on AosException catch (error) {
        throw invalid(error.code);
      }
      if (validate({'env': mapping}).isNotEmpty) throw invalid('invalid-env-file');
      for (final MapEntry(:key, :value) in _sorted(mapping)) {
        set(key, _envValue(value));
      }
    }
    texts[file.path] = bytes;
  }
  for (final MapEntry(:key, :value) in _sorted(manifest['env'])) {
    set(key, _envValue(value));
  }
  final applied = jsonDecode(jsonEncode(manifest)) as Map<String, Object?>;
  final defaults = defaultPaths(manifest, files);
  for (final addition in defaults) {
    notApplied.add('paths: ${addition.path}/ on ${addition.variable}, which a layer from elsewhere may not change');
  }
  if (defaults.isNotEmpty) {
    final table = (applied['defaults'] as Map<String, Object?>?) ?? <String, Object?>{};
    table['bin'] = false;
    table['lib'] = false;
    applied['defaults'] = table;
  }
  final lists = <String, List<(String, bool)>>{};
  for (final addition in pathsOf(applied, files)) {
    if (osMatches(addition.os, os)) (lists[addition.variable] ??= []).add((addition.path, addition.append));
  }
  for (final MapEntry(:key, :value) in lists.entries) {
    set(key, EnvPaths(value));
  }
  for (final name in env.keys.skip(1)) {
    if (isReservedEnv(name)) throw OverlayRefusal('reserved', name);
  }

  final skills = <String, Skill?>{for (final name in skillsOf(manifest, files)) name: Skill('skills/$name')};

  final mcp = <String, McpServer?>{};
  final servers = applied['mcp'] is Map ? (applied['mcp']! as Map)['servers'] : null;
  for (final server in (servers is List ? servers : const []).whereType<Map<String, Object?>>()) {
    final name = '${server['name'] ?? ''}';
    if (server.containsKey('timeout')) notApplied.add('mcp.$name.timeout');
    if (server['transport'] == 'stdio') {
      final command = '${server['command'] ?? ''}';
      final variables = <String, EnvValue>{
        for (final MapEntry(:key, :value) in _sorted(server['env'])) key: _envValue(value),
        for (final variable in (server['env_passthrough'] is List ? server['env_passthrough']! as List : const []))
          '$variable': EnvReference('env:$variable', optional: true),
      };
      for (final variable in variables.keys) {
        if (isReservedEnv(variable)) throw OverlayRefusal('reserved', variable);
      }
      notApplied.add("mcp.$name: it runs in the overlay's root, which Canoa does not set yet");
      mcp[name] = McpStdio(
        command: command.contains(RegExp(r'[/\\]')) ? _resolvePath(root, command, windows) : command,
        args: [for (final arg in (server['args'] is List ? server['args']! as List : const [])) '$arg'],
        env: variables,
      );
    } else {
      final url = '${server['url'] ?? ''}';
      final plain = isInsecureUrl(url);
      final headers = <String, EnvValue>{};
      for (final MapEntry(:key, :value) in _sorted(server['headers_env'])) {
        if (plain) {
          notApplied.add('mcp.$name.headers_env.$key: never sent over plain http to another host');
          continue;
        }
        headers[key] = EnvReference('env:$value', optional: true);
      }
      if (plain) server.remove('headers_env');
      mcp[name] = McpHttp(url: url, headers: headers);
    }
  }
  final links = manifest['mcp'] is Map ? (manifest['mcp']! as Map)['skills'] : null;
  for (final link in (links is List ? links : const []).whereType<Map<dynamic, dynamic>>()) {
    notApplied.add('mcp.skills.${link['name'] ?? ''}');
  }

  final contexts = <ProfileContext>[for (final path in rulesOf(manifest, files)) ProfileContext.file(path)];
  final routing = [
    for (final line in (manifest['routing'] is List ? manifest['routing']! as List : const []))
      if (line is String && line.trim().isNotEmpty) line,
  ];
  if (routing.isNotEmpty) contexts.add(ProfileContext.text(routing.join('\n')));
  contexts.addAll([for (final path in contextOf(manifest, files)) ProfileContext.file(path)]);
  String? model, effort;
  for (final MapEntry(key: id, value: hints) in _sorted(manifest['harness'])) {
    if (harness != id) {
      notApplied.add('harness.$id: hints for another harness');
      continue;
    }
    for (final MapEntry(:key, :value) in _sorted(hints)) {
      switch ((key, value)) {
        case ('model', final String text):
          model = text;
        case ('effort', final String text):
          effort = text;
        case ('context', final List<dynamic> paths):
          contexts.addAll([for (final path in paths) if (path is String) ProfileContext.file(path)]);
        case ('skills', true):
          break;
        case ('skills', _):
          notApplied.add('harness.$id.skills: delivery by routing line');
        default:
          notApplied.add("harness.$id.$key: the harness's own setting");
      }
    }
  }

  bool on(String key) => !(manifest['defaults'] is Map && (manifest['defaults']! as Map)[key] == false);
  for (final (field, stem) in const [('env_scripts', 'env'), ('setup', 'setup')]) {
    if (on(field)) {
      for (final extension in _defaultScripts) {
        if (files.contains('$stem$extension')) notApplied.add('$field: $stem$extension');
      }
    }
    for (final entry in (manifest[field] is List ? manifest[field]! as List : const [])) {
      final line = '$field: ${entry is Map ? entry['path'] ?? '' : ''}';
      if (!notApplied.contains(line)) notApplied.add(line);
    }
  }
  if (on('deps')) {
    for (final path in const ['lib/python/requirements.txt', 'lib/node/package.json']) {
      if (files.contains(path)) notApplied.add('dependencies: $path');
    }
  }
  for (final MapEntry(:key) in _sorted(manifest['runtime'])) {
    notApplied.add('runtime.$key');
  }
  for (final MapEntry(:key, :value) in _sorted(manifest['scripts'])) {
    if (value != false) notApplied.add('scripts.$key');
  }

  final about = <String, String>{};
  if (manifest['about'] case final Map<String, Object?> table) {
    final aboutIssues = validate({'about': table}, files: files);
    bool valid(String key) => !aboutIssues.any((issue) => issue.at == '/about' || issue.at.startsWith('/about/$key'));
    for (final key in const ['summary', 'icon']) {
      if (table[key] case final String value when valid(key)) about[key] = value;
    }
  }

  return Layer(
    LayerId(LayerKind.overlay, identity),
    Profile(
      model: model,
      effort: effort,
      env: Map.unmodifiable(env),
      skills: Map.unmodifiable(skills),
      mcp: Map.unmodifiable(mcp),
      contexts: List.unmodifiable(contexts),
    ),
    dir: root,
    aos: AosLayer(
      identity: identity,
      root: root,
      os: os,
      digest: digest.digest,
      pinned: pinned,
      manifest: applied,
      files: files,
      texts: texts,
      references: references,
      notApplied: notApplied,
      about: about,
    ),
  );
}

bool _shared(Map<String, LayerId> shared, String name, bool windows) =>
    shared.keys.any((key) => key == name || windows && key.toLowerCase() == name.toLowerCase());

AosFacts _facts(String os, Map<String, LayerId> shared, String? Function(String) lookup) => AosFacts(
  os,
  grants: {'env': shared.keys.toList()},
  hostEnv: {for (final name in shared.keys) name: ?lookup(name)},
);

/// An overlay's report, with what the person's own layers share as its grants.
OverlayReport overlayReport(LayerId layer, AosLayer aos, Map<String, LayerId> shared) {
  final windows = aos.os == 'windows';
  final weakenings = <AosWeakening>[if (!aos.pinned) const AosWeakening('unpinned', '')];
  final ungranted = grantsReport(aos.manifest, _facts(aos.os, shared, (_) => null), weakenings);
  final unresolved = [
    for (final (variable, reference, optional) in aos.references)
      if (!optional && !(reference.startsWith('env:') && _shared(shared, reference.substring(4), windows)))
        '$variable: $reference',
  ];
  final sorted = weakenings.toSet().toList()..sort();
  return OverlayReport(layer, aos.digest, aos.notApplied, ungranted, unresolved, sorted);
}

String _reason(String code) => switch (code) {
  'undefined-variable' => "an overlay's placeholder names a variable that is not set",
  'secret-placeholder' => "an overlay's placeholder reads a secret, or a variable of this computer none of your layers shares",
  'unresolved-reference' =>
    'an overlay reads a variable none of your layers shares, or one that is not set, or a secret Canoa does not hold',
  'invalid-pattern' => "an overlay's placeholder has a pattern that is not valid",
  'invalid-reference' => 'an overlay names a secret file that is not a relative path',
  'missing-file' || 'invalid-env-file' => "an overlay's env file cannot be read",
  'env-path-conflict' => 'an overlay sets a variable both by env and by a path addition',
  'reserved-variable' => 'an overlay sets a variable the consumer sets',
  _ => "an overlay's environment cannot be built",
};

/// Builds the AOS overlays' environment, lowest first, over this process's environment as [lookup] reads it: every
/// root variable first, then each overlay's env files, `env` and path additions. A placeholder reads only what the
/// overlays set and the variables the person's own layers share. Throws [ProfileException] `resolve`.
AosEnvironment buildOverlayEnvironment(List<AosLayer> layers, Map<String, LayerId> shared, String? Function(String) lookup) {
  if (layers.isEmpty) return AosEnvironment();
  final os = layers.first.os;
  final windows = os == 'windows';
  final env = AosEnvironment(windows: windows, base: (name) {
    final value = lookup(name);
    return value == null ? null : (value, _shared(shared, name, windows));
  });
  for (final layer in layers) {
    env.set(rootVariable(layer.identity), layer.root);
  }
  final facts = _facts(os, shared, lookup);
  for (final layer in layers) {
    try {
      applyOverlay(
        env,
        AosOverlay(identity: layer.identity, manifest: layer.manifest, files: layer.files, root: layer.root,
            read: (path) => layer.texts[path]),
        facts,
        [],
      );
    } on AosException catch (error) {
      throw ProfileException('resolve', key: 'overlay:${layer.identity}.env.${error.subject}', reason: _reason(error.code));
    }
  }
  return env;
}
