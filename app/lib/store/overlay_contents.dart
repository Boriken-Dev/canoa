/// What an overlay provides, as the host reads it from the overlay's own files (`store/read_ahead`): its skills, rules and context,
/// knowledge, hooks, environment scripts, MCP servers, `bin` commands and requirements. A source with no overlay.cfg is shown
/// through the manifest Canoa generates for it.
library;

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

/// A file of an overlay that holds a view or a catalog of one (`surfaces/*.json`, `catalogs/*.json`), as the host read it.
final class ViewFile {
  const ViewFile(this.path, this.text);
  final String path, text;
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
    this.views = const [],
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

  /// The files of its views and their catalogs.
  final List<ViewFile> views;

  /// Whether the source had no overlay.cfg and [manifest] is the one Canoa made for it.
  final bool generated;

  /// The overlay.cfg text generated for a source that has none.
  final String? manifest;
  final String? digest;

  /// The pointers of the manifest's fields that are not valid; they are shown, never acted on.
  final List<String> issues;

  bool get isEmpty =>
      skills.isEmpty && rules.isEmpty && context.isEmpty && knowledge.isEmpty && hooks.isEmpty && envScripts.isEmpty &&
      setup.isEmpty && mcpServers.isEmpty && bin.isEmpty && requirements.isEmpty && views.isEmpty;

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
    'views': [for (final file in views) {'path': file.path, 'text': file.text}],
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
      views: [
        if (json['views'] case final List<Object?> list)
          for (final each in list)
            if (each is Map<Object?, Object?> && each['path'] is String && each['text'] is String) ViewFile(each['path']! as String, each['text']! as String),
      ],
      generated: json['generated'] == true,
      manifest: json['manifest'] is String ? json['manifest']! as String : null,
      digest: json['digest'] is String ? json['digest']! as String : null,
      issues: texts(json['issues']),
    );
  }
}

