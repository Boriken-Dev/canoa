import 'dart:convert' show jsonEncode;

import 'package:flutter/material.dart' show ThemeMode;
import 'package:toml/toml.dart';

import '../acp/agent_commands.dart';
import '../ledger/session_ledger.dart';
import '../profile/aos.dart' show normalizeName, validOverlayName;
import '../profile/aos_catalog.dart' show holdsPassword;
import '../profile/profile.dart';
import '../store/store_settings.dart';
import '../theme/look.dart';

/// One entry of `[host]`, as canoa.cfg has it.
final class HostEntry {
  const HostEntry({required this.name, required this.kind, required this.runs, this.args = const [], this.listen});

  factory HostEntry.read(String name, String kind, Map<dynamic, dynamic> table) => HostEntry(
    name: name,
    kind: kind,
    runs: '${table[kind == 'agent' ? 'program' : 'library'] ?? ''}',
    args: [if (table['args'] case final List<dynamic> args) for (final arg in args) '$arg'],
    listen: table['listen'] is String ? table['listen'] as String : null,
  );

  /// canoa's rule for an entry's name.
  static bool isName(String name) => RegExp(r'^[A-Za-z0-9_-]{1,64}$').hasMatch(name);

  final String name;

  /// `agent` or `provider`.
  final String kind;

  /// The program an agent runs, or a provider's library.
  final String runs;
  final List<String> args;

  /// The address it listens on, when the file names one.
  final String? listen;
}

/// `[notifications]`: which kinds of news from a session reach the person outside the window. Each is on by default.
final class NotificationSettings {
  const NotificationSettings({this.permission = true, this.question = true, this.finished = true});

  /// A permission request, a question from the agent, and a turn that ended.
  final bool permission, question, finished;

  NotificationSettings copyWith({bool? permission, bool? question, bool? finished}) => NotificationSettings(
    permission: permission ?? this.permission,
    question: question ?? this.question,
    finished: finished ?? this.finished,
  );
}

/// Deployment config uses TOML in canoa.cfg, consistent with AI6's OO.cfg.
/// This file must never contain secrets or private keys.
final class CanoaConfig {
  const CanoaConfig({
    this.startHidden = false,
    this.closeToTray = true,
    this.minimizeToTray = false,
    this.localHost = true,
    this.preferWebTransport = true,
    this.connectTimeout = 10,
    this.cdn = 'none',
    this.look = LookId.bubbles,
    this.brightness = 'dark',
    this.agents = const [],
    this.host,
    this.sessions = const LedgerSettings(),
    this.overlays = const {},
    this.profile = const Profile(),
    this.agentProfiles = const {},
    this.tables = const {},
    this.fullAccess = const {},
    this.notifications = const NotificationSettings(),
    this.hiddenSources = const {},
    this.pasteAsFile = defaultPasteAsFile,
    this.embedLimit = defaultEmbedLimit,
  });

  /// `[attachments] paste_as_file`: the most characters a paste puts in the message; a longer one goes as an attachment.
  final int pasteAsFile;
  static const defaultPasteAsFile = 8000;

  /// `[attachments] embed_limit`: the largest file, in bytes, sent as the agent's embedded context; a larger one goes as a
  /// link to it.
  final int embedLimit;
  static const defaultEmbedLimit = 1024 * 1024;
  final bool startHidden, closeToTray, minimizeToTray, preferWebTransport;

  /// `[desktop] local_host`: whether the desktop app runs its own host.
  final bool localHost;
  final int connectTimeout;
  final String cdn;
  final LookId look;

  /// Agents this person added, offered beside the presets when what they need is installed.
  final List<AgentCommand> agents;

  /// The `[host]` table, which `canoa` reads: kept as it was read, so saving settings never loses it.
  final Map<String, dynamic>? host;

  /// `[sessions]` and `[harnesses]`: how much the session list keeps, and the harness each reported agent name stands for.
  final LedgerSettings sessions;

  /// `[overlays]`: each named overlay's source, in the file's order, which is the order they layer in.
  final Map<String, OverlaySource> overlays;

  /// `[profile]`: the person's own layer.
  final Profile profile;

  /// `[agents.NAME.profile]`: each agent entry's layer, by the agent's label; an entry with no program names a preset.
  final Map<String, Profile> agentProfiles;

  /// `[overlays]`, `[profile]` and each `[agents.NAME.profile]` as they were read, so saving writes them back unchanged.
  final Map<String, Object?> tables;

  /// `[permissions] full_access`: the agents, by label, whose sessions start with full access, which answers each permission request
  /// that offers to allow once with that option and no question. Empty by default.
  final Set<String> fullAccess;

  final NotificationSettings notifications;

  /// `[sources] hidden`: the presets, by label, that Settings keeps out of the new-session picker.
  final Set<String> hiddenSources;

  /// `light`, `dark`, or `system` to follow the machine.
  final String brightness;

  ThemeMode get themeMode => switch (brightness) {
    'light' => ThemeMode.light,
    'system' => ThemeMode.system,
    _ => ThemeMode.dark,
  };

  CanoaConfig copyWith({bool? startHidden, bool? closeToTray,
      bool? minimizeToTray, bool? preferWebTransport, int? connectTimeout,
      LookId? look, String? brightness, LedgerSettings? sessions, Set<String>? fullAccess,
      NotificationSettings? notifications, List<AgentCommand>? agents, Set<String>? hiddenSources}) =>
      CanoaConfig(
        startHidden: startHidden ?? this.startHidden,
        closeToTray: closeToTray ?? this.closeToTray,
        minimizeToTray: minimizeToTray ?? this.minimizeToTray,
        localHost: localHost,
        preferWebTransport: preferWebTransport ?? this.preferWebTransport,
        connectTimeout: connectTimeout ?? this.connectTimeout,
        cdn: cdn,
        look: look ?? this.look,
        brightness: brightness ?? this.brightness,
        agents: agents ?? this.agents,
        host: host,
        sessions: sessions ?? this.sessions,
        overlays: overlays,
        profile: profile,
        agentProfiles: agentProfiles,
        tables: tables,
        fullAccess: fullAccess ?? this.fullAccess,
        notifications: notifications ?? this.notifications,
        hiddenSources: hiddenSources ?? this.hiddenSources,
        pasteAsFile: pasteAsFile,
        embedLimit: embedLimit,
      );

  /// This configuration with [host] as its `[host]` table.
  CanoaConfig withHost(Map<String, dynamic>? host) => CanoaConfig(
    startHidden: startHidden,
    closeToTray: closeToTray,
    minimizeToTray: minimizeToTray,
    localHost: localHost,
    preferWebTransport: preferWebTransport,
    connectTimeout: connectTimeout,
    cdn: cdn,
    look: look,
    brightness: brightness,
    agents: agents,
    host: host,
    sessions: sessions,
    overlays: overlays,
    profile: profile,
    agentProfiles: agentProfiles,
    tables: tables,
    fullAccess: fullAccess,
    notifications: notifications,
    hiddenSources: hiddenSources,
    pasteAsFile: pasteAsFile,
    embedLimit: embedLimit,
  );

  /// What `[host]` names for canoa to serve, agents first, as written. canoa itself checks each one.
  List<HostEntry> get hostEntries => [
    for (final (table, kind) in const [('agents', 'agent'), ('providers', 'provider')])
      if (host?[table] case final Map<dynamic, dynamic> entries)
        for (final MapEntry(:key, :value) in entries.entries)
          if (value is Map) HostEntry.read('$key', kind, value),
  ];

  /// This configuration with a stdio ACP agent added to `[host]`, which canoa serves on [listen].
  CanoaConfig withHostAgent({
    required String name,
    required String program,
    List<String> args = const [],
    required String listen,
    List<String> workingRoots = const [],
  }) {
    if (!HostEntry.isName(name)) {
      throw ArgumentError.value(name, 'name', 'Use 1 to 64 letters, digits, - or _');
    }
    if (hostEntries.any((entry) => entry.name == name)) {
      throw ArgumentError.value(name, 'name', 'Another entry has that name');
    }
    if (program.trim().isEmpty) throw ArgumentError.value(program, 'program', 'Name a program');
    final copy = _deepCopy(host ?? const {});
    final table = (copy['agents'] as Map<String, dynamic>?) ?? <String, dynamic>{};
    table[name] = <String, dynamic>{
      'program': program,
      if (args.isNotEmpty) 'args': args,
      'listen': listen,
      if (workingRoots.isNotEmpty) 'working_roots': workingRoots,
    };
    copy['agents'] = table;
    return withHost(copy);
  }

  /// This configuration without the `[host]` entry [name].
  CanoaConfig withoutHostEntry(String name) {
    final copy = _deepCopy(host ?? const {});
    for (final table in const ['agents', 'providers']) {
      (copy[table] as Map<String, dynamic>?)?.remove(name);
    }
    return withHost(copy);
  }

  /// This configuration with the overlay [name] pinned to the tree digest [digest]: its entry becomes
  /// `{ source = "...", pin = "..." }`, or keeps its `dir` beside the pin.
  CanoaConfig withOverlayPin(String name, String digest) {
    final source = overlays[name];
    if (source == null) throw ArgumentError.value(name, 'name', 'No overlay of that name');
    final raw = _deepCopy((tables['overlays'] as Map?) ?? const {});
    raw[name] = switch (source) {
      GitOverlay() => <String, dynamic>{'source': source.spec, 'pin': digest},
      DirOverlay(:final dir) => <String, dynamic>{'dir': dir, 'pin': digest},
    };
    return withProfileTables(
      overlays: {
        for (final MapEntry(:key, :value) in overlays.entries)
          key: key != name
              ? value
              : switch (value) {
                  GitOverlay() => value.pinnedTo(digest),
                  DirOverlay(:final dir) => DirOverlay(dir, pin: digest),
                },
      },
      tables: {...tables, 'overlays': raw},
    );
  }

  /// `[catalogs]`: each catalog the store lists, by the label the person gives it, a git source whose `#path` names the
  /// catalog file.
  Map<String, GitOverlay> get catalogs => _catalogs(tables['catalogs']);

  /// `[store]`: the store's limits.
  StoreSettings get store => StoreSettings.parse((tables['store'] as Map?) ?? const {});

  /// `[agents.NAME.mcp]`: for each agent, by label, the MCP servers the person turned on (`true`) or off (`false`), and
  /// those of an overlay or a project allowed at one definition (its digest, `sha256:...`).
  Map<String, Map<String, Object>> get agentMcp => _agentMcp(tables['agents']);

  static Map<String, Map<String, Object>> _agentMcp(Object? agents) {
    final out = <String, Map<String, Object>>{};
    if (agents is! Map) return out;
    for (final MapEntry(key: agent, :value) in agents.entries) {
      final table = value is Map ? value['mcp'] : null;
      if (table == null) continue;
      if (table is! Map) throw FormatException('canoa.cfg: agents.$agent.mcp must be a table');
      final choices = <String, Object>{};
      for (final MapEntry(:key, value: choice) in table.entries) {
        if (!isEntryName('$key') ||
            !(choice is bool || (choice is String && RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(choice)))) {
          throw FormatException('canoa.cfg: agents.$agent.mcp.$key is true, false, or the sha256: digest of what was '
              'allowed');
        }
        choices['$key'] = choice as Object;
      }
      out['$agent'] = choices;
    }
    return out;
  }

  /// This configuration with [choices] as `[agents.NAME.mcp]` of [agent], as [agentMcp] reads them.
  CanoaConfig withAgentMcp(String agent, Map<String, Object> choices) {
    final raw = _deepCopy((tables['agents'] as Map?) ?? const {});
    final entry = (raw[agent] as Map<String, dynamic>?) ?? <String, dynamic>{};
    if (choices.isEmpty) {
      entry.remove('mcp');
    } else {
      entry['mcp'] = <String, dynamic>{...choices};
    }
    if (entry.isEmpty) {
      raw.remove(agent);
    } else {
      raw[agent] = entry;
    }
    _agentMcp(raw);
    return _withTable('agents', raw);
  }

  static Map<String, GitOverlay> _catalogs(Object? raw) {
    if (raw == null) return const {};
    if (raw is! Map) throw const FormatException('canoa.cfg: catalogs must be a table');
    final out = <String, GitOverlay>{};
    final normalized = <String>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      final label = '$key';
      if (!isCatalogLabel(label) || !normalized.add(normalizeName(label))) {
        throw FormatException('canoa.cfg: catalogs.$label: a label is a letter, then letters, digits, - or _, at most '
            '64, unique when compared as AOS compares names');
      }
      out[label] = switch (value) {
        final String spec => _catalogSource(label, spec),
        _ => throw FormatException('canoa.cfg: catalogs.$label is a git source, <repository>@<ref>#<catalog file>'),
      };
    }
    return out;
  }

  /// Whether [label] can name a catalog: a valid overlay name that is also a name Canoa gives a directory.
  static bool isCatalogLabel(String label) => isEntryName(label) && validOverlayName(label);

  static GitOverlay _catalogSource(String label, String spec) {
    final GitOverlay source;
    try {
      source = OverlaySource.parseGit(spec);
    } on ProfileException {
      throw FormatException('canoa.cfg: catalogs.$label is not <repository>@<ref>#<catalog file> of a source git may '
          'fetch');
    }
    if (source.path.isEmpty) throw FormatException('canoa.cfg: catalogs.$label names no catalog file after #');
    if (holdsPassword(source.repo)) {
      throw FormatException('canoa.cfg: catalogs.$label holds a password, which canoa.cfg may not; use a credential '
          'helper');
    }
    return source;
  }

  CanoaConfig _withTable(String name, Map<String, dynamic> table, {Map<String, OverlaySource>? overlays}) =>
      withProfileTables(overlays: overlays ?? this.overlays, tables: {
        for (final MapEntry(:key, :value) in tables.entries)
          if (key != name) key: value,
        if (table.isNotEmpty) name: table,
      });

  /// This configuration with the catalog [spec] labelled [label] in `[catalogs]`. Throws [FormatException].
  CanoaConfig withCatalog(String label, String spec) {
    final raw = _deepCopy((tables['catalogs'] as Map?) ?? const {})..[label] = spec;
    _catalogs(raw);
    return _withTable('catalogs', raw);
  }

  /// This configuration without the catalog [label].
  CanoaConfig withoutCatalog(String label) =>
      _withTable('catalogs', _deepCopy((tables['catalogs'] as Map?) ?? const {})..remove(label));

  /// This configuration with the overlay [name] from [source], pinned to [pin]: `NAME = { source, pin }`.
  CanoaConfig withOverlay(String name, GitOverlay source, String pin) {
    if (!isEntryName(name)) throw ArgumentError.value(name, 'name', 'An overlay name has 1 to 64 letters, digits, - or _');
    final raw = _deepCopy((tables['overlays'] as Map?) ?? const {})
      ..[name] = <String, dynamic>{'source': source.spec, 'pin': pin};
    return _withTable('overlays', raw, overlays: {...overlays, name: source.pinnedTo(pin)});
  }

  /// This configuration without the overlay [name] in `[overlays]`.
  CanoaConfig withoutOverlay(String name) => _withTable('overlays',
      _deepCopy((tables['overlays'] as Map?) ?? const {})..remove(name),
      overlays: {for (final MapEntry(:key, :value) in overlays.entries) if (key != name) key: value});

  /// This configuration with another's `[overlays]`, `[profile]` and agent profiles, as they are now.
  CanoaConfig withProfilesOf(CanoaConfig other) =>
      withProfileTables(overlays: other.overlays, tables: other.tables, profile: other.profile,
          agentProfiles: other.agentProfiles);

  CanoaConfig withProfileTables({required Map<String, OverlaySource> overlays, required Map<String, Object?> tables,
      Profile? profile, Map<String, Profile>? agentProfiles}) => CanoaConfig(
    startHidden: startHidden,
    closeToTray: closeToTray,
    minimizeToTray: minimizeToTray,
    localHost: localHost,
    preferWebTransport: preferWebTransport,
    connectTimeout: connectTimeout,
    cdn: cdn,
    look: look,
    brightness: brightness,
    agents: agents,
    host: host,
    sessions: sessions,
    overlays: overlays,
    profile: profile ?? this.profile,
    agentProfiles: agentProfiles ?? this.agentProfiles,
    tables: tables,
    fullAccess: fullAccess,
    notifications: notifications,
    hiddenSources: hiddenSources,
    pasteAsFile: pasteAsFile,
    embedLimit: embedLimit,
  );

  static Map<String, dynamic> _deepCopy(Map<dynamic, dynamic> map) => {
    for (final MapEntry(:key, :value) in map.entries)
      '$key': switch (value) {
        final Map<dynamic, dynamic> inner => _deepCopy(inner),
        final List<dynamic> list => List<dynamic>.of(list),
        _ => value,
      },
  };

  factory CanoaConfig.parse(String source) {
    final content = source.startsWith('#!toml')
        ? source.substring(source.indexOf('\n') + 1) : source;
    final data = TomlDocument.parse(content).toMap();
    for (final name in data.keys) {
      if (!{'desktop', 'network', 'assets', 'appearance', 'agents', 'host', 'sessions', 'harnesses', 'overlays', 'profile',
            'permissions', 'notifications', 'catalogs', 'store', 'sources', 'attachments'}
          .contains(name)) {
        throw FormatException('canoa.cfg: unknown table $name');
      }
    }
    Map<String, dynamic> table(String name, Set<String> allowed) {
      final raw = data[name];
      if (raw == null) return {};
      if (raw is! Map) throw FormatException('canoa.cfg: expected a table');
      final entries = Map<String, dynamic>.from(raw);
      for (final key in entries.keys) {
        if (!allowed.contains(key)) throw FormatException('canoa.cfg: invalid key $key');
      }
      return entries;
    }
    final desktop = table('desktop', {'start_hidden','close_to_tray','minimize_to_tray','local_host'});
    final network = table('network', {'prefer_webtransport','connect_timeout'});
    final assets = table('assets', {'cdn'});
    final appearance = table('appearance', {'look', 'brightness'});
    final sessions = table('sessions', {'keep', 'keep_text', 'text_messages'});
    final permissions = table('permissions', {'full_access'});
    final notifications = table('notifications', {'permission', 'question', 'finished'});
    final sources = table('sources', {'hidden'});
    final attachments = table('attachments', {'paste_as_file', 'embed_limit'});
    int limit(String key, int fallback) {
      final raw = attachments[key] ?? fallback;
      if (raw is! int || raw < 0) throw FormatException('canoa.cfg: attachments.$key must be a whole number, 0 or more');
      return raw;
    }
    final hidden = sources['hidden'] ?? const <Object>[];
    if (hidden is! List || hidden.any((name) => name is! String || name.trim().isEmpty)) {
      throw const FormatException('canoa.cfg: sources.hidden must be a list of preset names');
    }
    final fullAccess = permissions['full_access'] ?? const <Object>[];
    if (fullAccess is! List || fullAccess.any((name) => name is! String || name.trim().isEmpty)) {
      throw const FormatException('canoa.cfg: permissions.full_access must be a list of agent names');
    }
    bool flag(Map<String, dynamic> map, String key, bool fallback) {
      final raw = map[key];
      if (raw == null) return fallback;
      if (raw is! bool) throw FormatException('canoa.cfg: invalid boolean $key');
      return raw;
    }
    final timeout = network['connect_timeout'] ?? 10;
    if (timeout is! int || timeout < 1 || timeout > 120) {
      throw const FormatException('canoa.cfg: connect_timeout must be 1..120');
    }
    final cdn = assets['cdn'] ?? 'none';
    if (cdn is! String || !{'none', 'local'}.contains(cdn)) {
      throw const FormatException('canoa.cfg: assets.cdn must be none or local');
    }
    final lookName = appearance['look'] ?? LookId.bubbles.name;
    final look = lookName is String ? LookId.named(lookName) : null;
    if (look == null) {
      throw FormatException('canoa.cfg: appearance.look must be one of ${LookId.values.map((id) => id.name).join(', ')}');
    }
    final brightness = appearance['brightness'] ?? 'dark';
    if (brightness is! String || !{'light', 'dark', 'system'}.contains(brightness)) {
      throw const FormatException('canoa.cfg: appearance.brightness must be light, dark or system');
    }
    final agents = <AgentCommand>[];
    final agentProfiles = <String, Profile>{};
    final agentTables = <String, Object?>{};
    final rawAgents = data['agents'];
    if (rawAgents != null) {
      if (rawAgents is! Map) throw const FormatException('canoa.cfg: agents must be a table of agents');
      for (final entry in rawAgents.entries) {
        final agent = entry.value;
        if (agent is Map && (agent['profile'] != null || agent['mcp'] != null)) {
          final table = agent['profile'];
          if (table != null && table is! Map) throw FormatException('canoa.cfg: agents.${entry.key}.profile must be a table');
          if (table is Map) agentProfiles['${entry.key}'] = _profile('agents.${entry.key}.profile', table, LayerKind.agent);
          agentTables['${entry.key}'] = {'profile': ?table, 'mcp': ?agent['mcp']};
          if (!agent.containsKey('program') && agent.keys.every(const {'profile', 'mcp'}.contains)) continue;
        }
        if (agent is! Map || agent['program'] is! String || (agent['program'] as String).isEmpty) {
          throw FormatException('canoa.cfg: agents.${entry.key} needs a program');
        }
        for (final key in agent.keys) {
          if (key != 'program' && key != 'args' && key != 'profile' && key != 'mcp') {
            throw FormatException('canoa.cfg: agents.${entry.key}: invalid key $key');
          }
        }
        final args = agent['args'] ?? const <Object>[];
        if (args is! List || args.any((arg) => arg is! String)) {
          throw FormatException('canoa.cfg: agents.${entry.key}.args must be a list of strings');
        }
        agents.add(AgentCommand(label: '${entry.key}', program: agent['program'] as String, args: args.cast<String>()));
      }
    }
    final host = data['host'];
    if (host != null && host is! Map) throw const FormatException('canoa.cfg: host must be a table');
    int count(String key, int fallback) {
      final raw = sessions[key] ?? fallback;
      if (raw is! int || raw < 0) throw FormatException('canoa.cfg: sessions.$key must be a whole number, 0 or more');
      return raw;
    }
    final harnesses = data['harnesses'] ?? const <String, Object?>{};
    if (harnesses is! Map || harnesses.values.any((label) => label is! String || label.trim().isEmpty)) {
      throw const FormatException('canoa.cfg: harnesses must map each reported agent name to a label');
    }
    final overlays = <String, OverlaySource>{};
    final rawOverlays = data['overlays'] ?? const <String, Object?>{};
    if (rawOverlays is! Map) throw const FormatException('canoa.cfg: overlays must be a table');
    for (final MapEntry(:key, :value) in rawOverlays.entries) {
      if (!isEntryName('$key')) {
        throw const FormatException('canoa.cfg: an overlay name has 1 to 64 letters, digits, - or _');
      }
      GitOverlay git(String spec) {
        try {
          return OverlaySource.parseGit(spec);
        } on ProfileException {
          throw FormatException('canoa.cfg: overlays.$key is not <repository>@<ref>[#<path>] of a source git may fetch');
        }
      }
      String? pin(Map<dynamic, dynamic> entry) => switch (entry['pin']) {
        null => null,
        final String digest when RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(digest) => digest,
        _ => throw FormatException('canoa.cfg: overlays.$key.pin is sha256: and 64 hexadecimal digits'),
      };
      overlays['$key'] = switch (value) {
        final String spec => git(spec),
        {'source': final String spec} when value.keys.every(const {'source', 'pin'}.contains) =>
          git(spec).pinnedTo(pin(value)),
        {'dir': final String dir} when dir.isNotEmpty && value.keys.every(const {'dir', 'pin'}.contains) =>
          DirOverlay(dir, pin: pin(value)),
        _ => throw FormatException('canoa.cfg: overlays.$key is a git source, { source, pin } or { dir, pin }'),
      };
    }
    final rawProfile = data['profile'];
    if (rawProfile != null && rawProfile is! Map) throw const FormatException('canoa.cfg: profile must be a table');
    _catalogs(data['catalogs']);
    _agentMcp(agentTables);
    final rawStore = data['store'];
    if (rawStore != null && rawStore is! Map) throw const FormatException('canoa.cfg: store must be a table');
    if (rawStore is Map) StoreSettings.parse(rawStore);
    return CanoaConfig(
      overlays: overlays,
      profile: rawProfile == null ? const Profile() : _profile('profile', rawProfile as Map, LayerKind.user),
      agentProfiles: agentProfiles,
      tables: {
        if (data['overlays'] != null) 'overlays': rawOverlays,
        'profile': ?rawProfile,
        if (agentTables.isNotEmpty) 'agents': agentTables,
        'catalogs': ?data['catalogs'],
        'store': ?rawStore,
      },
      agents: agents,
      host: host == null ? null : Map<String, dynamic>.from(host as Map),
      look: look, brightness: brightness,
      startHidden: flag(desktop, 'start_hidden', false),
      closeToTray: flag(desktop, 'close_to_tray', true),
      minimizeToTray: flag(desktop, 'minimize_to_tray', false),
      localHost: flag(desktop, 'local_host', true),
      preferWebTransport: flag(network, 'prefer_webtransport', true),
      connectTimeout: timeout, cdn: cdn,
      fullAccess: {for (final name in fullAccess) name as String},
      hiddenSources: {for (final name in hidden) name as String},
      pasteAsFile: limit('paste_as_file', defaultPasteAsFile),
      embedLimit: limit('embed_limit', defaultEmbedLimit),
      notifications: NotificationSettings(
        permission: flag(notifications, 'permission', true),
        question: flag(notifications, 'question', true),
        finished: flag(notifications, 'finished', true),
      ),
      sessions: LedgerSettings(
        keep: count('keep', LedgerSettings.defaultKeep),
        keepText: flag(sessions, 'keep_text', false),
        textMessages: count('text_messages', LedgerSettings.defaultTextMessages),
        harnesses: {for (final MapEntry(:key, :value) in harnesses.entries) '$key': value as String},
      ),
    );
  }

  String encode() => '''
# Canoa deployment settings — Boriken Dev
[desktop]
start_hidden = $startHidden
close_to_tray = $closeToTray
minimize_to_tray = $minimizeToTray
local_host = $localHost

[network]
prefer_webtransport = $preferWebTransport
connect_timeout = $connectTimeout

[assets]
cdn = "$cdn"

[appearance]
look = "${look.name}"
brightness = "$brightness"

[sessions]
keep = ${sessions.keep}
keep_text = ${sessions.keepText}
text_messages = ${sessions.textMessages}

[notifications]
permission = ${notifications.permission}
question = ${notifications.question}
finished = ${notifications.finished}

[attachments]
paste_as_file = $pasteAsFile
embed_limit = $embedLimit
${fullAccess.isEmpty ? '' : '\n[permissions]\nfull_access = ${jsonEncode(fullAccess.toList())}\n'}${hiddenSources.isEmpty ? '' : '\n[sources]\nhidden = ${jsonEncode(hiddenSources.toList())}\n'}${sessions.harnesses.isEmpty ? '' : '\n[harnesses]\n${sessions.harnesses.entries.map((name) => '${jsonEncode(name.key)} = ${jsonEncode(name.value)}\n').join()}'}${agents.map((agent) => '''
[agents.${jsonEncode(agent.label)}]
program = ${jsonEncode(agent.program)}
args = ${jsonEncode(agent.args)}
''').join()}${tables.isEmpty ? '' : '\n${TomlDocument.fromMap(tables)}'}${host == null ? '' : '\n${TomlDocument.fromMap({'host': host})}'}''';

  /// A profile table of canoa.cfg, its refusal named by [where] and never quoting a value.
  static Profile _profile(String where, Map<dynamic, dynamic> table, LayerKind kind) {
    try {
      return Profile.fromMap(table, kind);
    } on ProfileException catch (error) {
      throw FormatException('canoa.cfg: $where: $error');
    }
  }
}
