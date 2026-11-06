/// Applying a profile to a harness: each part goes where that harness takes it (the process environment, ACP's
/// `systemPrompt` and `mcpServers`, `_meta` an adapter documents, a directory of skills it reads), and every part says
/// what became of it. `docs/configuration.md`, "Applying a profile", has the table this follows.
library;

import 'dart:convert';

import 'package:coqui_acp/coqui_acp.dart'
    show AcpMcpHttp, AcpMcpServer, AcpMcpStdio, AcpMcpTransport, AcpSessionSetup;
import 'package:crypto/crypto.dart';

import 'aos.dart';
import 'overlay_layer.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'profile_layers.dart';

/// The longest initial context a session is given, in UTF-8 bytes, all layers together.
const int maxContextBytes = 1024 * 1024;

/// What one skill's directory may hold, in files and bytes.
const int maxSkillFiles = 1024;
const int maxSkillBytes = 16 * 1024 * 1024;

/// Where Codex reads skills inside each directory given as ACP `additionalDirectories`: the Agent Skills directory.
const codexSkillsPath = '.agents/skills';

/// The plugin Canoa gives Claude Code a profile's skills in; its skills are named `canoa-profile:<skill>`.
const claudePluginName = 'canoa-profile';

/// Each harness's name, by its AOS harness id.
const harnessLabels = {
  'claude-code': 'Claude Code',
  'codex': 'Codex',
  'gemini-cli': 'Gemini CLI',
  'pi': 'Pi',
  'opencode': 'OpenCode',
  'grok': 'Grok',
};

/// The AOS harness id of what an agent reported in `initialize`, else of the preset [entry] it was started from; null
/// for neither.
String? harnessIdOf({String? reportedName, bool grokShell = false, required String entry}) => switch (reportedName) {
  'coqui-acp-claude' || '@agentclientprotocol/claude-agent-acp' => 'claude-code',
  'coqui-acp-codex' || '@agentclientprotocol/codex-acp' => 'codex',
  'gemini-cli' => 'gemini-cli',
  'pi-acp' => 'pi',
  'OpenCode' => 'opencode',
  _ => grokShell ? 'grok' : presetHarnesses[entry],
};

/// What became of one part of a profile.
enum ItemState {
  applied('applied'),
  unsupported('not supported'),
  off('off'),
  refused('refused');

  const ItemState(this.label);
  final String label;
}

/// One part of a profile ([key] such as `context`, `skills.review`, `mcp.github`, `env.TOKEN`, `model`), its [layer],
/// what became of it, and [how]: the channel it went through, or why it did not.
final class ProfileItem {
  const ProfileItem(this.key, this.layer, this.state, this.how);
  final String key, layer, how;
  final ItemState state;

  @override
  String toString() => '$key ($layer): ${state.label}, $how';
}

/// What a session was given of its profile, for the harness it went to.
final class ProfileReport {
  const ProfileReport(this.harness, this.items);

  /// The harness's name, or the agent entry's when Canoa does not know it.
  final String harness;
  final List<ProfileItem> items;

  Iterable<ProfileItem> get applied => items.where((item) => item.state == ItemState.applied);
  Iterable<ProfileItem> get notApplied => items.where((item) => item.state != ItemState.applied);

  /// One line for the conversation: how many applied, and which did not, by key.
  String get summary {
    final missed = notApplied.toList();
    return 'Profile for ${harnessLabels[harness] ?? harness}: ${applied.length} applied'
        '${missed.isEmpty ? '' : '; not applied: ${missed.map((item) => '${item.key} (${item.state.label})').join(', ')}'}';
  }
}

/// A skill's files, read once: from the bytes an overlay's digest covered, or from its directory through a confined read.
final class PreparedSkill {
  const PreparedSkill(this.name, this.layer, {this.files = const {}, this.refused});
  final String name;
  final LayerId layer;

  /// By path relative to the skill's directory, `/`-separated.
  final Map<String, List<int>> files;
  final String? refused;
}

/// An MCP server as the person sees it before a session starts: what it runs or reaches, the variables or headers it
/// receives (never their values), and the server itself once resolved.
final class PreparedMcp {
  const PreparedMcp({
    required this.name,
    required this.layer,
    required this.shown,
    required this.digest,
    required this.isHttp,
    this.server,
    this.receives = const [],
    this.withheld = const [],
    this.notes = const [],
    this.refused,
  });
  final String name;
  final LayerId layer;

  /// Its command line, or its transport and URL.
  final String shown;

  /// SHA-256 of its definition as shown, which an approval is bound to.
  final String digest;

  /// Reached over HTTP or SSE, so what it [receives] are headers rather than variables.
  final bool isHttp;
  final AcpMcpServer? server;

  /// The environment variables (stdio) or headers (HTTP) it is given, by name.
  final List<String> receives;

  /// Those left out, each with why.
  final List<String> withheld;

  /// What else to know before starting it.
  final List<String> notes;
  final String? refused;

  /// Whether it comes from an overlay or a project, so it stays off until the person allows it.
  bool get confined => layer.kind.confined;
}

/// One piece of the initial context, as text.
final class PreparedContext {
  const PreparedContext(this.text, this.layer, this.source);
  final String text;
  final LayerId layer;

  /// `text`, or the file it was read from.
  final String source;
}

/// A profile resolved for one launch: what every harness could be given, before it is known which harness this is.
final class PreparedProfile {
  const PreparedProfile({
    this.environment = const {},
    this.environmentLayers = const {},
    this.contexts = const [],
    this.skills = const [],
    this.mcp = const [],
    this.model,
    this.effort,
    this.mode,
    this.problems = const [],
  });

  /// The variables the agent's process starts with, beside its own environment.
  final Map<String, Secret> environment;
  final Map<String, LayerId> environmentLayers;
  final List<PreparedContext> contexts;
  final List<PreparedSkill> skills;
  final List<PreparedMcp> mcp;
  final Sourced<String>? model, effort, mode;

  /// Parts that could not be read, each refused.
  final List<ProfileItem> problems;

  bool get isEmpty =>
      environment.isEmpty && contexts.isEmpty && skills.isEmpty && mcp.isEmpty && model == null && effort == null &&
      mode == null && problems.isEmpty;

  static const empty = PreparedProfile();
}

/// Where a session's skills are written for a harness that reads them from a directory. A desktop has one; nothing is
/// written anywhere else.
abstract interface class SessionFiles {
  /// A new, private, empty directory holding [files], by `/`-separated relative path; its absolute path.
  Future<String> write(Map<String, List<int>> files);

  /// Removes a directory [write] made, and everything in it.
  Future<void> remove(String directory);
}

/// Whether [path] is [root] or below it, both canonical, without case where the OS has none.
bool _inside(String path, String root, String os) {
  final fold = os == 'windows' || os == 'macos';
  final a = fold ? path.toLowerCase() : path, b = fold ? root.toLowerCase() : root;
  final base = b.endsWith('/') || b.endsWith(r'\') ? b : '$b${os == 'windows' ? r'\' : '/'}';
  return a == b || a.startsWith(base);
}

String _hash(Object? json) => 'sha256:${sha256.convert(utf8.encode(jsonEncode(json)))}';

/// Resolves every part of [layers]' effective profile once, for a launch: the environment (references read), the context
/// as text, each skill's files and each MCP server. An overlay's skills and context come from the bytes its digest
/// covered; a project's paths are read only inside its directory, links resolved. Throws [ProfileException] when the
/// environment cannot be built, since no session may then start with it.
Future<PreparedProfile> prepareProfile(
  ProfileLayers layers, {
  required ProfileHome home,
  required String? Function(String name) lookup,
  required Future<String?> Function(String program) findProgram,
}) async {
  final effective = layers.effective;
  final after = <String, AosEnvironment>{};
  final environment = await effective.environment(lookup: lookup, readFile: home.readSecret, after: after);
  final problems = <ProfileItem>[];
  AosLayer? aosOf(LayerId layer) =>
      layer.kind == LayerKind.overlay ? effective.aos.where((aos) => aos.identity == layer.name).firstOrNull : null;

  final contexts = <PreparedContext>[];
  var contextBytes = 0;
  Future<void> addContext(Sourced<ProfileContext> context) async {
    final value = context.value;
    String? text = value.text;
    if (text == null) {
      final file = value.file!;
      final aos = aosOf(context.layer);
      if (aos != null) {
        final bytes = aos.blobs[file];
        try {
          text = bytes == null ? null : utf8.decode(bytes);
        } on FormatException {
          text = null;
        }
      } else {
        final dir = context.dir;
        final confined = context.layer.kind.confined;
        if (dir != null || isAbsolutePath(file)) {
          text = await home.readSecret(isAbsolutePath(file) ? file : home.join(dir!, file),
              within: confined ? dir : null);
        }
      }
      if (text == null) {
        problems.add(ProfileItem('context', '${context.layer}', ItemState.refused,
            'the file $file cannot be read as UTF-8 text of at most 1 MiB${context.layer.kind.confined ? ' inside its layer' : ''}'));
        return;
      }
    }
    contextBytes += utf8.encode(text).length;
    if (contextBytes > maxContextBytes) {
      problems.add(ProfileItem('context', '${context.layer}', ItemState.refused,
          'the initial context passes $maxContextBytes bytes'));
      return;
    }
    contexts.add(PreparedContext(text, context.layer, value.file ?? 'text'));
  }

  for (final context in effective.contexts) {
    await addContext(context);
  }
  if (effective.context case final context?) await addContext(context);

  final skills = <PreparedSkill>[];
  for (final MapEntry(key: name, :value) in effective.skills.entries) {
    final aos = aosOf(value.layer);
    if (aos != null) {
      final prefix = '${value.value.path}/';
      final files = {
        for (final MapEntry(:key, value: bytes) in aos.blobs.entries)
          if (key.startsWith(prefix)) key.substring(prefix.length): bytes,
      };
      skills.add(files.containsKey('SKILL.md')
          ? PreparedSkill(name, value.layer, files: files)
          : PreparedSkill(name, value.layer, refused: 'its directory holds no SKILL.md'));
      continue;
    }
    skills.add(await _skillFromDisk(name, value, home));
  }

  final mcp = <PreparedMcp>[
    for (final MapEntry(key: name, :value) in effective.mcp.entries)
      await _prepareMcp(name, value, effective, after, home, lookup, findProgram),
  ];

  return PreparedProfile(
    environment: environment,
    environmentLayers: {for (final MapEntry(:key, :value) in effective.env.entries) key: value.layer},
    contexts: contexts,
    skills: skills,
    mcp: mcp,
    model: effective.model,
    effort: effective.effort,
    mode: effective.mode,
    problems: problems,
  );
}

Future<PreparedSkill> _skillFromDisk(String name, Sourced<Skill> skill, ProfileHome home) async {
  final dir = skill.dir;
  final path = skill.value.path;
  if (dir == null && !isAbsolutePath(path)) return PreparedSkill(name, skill.layer, refused: 'its layer has no directory');
  final directory = home.canonicalDirectory(isAbsolutePath(path) ? path : home.join(dir!, path));
  if (directory == null) return PreparedSkill(name, skill.layer, refused: 'its directory is not there');
  if (skill.layer.kind.confined) {
    final root = dir == null ? null : home.canonicalDirectory(dir);
    if (root == null || !_inside(directory, root, home.os)) {
      return PreparedSkill(name, skill.layer, refused: 'its directory, links resolved, is outside its layer');
    }
  }
  final List<AosTreeEntry> tree;
  try {
    tree = await home.readTree(directory, maxFiles: maxSkillFiles, maxBytes: maxSkillBytes);
  } on Object catch (error) {
    return PreparedSkill(name, skill.layer, refused: 'its files cannot be read ($error)');
  }
  if (tree.any((entry) => entry.kind == AosEntryKind.symlink || entry.kind == AosEntryKind.special)) {
    return PreparedSkill(name, skill.layer, refused: 'its directory holds a link or a special file');
  }
  final files = {
    for (final entry in tree)
      if (entry.kind == AosEntryKind.file && !entry.isExcluded) entry.path: entry.data,
  };
  return files.containsKey('SKILL.md')
      ? PreparedSkill(name, skill.layer, files: files)
      : PreparedSkill(name, skill.layer, refused: 'its directory holds no SKILL.md');
}

Future<PreparedMcp> _prepareMcp(
  String name,
  Sourced<McpServer> sourced,
  EffectiveProfile effective,
  Map<String, AosEnvironment> after,
  ProfileHome home,
  String? Function(String) lookup,
  Future<String?> Function(String) findProgram,
) async {
  final layer = sourced.layer;
  final server = sourced.value;
  final digest = _hash({'layer': '$layer', 'server': server.toJson()});
  final aos = layer.kind == LayerKind.overlay
      ? effective.aos.where((aos) => aos.identity == layer.name).firstOrNull
      : null;
  final windows = home.os == 'windows';
  final withheld = <String>[];

  // One value, or null when it is left out (and said in [withheld]). Throws [ProfileException] or [AosException].
  Future<String?> value(String key, String label, EnvValue raw) async {
    switch (raw) {
      case EnvText(:final text) when aos != null:
        // An overlay's text reads what that overlay's environment held once it applied, and its own root and name.
        final env = after[aos.identity];
        return substitute(text, (variable) {
          if (variable == 'AOS_OVERLAY_ROOT' || windows && variable.toUpperCase() == 'AOS_OVERLAY_ROOT') return aos.root;
          if (variable == 'AOS_OVERLAY_NAME' || windows && variable.toUpperCase() == 'AOS_OVERLAY_NAME') {
            return normalizeName(aos.identity);
          }
          return env?.lookup(variable);
        });
      case EnvPath(:final path) when aos != null:
        return home.join(aos.root, path);
      case EnvReference(:final reference, :final optional):
        final colon = reference.indexOf(':');
        final provider = colon < 0 ? reference : reference.substring(0, colon);
        final variable = colon < 0 ? '' : reference.substring(colon + 1);
        if (provider != 'env') {
          withheld.add('$label: reads $reference, a secret Canoa does not hold');
          return null;
        }
        if (!isShared(effective.shareEnv, variable, windows: windows)) {
          withheld.add('$label: reads \$$variable, which none of your layers shares');
          return null;
        }
        final found = lookup(variable);
        if (found == null) {
          if (optional) {
            withheld.add('$label: \$$variable is not set');
            return null;
          }
          throw ProfileException('resolve', key: key);
        }
        return found;
      default:
        return (await resolveValue(key, raw, sourced.dir,
                confined: layer.kind.confined, lookup: lookup, readFile: home.readSecret))
            .expose();
    }
  }

  try {
    switch (server) {
      case McpStdio(:final command, :final args, :final env):
        final shown = [command, ...args].map((word) => word.contains(' ') ? '"$word"' : word).join(' ');
        String? program;
        if (isAbsolutePath(command)) {
          program = command;
        } else if (command.contains('/') || command.contains(r'\')) {
          final dir = sourced.dir;
          program = dir == null ? null : home.join(dir, command);
        } else {
          program = await findProgram(command);
        }
        final values = <String, String>{};
        for (final MapEntry(:key, value: raw) in env.entries) {
          final found = await value('mcp.$name.env.$key', key, raw);
          if (found != null) values[key] = found;
        }
        return PreparedMcp(
          name: name,
          layer: layer,
          shown: shown,
          digest: digest,
          isHttp: false,
          receives: env.keys.toList(),
          withheld: withheld,
          notes: [
            if (aos != null) "Runs in the session's working directory, not the overlay's: ACP gives an MCP server none",
            if (windows && program != null && RegExp(r'\.(cmd|bat)$', caseSensitive: false).hasMatch(program))
              'A .cmd or .bat, which an agent built on Node cannot start without a shell',
          ],
          refused: program == null ? '$command is not found on PATH' : null,
          server: program == null ? null : AcpMcpStdio(name: name, command: program, args: args, env: values),
        );
      case McpHttp(:final url, :final headers, :final sse):
        final values = <String, String>{};
        for (final MapEntry(:key, value: raw) in headers.entries) {
          final found = await value('mcp.$name.headers.$key', key, raw);
          if (found != null) values[key] = found;
        }
        return PreparedMcp(
          name: name,
          layer: layer,
          shown: '${sse ? 'SSE' : 'HTTP'} $url',
          digest: digest,
          isHttp: true,
          receives: headers.keys.toList(),
          withheld: withheld,
          notes: [if (url.startsWith('http://')) 'Plain HTTP: what it is sent can be read on the way'],
          server: AcpMcpHttp(name: name, url: url, headers: values, sse: sse),
        );
    }
  } on ProfileException catch (error) {
    return PreparedMcp(name: name, layer: layer, shown: _shown(server), digest: digest, isHttp: server is McpHttp,
        refused: '$error');
  } on AosException catch (error) {
    return PreparedMcp(name: name, layer: layer, shown: _shown(server), digest: digest, isHttp: server is McpHttp,
        refused: 'its text cannot be substituted (${error.code}: ${error.subject})');
  }
}

/// A server's command line or URL, as the person reads it.
String _shown(McpServer server) => switch (server) {
  McpStdio(:final command, :final args) => [command, ...args].join(' '),
  McpHttp(:final url, :final sse) => '${sse ? 'SSE' : 'HTTP'} $url',
};

/// Whether an MCP server starts when nothing was chosen for it: one of the person's own layers, unless canoa.cfg turns it
/// off for the agent; one from an overlay or a project, only once allowed for the agent at this very definition.
/// [remembered] is `[agents.NAME.mcp]`: a server's name to `false`, `true`, or the digest it was allowed at.
bool mcpOnByDefault(PreparedMcp server, Map<String, Object> remembered) {
  final choice = remembered[server.name];
  if (server.refused != null || choice == false) return false;
  if (server.confined) return choice == server.digest;
  return true;
}

/// What an agent says it takes with a session.
final class AgentTakes {
  const AgentTakes({required this.mcp, this.systemPrompt = false, this.additionalDirectories = false});
  final bool Function(AcpMcpTransport transport) mcp;
  final bool systemPrompt, additionalDirectories;
}

/// A profile as one harness takes it: the session's setup, what was written for it, and the report.
final class ProfileLaunch {
  const ProfileLaunch(this.setup, this.items, {this.temporary});
  final AcpSessionSetup setup;
  final List<ProfileItem> items;

  /// The directory written for this session, which goes when the session's agent does.
  final String? temporary;
}

String _contextNotTaken(String? harness) => switch (harness) {
  'pi' => 'pi-acp takes no system prompt; Pi reads SYSTEM.md and APPEND_SYSTEM.md only from its own directory',
  'gemini-cli' => 'Gemini CLI takes no system prompt over ACP; GEMINI_SYSTEM_MD would replace its whole prompt',
  'opencode' => 'OpenCode takes instructions only from its configuration, when it starts',
  'grok' => 'Grok takes a prompt only from an agent profile given when it starts',
  _ => 'the agent does not say it takes a system prompt',
};

String _skillsNotTaken(String? harness) => switch (harness) {
  'pi' => 'pi-acp passes Pi no --skill; Pi reads skills only from its own and the project\'s directories',
  'gemini-cli' => 'Gemini CLI reads skills only from its own, your home\'s and the project\'s skill directories',
  'opencode' => 'OpenCode reads skills only from its configuration and the project\'s directories',
  'grok' => 'Grok reads skills only from its own, your home\'s and the project\'s skill directories',
  'codex' => 'the adapter does not say it takes more directories, where Codex would read them',
  _ => 'ACP has no field for skills, and Canoa knows no other way this agent takes them',
};

/// [prepared] as [harness] takes it, for an agent that says it [takes] what it does. Only the MCP servers named in
/// [mcpOn] start; the rest are reported off. Skills a harness reads from a directory are written through [files].
Future<ProfileLaunch> launchFor(
  PreparedProfile prepared, {
  required String? harness,
  required AgentTakes takes,
  required Set<String> mcpOn,
  SessionFiles? files,
}) async {
  final items = <ProfileItem>[...prepared.problems];
  for (final MapEntry(:key, :value) in prepared.environmentLayers.entries) {
    if (prepared.environment.containsKey(key)) {
      items.add(ProfileItem('env.$key', '$value', ItemState.applied, "the agent's process environment"));
    } else {
      items.add(ProfileItem('env.$key', '$value', ItemState.off, 'an optional reference that is not set'));
    }
  }

  String? systemPrompt;
  if (prepared.contexts.isNotEmpty) {
    final taken = takes.systemPrompt;
    if (taken) systemPrompt = prepared.contexts.map((context) => context.text).join('\n\n');
    for (final context in prepared.contexts) {
      items.add(ProfileItem(
        'context',
        '${context.layer}',
        taken ? ItemState.applied : ItemState.unsupported,
        taken ? 'ACP systemPrompt, appended (${context.source})' : _contextNotTaken(harness),
      ));
    }
  }

  final written = <String, List<int>>{};
  final usable = prepared.skills.where((skill) => skill.refused == null).toList();
  for (final skill in prepared.skills.where((skill) => skill.refused != null)) {
    items.add(ProfileItem('skills.${skill.name}', '${skill.layer}', ItemState.refused, skill.refused!));
  }
  String? temporary;
  Map<String, Object?>? meta;
  final directories = <String>[];
  if (usable.isNotEmpty) {
    final (prefix, how) = switch (harness) {
      'claude-code' => ('$claudePluginName/skills', "the Claude plugin $claudePluginName, through the adapter's _meta"),
      'codex' when takes.additionalDirectories =>
        (codexSkillsPath, 'a directory of skills Codex reads, through ACP additionalDirectories'),
      _ => (null, ''),
    };
    if (prefix == null || files == null) {
      for (final skill in usable) {
        items.add(ProfileItem('skills.${skill.name}', '${skill.layer}', ItemState.unsupported,
            files == null ? 'nothing can be written for it here' : _skillsNotTaken(harness)));
      }
    } else {
      for (final skill in usable) {
        skill.files.forEach((path, bytes) => written['$prefix/${skill.name}/$path'] = bytes);
        items.add(ProfileItem('skills.${skill.name}', '${skill.layer}', ItemState.applied,
            harness == 'claude-code' ? '$how, as $claudePluginName:${skill.name}' : how));
      }
      if (harness == 'claude-code') {
        written['$claudePluginName/.claude-plugin/plugin.json'] = utf8.encode(jsonEncode({
          'name': claudePluginName,
          'version': '0.0.0',
          'description': "The skills of Canoa's profile for this session",
        }));
      }
      temporary = await files.write(written);
      if (harness == 'claude-code') {
        meta = {
          'claudeCode': {
            'options': {
              'plugins': [
                {'type': 'local', 'path': '$temporary${_separator(temporary)}$claudePluginName'},
              ],
            },
          },
        };
      } else {
        directories.add(temporary);
      }
    }
  }

  final servers = <AcpMcpServer>[];
  for (final server in prepared.mcp) {
    final key = 'mcp.${server.name}', layer = '${server.layer}';
    final resolved = server.server;
    if (server.refused != null || resolved == null) {
      items.add(ProfileItem(key, layer, ItemState.refused, server.refused ?? 'it cannot be resolved'));
    } else if (!mcpOn.contains(server.name)) {
      items.add(ProfileItem(key, layer, ItemState.off, server.confined
          ? 'not allowed for this session' : 'turned off for this session'));
    } else if (harness == 'pi') {
      items.add(ProfileItem(key, layer, ItemState.unsupported, 'pi-acp accepts mcpServers and starts none of them'));
    } else if (!takes.mcp(resolved.transport)) {
      items.add(ProfileItem(key, layer, ItemState.unsupported,
          'the agent does not say it starts MCP servers over ${resolved.transport.name}'));
    } else {
      servers.add(resolved);
      final said = [
        'ACP mcpServers',
        if (harness == 'codex') 'Codex skips it if its own configuration names a server ${server.name}',
        if (harness == 'opencode') 'OpenCode keeps it for this directory while it runs',
        if (server.withheld.isNotEmpty) 'left out: ${server.withheld.join(', ')}',
      ];
      items.add(ProfileItem(key, layer, ItemState.applied, said.join('; ')));
    }
  }

  for (final (key, value) in [('model', prepared.model), ('effort', prepared.effort), ('mode', prepared.mode)]) {
    if (value != null) {
      items.add(ProfileItem(key, '${value.layer}', ItemState.unsupported, 'set once the session is open'));
    }
  }

  return ProfileLaunch(
    AcpSessionSetup(
      mcpServers: servers,
      systemPrompt: systemPrompt,
      additionalDirectories: directories,
      meta: meta,
    ),
    items,
    temporary: temporary,
  );
}

String _separator(String path) => path.contains(r'\') ? r'\' : '/';

/// The session's choices for the profile's model, effort and mode: each config option, by its id or category, that offers
/// the value. [options] are the session's, as (id, category, values, isMode).
List<(String id, String value, String key)> choicesFor(
  PreparedProfile prepared,
  List<(String id, String? category, List<String> values, bool isMode)> options,
) {
  const ids = {
    'model': ['model'],
    'effort': ['effort', 'reasoning_effort', 'thought_level'],
    'mode': ['mode'],
  };
  const categories = {'model': 'model', 'effort': 'thought_level', 'mode': 'mode'};
  final out = <(String, String, String)>[];
  for (final (key, value) in [('model', prepared.model), ('effort', prepared.effort), ('mode', prepared.mode)]) {
    if (value == null) continue;
    final option = options.where((option) =>
        (option.$2 == categories[key] || ids[key]!.contains(option.$1) || (key == 'mode' && option.$4)) &&
        option.$3.contains(value.value)).firstOrNull;
    if (option != null) out.add((option.$1, value.value, key));
  }
  return out;
}
