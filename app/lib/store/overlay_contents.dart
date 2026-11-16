/// What an overlay provides, read from its own files with coqui_aos: its skills, rules and context, knowledge, hooks, environment
/// scripts, MCP servers, `bin` commands and requirements. A source with no overlay.cfg is shown through the manifest Canoa
/// generates for it. Reading runs nothing and installs nothing.
library;

import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../profile/aos_support.dart';

/// A skill an overlay provides: its name and the description its SKILL.md gives.
final class SkillInfo {
  const SkillInfo(this.name, this.description);
  final String name;
  final String? description;

  Map<String, Object?> toJson() => {'name': name, 'description': ?description};
  static SkillInfo fromJson(Map<Object?, Object?> json) =>
      SkillInfo('${json['name']}', json['description'] is String ? json['description']! as String : null);
}

/// A knowledge file an overlay carries, and whether the overlay serves it (through the knowledge tools) rather than putting it
/// in the context.
final class KnowledgeInfo {
  const KnowledgeInfo(this.path, {required this.served});
  final String path;
  final bool served;
}

final class OverlayContents {
  const OverlayContents({
    this.skills = const [],
    this.rules = const [],
    this.context = const [],
    this.knowledge = const [],
    this.hooks = const [],
    this.envScripts = const [],
    this.setup = const [],
    this.mcpServers = const [],
    this.bin = const [],
    this.requirements = const [],
    this.generated = false,
    this.manifest,
    this.digest,
    this.issues = const [],
  });

  final List<SkillInfo> skills;
  final List<String> rules, context;
  final List<KnowledgeInfo> knowledge;

  /// `event: path`, or `event` when the hook names no file.
  final List<String> hooks;
  final List<String> envScripts, setup;

  /// `name (transport)`.
  final List<String> mcpServers;
  final List<String> bin;
  final List<String> requirements;

  /// Whether the source had no overlay.cfg and [manifest] is the one Canoa made for it.
  final bool generated;

  /// The overlay.cfg text generated for a source that has none.
  final String? manifest;
  final String? digest;

  /// The pointers of the manifest's fields that are not valid; they are shown, never acted on.
  final List<String> issues;

  bool get isEmpty =>
      skills.isEmpty && rules.isEmpty && context.isEmpty && knowledge.isEmpty && hooks.isEmpty && envScripts.isEmpty &&
      setup.isEmpty && mcpServers.isEmpty && bin.isEmpty && requirements.isEmpty;

  Map<String, Object?> toJson() => {
    'skills': [for (final skill in skills) skill.toJson()],
    'rules': rules,
    'context': context,
    'knowledge': [for (final file in knowledge) {'path': file.path, 'served': file.served}],
    'hooks': hooks,
    'env_scripts': envScripts,
    'setup': setup,
    'mcp_servers': mcpServers,
    'bin': bin,
    'requirements': requirements,
    'generated': generated,
    'manifest': ?manifest,
    'digest': ?digest,
    'issues': issues,
  };

  static OverlayContents fromJson(Map<Object?, Object?> json) {
    List<String> texts(Object? value) => [if (value is List<Object?>) for (final each in value) '$each'];
    return OverlayContents(
      skills: [
        if (json['skills'] case final List<Object?> list)
          for (final each in list)
            if (each is Map<Object?, Object?>) SkillInfo.fromJson(each),
      ],
      rules: texts(json['rules']),
      context: texts(json['context']),
      knowledge: [
        if (json['knowledge'] case final List<Object?> list)
          for (final each in list)
            if (each is Map<Object?, Object?>) KnowledgeInfo('${each['path']}', served: each['served'] == true),
      ],
      hooks: texts(json['hooks']),
      envScripts: texts(json['env_scripts']),
      setup: texts(json['setup']),
      mcpServers: texts(json['mcp_servers']),
      bin: texts(json['bin']),
      requirements: texts(json['requirements']),
      generated: json['generated'] == true,
      manifest: json['manifest'] is String ? json['manifest']! as String : null,
      digest: json['digest'] is String ? json['digest']! as String : null,
      issues: texts(json['issues']),
    );
  }
}

String? _description(Uint8List? skillFile) {
  if (skillFile == null) return null;
  try {
    final read = aos.readFrontMatter(skillFile, aos.defaultSettings);
    return switch (read.data?['description']) {
      final String text when text.trim().isNotEmpty => text.trim(),
      _ => null,
    };
  } on aos.AosError {
    return null;
  }
}

/// The skill directories a source with no manifest holds by the conventions of skill repositories: `skills/NAME/SKILL.md`, or
/// `NAME/SKILL.md` at the root, or a SKILL.md at the root alone (named [fallback]).
List<(String, String)> _skillDirectories(Set<String> files, String fallback) {
  final found = <(String, String)>[];
  for (final path in files.toList()..sort()) {
    final parts = path.split('/');
    if (parts.last != 'SKILL.md') continue;
    if (parts.length == 3 && parts.first == 'skills') {
      found.add((parts[1], path));
    } else if (parts.length == 2) {
      found.add((parts.first, path));
    } else if (parts.length == 1) {
      found.add((fallback, path));
    }
  }
  return found;
}

/// The overlay.cfg Canoa generates for a source that has none, from what it holds.
String generatedManifest(String name, List<SkillInfo> skills) {
  String quote(String text) => '"${text.replaceAll('\\', r'\\').replaceAll('"', r'\"').replaceAll('\n', ' ')}"';
  final first = skills.map((skill) => skill.description).whereType<String>().firstOrNull;
  return [
    '# Generated by Canoa: this source has no overlay.cfg. It is read, never written into the source.',
    'name = ${quote(name)}',
    '',
    '[about]',
    'summary = ${quote(first ?? (skills.isEmpty ? name : '${skills.length} skills'))}',
    '',
  ].join('\n');
}

/// What the overlay whose files are [files] provides. [name] is the entry's name, for the manifest generated when the files hold
/// none. Throws [aos.AosError] for files that cannot be an overlay.
OverlayContents readContents(Map<String, List<int>> files, {required String name, String os = 'linux'}) {
  final read = treeOfFiles(files);
  final tree = read.tree;
  final paths = aos.OverlayTree.fromPaths(files.keys.toSet());
  final hasManifest = tree.contains('OVERLAY.md') || tree.contains('overlay.cfg');
  aos.Document manifest = const {};
  var issues = const <String>[];
  var context = const <String>[];
  if (hasManifest) {
    final overlay = aos.readOverlay(tree, aos.defaultSettings);
    manifest = overlay.manifest;
    context = overlay.context;
    issues = overlay.issues;
  }
  Uint8List? bytes(String path) => files.containsKey(path) ? tree.bytes(path) : null;

  // Skills: the manifest's and the conventions' (bundled), or the skill repositories' layouts for a source with no manifest.
  final skills = <SkillInfo>[];
  if (hasManifest || !_skillDirectories(files.keys.toSet(), name).any((_) => true)) {
    for (final entry in aos.skillEntries(manifest, paths)) {
      skills.add(SkillInfo(entry.name, entry.origin == 'bundled' ? _description(bytes('skills/${entry.name}/SKILL.md')) : null));
    }
  } else {
    for (final (skill, path) in _skillDirectories(files.keys.toSet(), name)) {
      skills.add(SkillInfo(skill, _description(bytes(path))));
    }
  }

  // Knowledge is what an overlay's rules and context name; `[knowledge] serve = true` has it served instead of put in the context.
  final served = manifest['knowledge'] is Map && (manifest['knowledge']! as Map)['serve'] == true;
  final knowledge = <String>{
    for (final path in [...aos.rulesOf(manifest, paths), ...context])
      if (path != 'OVERLAY.md') path,
  };

  final hooks = <String>[];
  final found = aos.hooksOf(manifest, paths, os, aos.interpreters);
  for (final hook in found.hooks) {
    hooks.add(hook.path == null ? hook.event : '${hook.event}: ${hook.path}');
  }
  for (final path in found.notRun) {
    hooks.add('$path (no interpreter to run it)');
  }

  final scripts = <String>[
    for (final entry in (manifest['env_scripts'] is List ? manifest['env_scripts']! as List : const []))
      if (entry is Map && entry['path'] is String) entry['path']! as String,
  ];
  final setup = <String>[
    for (final entry in (manifest['setup'] is List ? manifest['setup']! as List : const []))
      if (entry is Map && entry['path'] is String) entry['path']! as String,
  ];
  if (aos.defaultOn(manifest, 'env_scripts')) {
    for (final stem in const ['env']) {
      for (final extension in const ['.py', '.js', '.mjs', '.cjs', '.ps1', '.bash', '.sh']) {
        if (files.containsKey('$stem$extension') && !scripts.contains('$stem$extension')) scripts.add('$stem$extension');
      }
    }
  }
  if (aos.defaultOn(manifest, 'setup')) {
    for (final extension in const ['.py', '.js', '.mjs', '.cjs', '.ps1', '.bash', '.sh']) {
      if (files.containsKey('setup$extension') && !setup.contains('setup$extension')) setup.add('setup$extension');
    }
  }

  final servers = manifest['mcp'] is Map ? (manifest['mcp']! as Map)['servers'] : null;
  final mcp = <String>[
    for (final server in (servers is List ? servers : const []))
      if (server is Map) '${server['name'] ?? ''} (${server['transport'] ?? 'stdio'})',
  ];

  final bin = <String>[
    for (final path in files.keys.toList()..sort())
      if (path.startsWith('bin/') && path.indexOf('/', 4) < 0) path.substring(4),
  ];

  final requirements = <String>[
    for (final item in (manifest['requires'] is List ? manifest['requires']! as List : const [])) 'overlay $item',
  ];
  try {
    for (final dependency in aos.dependenciesOf(manifest, paths, os)) {
      requirements.add('${dependency.runtime}: ${dependency.file}');
    }
  } on aos.AosError {
    // A dependency file that cannot be read is not listed; the manifest's own issues say what was wrong.
  }

  return OverlayContents(
    skills: skills,
    rules: aos.rulesOf(manifest, paths),
    context: context,
    knowledge: [for (final path in knowledge) KnowledgeInfo(path, served: served)],
    hooks: hooks,
    envScripts: scripts,
    setup: setup,
    mcpServers: mcp,
    bin: bin,
    requirements: requirements,
    generated: !hasManifest,
    manifest: hasManifest ? null : generatedManifest(name, skills),
    digest: '${read.digest.digest}',
    issues: issues,
  );
}

/// What a skill fetched from its own source provides: itself, with the description its SKILL.md gives.
OverlayContents readSkillContents(List<(String, Uint8List)> files, {required String name}) {
  final skill = files.where((file) => file.$1 == 'SKILL.md').firstOrNull;
  return OverlayContents(skills: [SkillInfo(name, _description(skill?.$2))]);
}
