/// An AOS overlay as a profile layer: what Canoa applies of it, what it reports as not applied yet, and its
/// environment, built when a session starts. canoa has the same (`canoa_host::profile::overlay_layer`), held to the
/// `overlays` cases of `crates/canoa-host/tests/vectors/profile.json`.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import 'aos_support.dart';
import 'profile.dart';
import 'skill_copies.dart';

/// An AOS overlay as Canoa applies it: what building its environment needs, and what Canoa does not apply.
final class AosLayer {
  const AosLayer({required this.identity, required this.root, required this.os, required this.digest,
      required this.pinned, required this.manifest, required this.files, required this.texts, required this.references,
      required this.notApplied, this.about = const {}, this.icon = const {}, this.blobs = const {}});

  /// The name canoa.cfg gives it, its identity.
  final String identity;

  /// Its root, absolute, as processes see it.
  final String root;

  /// The OS its environment is built for, as `os` keys name it.
  final String os;
  final String digest;
  final bool pinned;

  /// Its manifest as Canoa applies it: the `bin` and `lib` defaults off, and no header sent over plain `http` to another
  /// host.
  final aos.Document manifest;
  final Set<String> files;

  /// Its env files' bytes, by relative path.
  final Map<String, List<int>> texts;

  /// Each secret reference of its environment: the variable, the reference and whether it is optional.
  final List<(String, String, bool)> references;

  /// What it declares that Canoa does not apply yet, each as `<field>` or `<field>: <detail>`.
  final List<String> notApplied;

  /// The valid `summary` and `icon` of its `[about]`, for Settings to show; nothing else reads them.
  final Map<String, String> about;

  /// The icon `[about]` names, by path, as the bytes its digest covered, once its header passed `readIcon`: what
  /// Settings decodes, never the file read again. Empty when there is none or it was refused.
  final Map<String, List<int>> icon;

  /// The bytes its digest covered of every file under a skill's directory and of every context file, by relative path:
  /// what a session is given, never the files read again.
  final Map<String, List<int>> blobs;
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
    'manifest' => 'Its manifest cannot be read',
    'invalid' => 'Its manifest is not valid',
    'env-file' => 'An env file of it cannot be read',
    _ => 'It sets a variable a layer from elsewhere may not set',
  }} ($subject)';
}

/// An AOS overlay's report in the view: what Canoa does not apply, what is not granted, and its non-default settings.
final class OverlayReport {
  const OverlayReport(this.layer, this.digest, this.notApplied, this.ungranted, this.unresolved, this.nonDefaults);
  final LayerId layer;
  final String digest;
  final List<String> notApplied, ungranted, unresolved;
  final List<aos.NonDefaultSetting> nonDefaults;

  Map<String, Object?> toJson() => {
    'layer': '$layer',
    'digest': digest,
    'not_applied': notApplied,
    'ungranted': ungranted,
    'unresolved': unresolved,
    'non_defaults': [for (final w in nonDefaults) [w.id, w.subject]],
  };
}

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
/// applied and are reported; so is everything Canoa does not run. Its `[about]` icon is kept only within [iconLimits].
/// A skill from elsewhere is delivered from its entry in [fetched] (by lower-case name) once [skillCopyRefusal] finds
/// nothing against it, and otherwise reported with the reason it is not. Throws [OverlayRefusal].
Layer overlayLayer({
  required String identity,
  required String root,
  required String os,
  required aos.OverlayTree tree,
  String? harness,
  bool pinned = false,
  aos.Settings? iconLimits,
  Map<String, SkillCopy> fetched = const {},
}) {
  if (!aos.validOverlayName(identity)) throw OverlayRefusal('name', identity);
  final aos.TreeDigest digest;
  try {
    digest = digestOfTree(tree);
  } on aos.AosError catch (error) {
    throw OverlayRefusal('digest', error.message);
  }
  final files = tree.members().toSet();
  final paths = aos.OverlayTree.fromPaths(files);
  Uint8List? member(String path) => files.contains(path) ? tree.bytes(path) : null;
  final aos.Document manifest;
  final List<String> ignored;
  try {
    final read = aos.readOverlay(tree, aos.defaultSettings);
    manifest = read.manifest;
    ignored = read.contextIgnored;
  } on aos.AosError catch (error) {
    throw OverlayRefusal('manifest', _reading(error));
  }
  final issues = aos.validate(manifest, paths);
  // `about` is store metadata: a problem in it never stops an overlay, and Canoa applies nothing of it.
  for (final issue in issues) {
    if (!aos.applies([issue])) throw OverlayRefusal('invalid', '${issue.at} ${issue.code}');
  }
  final windows = os == 'windows';
  final notApplied = <String>[
    for (final file in ignored) 'context: $file, ignored while the body of OVERLAY.md is the context',
  ];
  final texts = <String, List<int>>{};
  final references = <(String, String, bool)>[];
  final env = <String, EnvValue>{aos.rootVariable(identity): EnvLiteral(root)};
  void set(String name, EnvValue value) {
    if (value is EnvReference) references.add((name, value.reference, value.optional));
    env[name] = value;
  }

  for (final file in aos.envFilesOf(manifest, paths)) {
    if (!aos.entryOn(file, os)) continue;
    final path = _text(file, 'path');
    final bytes = member(path) ?? (throw OverlayRefusal('env-file', '$path: missing-file'));
    OverlayRefusal invalid(String code) => OverlayRefusal('env-file', '$path: $code');
    if (_text(file, 'format') == 'dotenv') {
      final List<aos.DotenvLine> lines;
      try {
        lines = aos.parseDotenv(bytes);
      } on aos.AosError catch (error) {
        throw invalid(error.errorClass);
      }
      for (final line in lines) {
        set(line.name, line.literal ? EnvLiteral(line.text) : EnvText(line.text));
      }
    } else {
      final aos.Document mapping;
      try {
        mapping = aos.readDocument(bytes, aos.defaultSettings);
      } on aos.AosError catch (error) {
        throw invalid(error.errorClass);
      }
      if (aos.validate({'env': mapping}).isNotEmpty) throw invalid('invalid-env-file');
      for (final MapEntry(:key, :value) in sortedEntries(mapping)) {
        set(key, _envValue(value));
      }
    }
    texts[path] = bytes;
  }
  for (final MapEntry(:key, :value) in sortedEntries(manifest['env'])) {
    set(key, _envValue(value));
  }
  final applied = _copy(manifest);
  final defaults = aos.defaultPaths(manifest, paths);
  for (final addition in defaults) {
    notApplied.add('paths: ${_text(addition, 'path')}/ on ${_text(addition, 'variable')}, which a layer from elsewhere may not change');
  }
  if (defaults.isNotEmpty) {
    final table = (applied['defaults'] as Map<String, Object>?) ?? <String, Object>{};
    table['bin'] = false;
    table['lib'] = false;
    applied['defaults'] = table;
  }
  final lists = <String, List<(String, bool)>>{};
  for (final addition in aos.pathsOf(applied, paths)) {
    if (aos.entryOn(addition, os)) {
      (lists[_text(addition, 'variable')] ??= []).add((_text(addition, 'path'), _text(addition, 'position') == 'append'));
    }
  }
  for (final MapEntry(:key, :value) in lists.entries) {
    set(key, EnvPaths(value));
  }
  for (final name in env.keys.skip(1)) {
    if (isReservedEnv(name)) throw OverlayRefusal('reserved', name);
  }

  final skills = <String, Skill?>{};
  final fetchedBlobs = <String, List<int>>{};
  for (final entry in aos.skillEntries(manifest, paths)) {
    if (entry.origin == 'bundled') {
      skills[entry.name] = Skill('skills/${entry.name}');
      continue;
    }
    final copy = fetched[skillKey(entry.name)];
    if (skillCopyRefusal(entry, identity, copy) case final why?) {
      notApplied.add('skills.${entry.name}: $why');
      continue;
    }
    skills[entry.name] = Skill('skills/${entry.name}');
    for (final MapEntry(:key, :value) in copy!.files.entries) {
      fetchedBlobs['skills/${entry.name}/$key'] = value;
    }
  }

  final insecure = _insecureServers(applied, files, texts, identity: identity, root: root, os: os, pinned: pinned);
  final mcp = <String, McpServer?>{};
  final servers = applied['mcp'] is Map ? (applied['mcp']! as Map)['servers'] : null;
  for (final server in (servers is List ? servers : const []).whereType<Map<String, Object?>>()) {
    final name = '${server['name'] ?? ''}';
    if (server.containsKey('timeout')) notApplied.add('mcp.$name.timeout');
    if (server['transport'] == 'stdio') {
      final command = '${server['command'] ?? ''}';
      final variables = <String, EnvValue>{
        for (final MapEntry(:key, :value) in sortedEntries(server['env'])) key: _envValue(value),
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
      final plain = insecure.contains(name);
      final headers = <String, EnvValue>{};
      for (final MapEntry(:key, :value) in sortedEntries(server['headers_env'])) {
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

  final contexts = <ProfileContext>[for (final path in aos.rulesOf(manifest, paths)) ProfileContext.file(path)];
  final routing = [
    for (final line in (manifest['routing'] is List ? manifest['routing']! as List : const []))
      if (line is String && line.trim().isNotEmpty) line,
  ];
  if (routing.isNotEmpty) contexts.add(ProfileContext.text(routing.join('\n')));
  contexts.addAll([for (final path in aos.contextOf(manifest, tree)) ?_context(tree, path)]);
  String? model, effort;
  for (final MapEntry(key: id, value: hints) in sortedEntries(manifest['harness'])) {
    if (harness != id) {
      notApplied.add('harness.$id: hints for another harness');
      continue;
    }
    for (final MapEntry(:key, :value) in sortedEntries(hints)) {
      switch ((key, value)) {
        case ('model' || 'effort', final String text) when !_plain(text):
          notApplied.add('harness.$id.$key: not one line of plain text');
        case ('model', final String text):
          model = text;
        case ('effort', final String text):
          effort = text;
        case ('context', final List<dynamic> paths):
          contexts.addAll([for (final path in paths) if (path is String) ?_context(tree, path)]);
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
  for (final MapEntry(:key) in sortedEntries(manifest['runtime'])) {
    notApplied.add('runtime.$key');
  }
  for (final MapEntry(:key, :value) in sortedEntries(manifest['scripts'])) {
    if (value != false) notApplied.add('scripts.$key');
  }

  final about = <String, String>{};
  if (manifest['about'] case final Map<String, Object> table) {
    for (final key in const ['summary', 'icon']) {
      if (table[key] case final String value when aos.aboutFieldValid(key, value)) about[key] = value;
    }
  }
  final icon = <String, List<int>>{};
  if (about['icon'] case final path?) {
    try {
      final bytes = member(path) ?? (throw aos.AosError('icon-format'));
      aos.readIcon(bytes, path, iconLimits ?? aos.defaultSettings);
      icon[path] = bytes;
    } on aos.AosError {
      about.remove('icon');
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
      digest: '${digest.digest}',
      pinned: pinned,
      manifest: applied,
      files: files,
      texts: texts,
      references: references,
      notApplied: notApplied,
      about: about,
      icon: icon,
      blobs: {
        ...fetchedBlobs,
        for (final path in files)
          if (skills.values.any((skill) => path.startsWith('${skill!.path}/')) ||
              contexts.any((context) => context.file == path))
            path: tree.bytes(path)!,
      },
    ),
  );
}

/// A reader's refusal as its class and line: the parsers disagree on columns, and the text of a document is never quoted.
String _reading(aos.AosError error) => error.line == null ? error.errorClass : '${error.errorClass} at line ${error.line}';

/// A context entry as the profile holds it: a file, or for `OVERLAY.md` its body as text, since the front matter is the
/// manifest and no context. An empty body is no context.
ProfileContext? _context(aos.OverlayTree tree, String path) {
  if (path != 'OVERLAY.md') return ProfileContext.file(path);
  final bytes = tree.bytes(path);
  if (bytes == null) return null;
  final text = utf8.decode(aos.contextFileText(bytes, aos.defaultSettings), allowMalformed: true);
  return text.trim().isEmpty ? null : ProfileContext.text(text);
}

/// Not empty, with no control character: what Canoa's own layers take for a model or an effort.
bool _plain(String text) => text.isNotEmpty && !text.runes.any((rune) => rune < 0x20 || (rune >= 0x7f && rune < 0xa0));

bool _shared(Map<String, LayerId> shared, String name, bool windows) =>
    shared.keys.any((key) => key == name || windows && key.toLowerCase() == name.toLowerCase());

/// A text field of a mapping, empty when it is absent or not text.
String _text(Object? entry, String key) => entry is Map && entry[key] is String ? entry[key]! as String : '';

/// An independent copy of a document.
aos.Document _copy(aos.Document manifest) => aos.fromPlain(aos.toPlain(manifest)) as aos.Document;

/// What `aos.effective` says of an overlay for the view. The manifest is read less the references it must have and
/// nothing shares (the view lists those as unresolved, and they would fail the call). [shared]
/// are the variables the person's own layers share, each granted and read as empty text. Throws [aos.AosError].
aos.Effective _probe(aos.Document manifest, Set<String> files, Map<String, List<int>> texts,
    {required String identity, required String root, required String os, required bool pinned, List<String> shared = const []}) {
  final copy = _copy(manifest);
  if (copy['env'] case final Map<String, Object> env) {
    env.removeWhere((_, entry) {
      if (entry is! Map<String, Object>) return false;
      final reference = entry.containsKey('provider') || entry.containsKey('secret');
      final granted = entry['provider'] == 'env' && entry['key'] is String && shared.contains(entry['key']);
      return reference && entry['optional'] != true && !granted;
    });
  }
  final facts = aos.Facts(os)
    ..root = root
    ..identity = identity
    ..pinned = pinned
    ..grants = {'env': [...shared]}
    ..hostEnv = {for (final name in shared) name: ''}
    ..baseEnv = {for (final name in shared) name: ''}
    ..tree = _treeOf(files, texts);
  return aos.effective(copy, facts);
}

aos.OverlayTree _treeOf(Set<String> files, Map<String, List<int>> texts) {
  var tree = aos.OverlayTree.fromPaths(files);
  for (final MapEntry(:key, :value) in texts.entries) {
    tree = tree.withBytes(key, Uint8List.fromList(value));
  }
  return tree;
}

/// The names of the servers that send to a host other than this computer in plain `http`.
Set<String> _insecureServers(aos.Document manifest, Set<String> files, Map<String, List<int>> texts,
    {required String identity, required String root, required String os, required bool pinned}) {
  try {
    final found = _probe(manifest, files, const {}, identity: identity, root: root, os: os, pinned: pinned);
    return {for (final setting in found.nonDefaults) if (setting.id == 'insecure-transport') setting.subject};
  } on aos.AosError {
    // Without a report, every plain `http` server counts as one to another host.
    final servers = manifest['mcp'] is Map ? (manifest['mcp']! as Map)['servers'] : null;
    return {
      for (final server in servers is List ? servers : const [])
        if (_text(server, 'url').toLowerCase().startsWith('http://')) _text(server, 'name'),
    };
  }
}

/// An overlay's report, with what the person's own layers share as its grants.
OverlayReport overlayReport(LayerId layer, AosLayer layerAos, Map<String, LayerId> shared) {
  final windows = layerAos.os == 'windows';
  List<String> ungranted = const [];
  List<aos.NonDefaultSetting> nonDefaults = const [];
  try {
    final found = _probe(layerAos.manifest, layerAos.files, layerAos.texts,
        identity: layerAos.identity, root: layerAos.root, os: layerAos.os, pinned: layerAos.pinned, shared: shared.keys.toList());
    ungranted = found.ungranted;
    // Canoa runs no script, setup or install, so what only those would make non-default is not Canoa's to report.
    nonDefaults = [
      for (final setting in found.nonDefaults)
        if (const {'unpinned', 'env-passthrough', 'secret-read', 'env-sensitive', 'insecure-transport'}.contains(setting.id)) setting,
    ];
  } on aos.AosError {
    // An overlay whose environment cannot be built starts no session; its view then lists no grants.
  }
  final unresolved = [
    for (final (variable, reference, optional) in layerAos.references)
      if (!optional && !(reference.startsWith('env:') && _shared(shared, reference.substring(4), windows)))
        '$variable: $reference',
  ];
  return OverlayReport(layer, layerAos.digest, layerAos.notApplied, ungranted, unresolved, nonDefaults);
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

/// An environment built from the overlays, and the variables of this process it started from.
final class BuiltEnvironment {
  BuiltEnvironment._(this._env, this._owned);
  final aos.Environment _env;

  // The names an overlay set, folded as the environment folds them.
  final Set<String> _owned;

  /// A variable an overlay set; null for one only this process holds.
  String? owned(String name) => _owned.contains(_env.windows ? name.toLowerCase() : name) ? _env.get(name) : null;

  /// What a placeholder reads. Throws [aos.AosError] `secret-placeholder` for a variable it may not read.
  String? lookup(String name) {
    if (_env.isSecret(name)) throw aos.AosError('secret-placeholder', name);
    return _env.get(name);
  }
}

void _words(Object? value, Set<String> out) {
  void add(String text) {
    for (final match in RegExp(r'[A-Za-z_][A-Za-z0-9_]*').allMatches(text)) {
      out.add(match[0]!);
    }
  }

  switch (value) {
    case final String text:
      add(text);
    case final List<Object?> items:
      for (final item in items) {
        _words(item, out);
      }
    case final Map<Object?, Object?> map:
      for (final MapEntry(:key, :value) in map.entries) {
        add('$key');
        _words(value, out);
      }
  }
}

/// A character no environment value of this process starts with in practice: a Unicode noncharacter.
const _mark = '\u{FFFF}';

/// Builds the AOS overlays' environment, lowest first, over this process's environment as [lookup] reads it. A
/// placeholder reads only what the overlays set and the variables the person's own layers share; any other variable of
/// this process is seen by a path addition and `if_unset`, never by a placeholder. Only the variables an overlay's text
/// names are read from the process. Into [after], when given, goes each overlay's environment as it stood once that
/// overlay applied: what its MCP servers' text is substituted against. Throws [ProfileException] `resolve`.
BuiltEnvironment buildOverlayEnvironment(List<AosLayer> layers, Map<String, LayerId> shared, String? Function(String) lookup,
    {Map<String, BuiltEnvironment>? after}) {
  final windows = layers.isNotEmpty && layers.first.os == 'windows';
  final named = <String>{};
  for (final layer in layers) {
    _words(layer.manifest, named);
    for (final text in layer.texts.values) {
      _words(utf8.decode(text, allowMalformed: true), named);
    }
  }
  final base = <String, String>{};
  final process = aos.Environment([], windows);
  // The same base with every value marked: a variable whose value is still marked after the overlays applied holds
  // only what this process gave it, whatever value an overlay would have written.
  final marked = aos.Environment([], windows);
  for (final name in named) {
    if (lookup(name) case final value?) {
      process.set(name, value, !_shared(shared, name, windows));
      marked.set(name, '$_mark$value', !_shared(shared, name, windows));
      base[name] = value;
    }
  }
  final granted = shared.keys.toList();
  final hostEnv = {for (final name in granted) name: ?lookup(name)};
  final active = <(aos.Document, aos.Facts)>[];
  for (final layer in layers) {
    final tree = _treeOf(layer.files, layer.texts);
    active.add((
      layer.manifest,
      aos.Facts(layer.os)
        ..root = layer.root
        ..identity = layer.identity
        ..pinned = layer.pinned
        ..grants = {'env': granted}
        ..hostEnv = hostEnv
        ..tree = tree,
    ));
  }
  BuiltEnvironment? last;
  for (var at = 0; at < layers.length; at++) {
    final composed = aos.composedEnvironment(active.sublist(0, at + 1), process.clone(), {}, []);
    final env = composed.env;
    if (env == null) {
      throw ProfileException('resolve',
          key: 'overlay:${layers[at].identity}.env.${composed.error?.subject ?? ''}',
          reason: _reason(composed.error?.errorClass ?? ''));
    }
    final traced = aos.composedEnvironment(active.sublist(0, at + 1), marked.clone(), {}, []).env;
    last = BuiltEnvironment._(env, {
      for (final (name, value) in env.values())
        if (!base.containsKey(name) || (traced == null ? base[name] != value : traced.get(name) != '$_mark$value')) windows ? name.toLowerCase() : name,
    });
    after?[layers[at].identity] = last;
  }
  return last ?? BuiltEnvironment._(aos.Environment([], windows), const {});
}

/// Whether the person's own layers share [name] with overlays and projects; on Windows names compare without case.
bool isShared(Map<String, LayerId> shared, String name, {required bool windows}) => _shared(shared, name, windows);
