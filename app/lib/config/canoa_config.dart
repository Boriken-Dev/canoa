import 'dart:convert' show jsonEncode;

import 'package:coqui_aos/coqui_aos.dart' show normalizeName, validOverlayName;
import 'package:flutter/material.dart' show ThemeMode;
import 'package:toml/toml.dart';

import '../accounts/account_models.dart' show AccountEntry;
import '../acp/agent_commands.dart';
import 'config_layers.dart';
import 'connection.dart';
import '../chat/modes.dart' show modeOrder;
import '../ledger/session_ledger.dart';
import '../profile/aos_support.dart' show holdsPassword;
import '../permissions/protected_paths.dart' show protectedLinesOf;
import '../profile/skill_copies.dart' show skillKey, skillsDirectory, validSkillName;
import '../profile/profile.dart';
import '../store/store_settings.dart';
import '../theme/look.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity, defaultWarnPermille;
import '../workspace/workspace_template.dart' show defaultWorkspace, workspaceProblem;

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

/// How a dictation is held: one press starts it and the next ends it, or it lasts while a key is down.
enum DictationMode { toggle, hold }

/// `[voice]`: where speech is turned into text and how. Nothing is recorded until the person starts a dictation.
final class VoiceSettings {
  const VoiceSettings({this.backend = defaultVoiceBackend, this.model, this.language, this.device, this.mode = DictationMode.toggle});

  /// The speech service, by the id it has in the app; [defaultVoiceBackend] is the operating system's.
  final String backend;

  /// The service's model where it has several; null is the service's own choice.
  final String? model;

  /// A language tag such as `es-PR`; null is automatic where the service can tell, else the system's.
  final String? language;

  /// The input device by the id the service gives it; null is the system's default.
  final String? device;
  final DictationMode mode;

  VoiceSettings copyWith({String? backend, String? model, bool clearModel = false, String? language, bool clearLanguage = false,
      String? device, bool clearDevice = false, DictationMode? mode}) => VoiceSettings(
    backend: backend ?? this.backend,
    model: clearModel ? null : model ?? this.model,
    language: clearLanguage ? null : language ?? this.language,
    device: clearDevice ? null : device ?? this.device,
    mode: mode ?? this.mode,
  );
}

/// The id of the operating system's speech service, which `[voice] backend` names unless the person chose another.
const defaultVoiceBackend = 'system';

/// Where a [CanoaConfig] was read from: the format and first-line marker of the person's own file, the layers below it (the
/// installed copy and the system's file, lowest first), and the person's file as it was read. A save writes the person's
/// file only, in its own format.
final class ConfigSource {
  const ConfigSource({this.format = ConfigFormat.toml, this.marked = false, this.layers = const [], this.own = const {}});

  final ConfigFormat format;

  /// Whether the file's first line names its format, so that it is written back with it.
  final bool marked;

  /// The layers below the person's file, lowest first.
  final List<ConfigLayer> layers;

  /// The person's file as read.
  final Map<String, dynamic> own;

  /// This source with the person's file holding [value] at the top-level [key], or none when [value] is null: what a save of one
  /// table by another part of the app leaves in it.
  ConfigSource withOwn(String key, Object? value) {
    final next = {...own};
    if (value == null) {
      next.remove(key);
    } else {
      next[key] = value;
    }
    return ConfigSource(format: format, marked: marked, layers: layers, own: next);
  }

  /// The layers below the person's file, merged.
  Map<String, dynamic> get lower {
    var merged = <String, dynamic>{};
    for (final layer in layers) {
      merged = mergeConfig(merged, layer.data);
    }
    return merged;
  }
}

/// canoa.cfg is one file in TOML, YAML or JSON, named by its first line, found and merged by layers (see
/// `config_layers.dart`). This file must never contain secrets or private keys.
final class CanoaConfig {
  const CanoaConfig({
    this.startHidden = false,
    this.closeToTray = true,
    this.minimizeToTray = false,
    this.localHost = true,
    this.preferWebTransport = true,
    this.connectTimeout = 10,
    this.cdn = 'none',
    this.look = LookId.canoa,
    this.followProvider = false,
    this.brightness = 'dark',
    this.density = ChatDensity.normal,
    this.agents = const [],
    this.host,
    this.sessions = const LedgerSettings(),
    this.overlays = const {},
    this.profile = const Profile(),
    this.agentProfiles = const {},
    this.tables = const {},
    this.defaultMode,
    this.allowOnceOnly = false,
    this.protectedPaths,
    this.notifications = const NotificationSettings(),
    this.voice = const VoiceSettings(),
    this.hiddenSources = const {},
    this.pasteAsFile = defaultPasteAsFile,
    this.embedLimit = defaultEmbedLimit,
    this.warnPermille = defaultWarnPermille,
    this.limitPhrases,
    this.source = const ConfigSource(),
  });

  /// Where this configuration was read from, which a save writes back to.
  final ConfigSource source;

  /// `[usage] warn_permille`: from this many thousandths used, the usage control shows a status (800 is 80%).
  final int warnPermille;

  /// `[usage] limit_phrases`, which only `canoa` reads (the phrases it takes as a usage limit in a failed turn of an agent that
  /// reports none): kept as written so saving settings never loses it. `null` is unset.
  final List<String>? limitPhrases;

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

  /// Whether the shown conversation's agent picks the family (Claude and Codex have one); [look] is the one for any other.
  final bool followProvider;

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

  /// `[permissions] default_mode`: the mode new sessions start in, one of `manual`, `accept_edits`, `auto`, `plan` and `bypass`;
  /// null leaves each agent's own. An older `full_access` list is read and dropped.
  final String? defaultMode;

  /// `[permissions] allow_once_only`: Bypass answers a request only with an answer that allows once, so a request that offers
  /// only an "always" is left to the person. Off, it takes the agent's "always" when that is all there is.
  final bool allowOnceOnly;

  /// `[permissions] protected`: the paths a remembered rule never covers, in gitignore syntax, one line a line (see
  /// `protected_paths.dart`); null leaves the recommended list, and a list written here replaces it.
  final List<String>? protectedPaths;

  final NotificationSettings notifications;

  /// `[voice]`: the speech service and how dictation is held.
  final VoiceSettings voice;

  /// `[sources] hidden`: the presets, by label, that Settings keeps out of the new-session picker.
  final Set<String> hiddenSources;

  /// `light`, `dark`, or `system` to follow the machine.
  final String brightness;

  /// `[appearance] density`: how much of an agent's work a conversation shows, `normal`, `thinking` or `verbose`.
  final ChatDensity density;

  ThemeMode get themeMode => switch (brightness) {
    'light' => ThemeMode.light,
    'system' => ThemeMode.system,
    _ => ThemeMode.dark,
  };

  CanoaConfig copyWith({bool? startHidden, bool? closeToTray,
      bool? minimizeToTray, bool? preferWebTransport, int? connectTimeout,
      LookId? look, bool? followProvider, String? brightness, ChatDensity? density, LedgerSettings? sessions, String? defaultMode, bool clearDefaultMode = false, bool? allowOnceOnly,
      List<String>? protectedPaths, bool clearProtectedPaths = false,
      NotificationSettings? notifications, VoiceSettings? voice, List<AgentCommand>? agents, Set<String>? hiddenSources, int? warnPermille}) =>
      CanoaConfig(
        startHidden: startHidden ?? this.startHidden,
        closeToTray: closeToTray ?? this.closeToTray,
        minimizeToTray: minimizeToTray ?? this.minimizeToTray,
        localHost: localHost,
        preferWebTransport: preferWebTransport ?? this.preferWebTransport,
        connectTimeout: connectTimeout ?? this.connectTimeout,
        cdn: cdn,
        look: look ?? this.look,
        followProvider: followProvider ?? this.followProvider,
        brightness: brightness ?? this.brightness,
        density: density ?? this.density,
        agents: agents ?? this.agents,
        host: host,
        sessions: sessions ?? this.sessions,
        overlays: overlays,
        profile: profile,
        agentProfiles: agentProfiles,
        tables: tables,
        defaultMode: clearDefaultMode ? null : defaultMode ?? this.defaultMode,
        allowOnceOnly: allowOnceOnly ?? this.allowOnceOnly,
        protectedPaths: clearProtectedPaths ? null : protectedPaths ?? this.protectedPaths,
        notifications: notifications ?? this.notifications,
        voice: voice ?? this.voice,
        hiddenSources: hiddenSources ?? this.hiddenSources,
        pasteAsFile: pasteAsFile,
        embedLimit: embedLimit,
        warnPermille: warnPermille ?? this.warnPermille,
        limitPhrases: limitPhrases,
        source: source,
      );

  /// This configuration with [host] as its `[host]` table.
  CanoaConfig withHost(Map<String, dynamic>? host, {ConfigSource? source}) => CanoaConfig(
    startHidden: startHidden,
    closeToTray: closeToTray,
    minimizeToTray: minimizeToTray,
    localHost: localHost,
    preferWebTransport: preferWebTransport,
    connectTimeout: connectTimeout,
    cdn: cdn,
    look: look,
    followProvider: followProvider,
    brightness: brightness,
    density: density,
    agents: agents,
    host: host,
    sessions: sessions,
    overlays: overlays,
    profile: profile,
    agentProfiles: agentProfiles,
    tables: tables,
    defaultMode: defaultMode,
    allowOnceOnly: allowOnceOnly,
    protectedPaths: protectedPaths,
    notifications: notifications,
    voice: voice,
    hiddenSources: hiddenSources,
    pasteAsFile: pasteAsFile,
    embedLimit: embedLimit,
    warnPermille: warnPermille,
    limitPhrases: limitPhrases,
    source: source ?? this.source,
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
    String? usage,
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
      'usage': ?usage,
    };
    copy['agents'] = table;
    return withHost(copy);
  }

  Map<dynamic, dynamic> get _hostAgents => host?['agents'] is Map ? host!['agents'] as Map : const {};

  /// The provider the agent entry [name] runs, or null for an entry that runs a program.
  String? hostHarnessOf(String name) => _hostAgents[name] is Map ? (_hostAgents[name] as Map)['harness'] as String? : null;

  /// The agent entries that run the provider [harness], in canoa.cfg's order.
  List<String> hostEntriesRunning(String harness) => [
    for (final MapEntry(:key, :value) in _hostAgents.entries)
      if (value is Map && value['harness'] == harness) '$key',
  ];

  /// The `[host.accounts]` the agent entry [name] offers a client; empty when it offers none, which leaves its harness to run on
  /// the login it has where it runs.
  List<String> hostAccountsOf(String name) => [
    if (_hostAgents[name] is Map && (_hostAgents[name] as Map)['accounts'] is List)
      for (final each in (_hostAgents[name] as Map)['accounts'] as List) '$each',
  ];

  /// Whether the agent entry [name] is open to every paired key: it names no `peers`.
  bool hostEntryIsOpen(String name) => _hostAgents[name] is Map && (_hostAgents[name] as Map)['peers'] == null;

  /// The entry that is the agent itself for the provider [harness]: Claude's or Codex's own source when canoa.cfg or the host names
  /// it, else the first entry that runs the provider, else the name the host makes one with (a source's own, or the provider's with
  /// a capital).
  String ownEntryOf(String harness) {
    const sources = {'claude': 'Claude', 'codex': 'Codex'};
    final running = hostEntriesRunning(harness);
    final source = sources[harness];
    if (source != null && running.contains(source)) return source;
    if (running.isNotEmpty) return running.first;
    return source ?? '${harness[0].toUpperCase()}${harness.substring(1)}';
  }

  /// This configuration as the host sees its agent entries: [entries], the host's own list, name the entries that run a provider with
  /// accounts, and what each offers is the host's to say. An entry canoa.cfg has keeps its other keys; one it has not (a built-in
  /// source) is read as an open entry that runs the provider. Never written back whole: it holds what the host serves, not the file.
  CanoaConfig withHostEntries(List<AccountEntry> entries) {
    if (entries.isEmpty) return this;
    final copy = _deepCopy(host ?? const {});
    final table = (copy['agents'] as Map<String, dynamic>?) ?? <String, dynamic>{};
    for (final entry in entries) {
      final one = table[entry.name] is Map ? Map<String, dynamic>.of((table[entry.name] as Map).cast<String, dynamic>()) : <String, dynamic>{};
      one['harness'] = entry.harness;
      entry.accounts.isEmpty ? one.remove('accounts') : one['accounts'] = List<String>.of(entry.accounts);
      if (!entry.open) one['peers'] ??= <String>[];
      table[entry.name] = one;
    }
    copy['agents'] = table;
    return withHost(copy);
  }

  /// This configuration with the account [account] offered by the agent entry [entry], or, with none named, by the agent's own entry
  /// for [harness] (see [ownEntryOf]); the app never makes an entry of its own for accounts. An entry the person wrote for the
  /// provider, other than the one named, is never changed. The account is appended, so it never becomes the default of an entry that
  /// already offers others.
  CanoaConfig withAccountOffered({required String account, required String harness, String? entry}) {
    final copy = _deepCopy(host ?? const {});
    final table = (copy['agents'] as Map<String, dynamic>?) ?? <String, dynamic>{};
    final target = entry ?? ownEntryOf(harness);
    if (table[target] == null) {
      table[target] = <String, dynamic>{'harness': harness, 'title': target, 'peers': <String>[]};
    }
    final entryTable = Map<String, dynamic>.of((table[target] as Map).cast<String, dynamic>());
    final names = [...?(entryTable['accounts'] as List?)?.map((each) => '$each')];
    if (!names.contains(account)) names.add(account);
    entryTable['accounts'] = names;
    table[target] = entryTable;
    copy['agents'] = table;
    return withHost(copy);
  }

  /// This configuration with the entry [entry] offering exactly [accounts], in that order (none takes the key out).
  CanoaConfig withAccountsOf(String entry, List<String> accounts) {
    final copy = _deepCopy(host ?? const {});
    final table = copy['agents'];
    if (table is Map<String, dynamic> && table[entry] is Map) {
      final one = Map<String, dynamic>.of((table[entry] as Map).cast<String, dynamic>());
      accounts.isEmpty ? one.remove('accounts') : one['accounts'] = List<String>.of(accounts);
      table[entry] = one;
    }
    return withHost(copy);
  }

  /// This configuration with the account [account] offered by no entry, or no longer by the entry [only]; an entry left with none goes
  /// back to the harness's own login, which is what an entry with no `accounts` runs.
  CanoaConfig withoutAccountOffered(String account, {String? only}) {
    final copy = _deepCopy(host ?? const {});
    final table = copy['agents'];
    if (table is Map<String, dynamic>) {
      for (final MapEntry(key: name, value: entry) in table.entries) {
        if (entry is! Map<String, dynamic> || entry['accounts'] is! List || (only != null && name != only)) continue;
        final left = [for (final each in entry['accounts'] as List) if ('$each' != account) '$each'];
        left.isEmpty ? entry.remove('accounts') : entry['accounts'] = left;
      }
    }
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
    // The entry's other keys (its options, its `kb` choice, any the host reads that the app does not) stay as they are.
    final entry = raw[name];
    final kept = entry is Map ? <String, dynamic>{...entry} : <String, dynamic>{};
    raw[name] = switch (source) {
      GitOverlay() => <String, dynamic>{...kept, 'source': source.spec, 'pin': digest},
      DirOverlay(:final dir) => <String, dynamic>{...kept, 'dir': dir, 'pin': digest},
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

  /// The values the person gave the options the overlay [name] declares: `[overlays.NAME] options`. The host reads them
  /// when it assembles the overlay's context; the overlay's manifest says which options exist and of what type.
  Map<String, Object?> overlayOptions(String name) {
    final entry = (tables['overlays'] as Map?)?[name];
    final options = entry is Map ? entry['options'] : null;
    return options is Map ? {for (final MapEntry(:key, :value) in options.entries) '$key': value} : const {};
  }

  /// How the overlay [name]'s knowledge reaches the agent: `[overlays.NAME] kb`, `index` (searched through the knowledge tools)
  /// or `files` (put in the context whole), or null when the person chose neither and the overlay's own hint decides.
  String? overlayKb(String name) {
    final entry = (tables['overlays'] as Map?)?[name];
    final choice = entry is Map ? entry['kb'] : null;
    return choice == 'index' || choice == 'files' ? choice as String : null;
  }

  /// This configuration with the knowledge of the overlay [name] reaching the agent as [choice] (`index`, `files`), or by the
  /// overlay's own hint when [choice] is null. The entry's other keys stay.
  CanoaConfig withOverlayKb(String name, String? choice) {
    final source = overlays[name];
    if (source == null) throw ArgumentError.value(name, 'name', 'No overlay of that name');
    if (choice != null && choice != 'index' && choice != 'files') throw ArgumentError.value(choice, 'choice', 'index or files');
    final raw = _deepCopy((tables['overlays'] as Map?) ?? const {});
    final entry = raw[name];
    final fields = entry is Map
        ? <String, dynamic>{...entry}
        : switch (source) {
            GitOverlay() => <String, dynamic>{'source': source.spec, if (source.pin != null) 'pin': source.pin},
            DirOverlay(:final dir, :final pin) => <String, dynamic>{'dir': dir, 'pin': ?pin},
          };
    choice == null ? fields.remove('kb') : fields['kb'] = choice;
    raw[name] = fields;
    return _withTable('overlays', raw, overlays: overlays);
  }

  /// `[host.kb]`: where the knowledge tools get meaning and ordering, as the file holds it. Only the four the app edits are
  /// named here; the others the host reads stay in the file.
  static const hostKbKeys = ['embeddings', 'model', 'llm_endpoint', 'llm_model'];

  /// The values of [hostKbKeys] that `[host.kb]` holds.
  Map<String, String> get hostKb {
    final kb = host?['kb'];
    if (kb is! Map) return const {};
    return {
      for (final key in hostKbKeys)
        if (kb[key] case final String value when value.isNotEmpty) key: value,
    };
  }

  /// This configuration with the [hostKbKeys] of `[host.kb]` set to [values] (a key not in it, or empty, is taken out), and every
  /// other key of the table, and of `[host]`, as it was. The table goes when nothing is left in it.
  CanoaConfig withHostKb(Map<String, String> values) {
    final copy = _deepCopy(host ?? const {});
    final kb = copy['kb'] is Map ? Map<String, dynamic>.of((copy['kb'] as Map).cast<String, dynamic>()) : <String, dynamic>{};
    for (final key in hostKbKeys) {
      final value = values[key]?.trim() ?? '';
      value.isEmpty ? kb.remove(key) : kb[key] = value;
    }
    kb.isEmpty ? copy.remove('kb') : copy['kb'] = kb;
    return withHost(copy.isEmpty ? null : copy);
  }

  /// This configuration with the option values [options] for the overlay [name], or none when [options] is empty. A string
  /// entry `NAME = "source"` becomes `{ source }` to carry them.
  CanoaConfig withOverlayOptions(String name, Map<String, Object?> options) {
    final source = overlays[name];
    if (source == null) throw ArgumentError.value(name, 'name', 'No overlay of that name');
    final raw = _deepCopy((tables['overlays'] as Map?) ?? const {});
    final entry = raw[name];
    final fields = entry is Map
        ? <String, dynamic>{...entry}
        : switch (source) {
            GitOverlay() => <String, dynamic>{'source': source.spec, if (source.pin != null) 'pin': source.pin},
            DirOverlay(:final dir, :final pin) => <String, dynamic>{'dir': dir, 'pin': ?pin},
          };
    if (options.isEmpty) {
      fields.remove('options');
    } else {
      fields['options'] = _deepCopy(options);
    }
    raw[name] = fields;
    return _withTable('overlays', raw, overlays: overlays);
  }

  /// `[catalogs]`: each catalog the store lists, by the label the person gives it, a git source whose `#path` names the
  /// catalog file.
  Map<String, GitOverlay> get catalogs => _catalogs(tables['catalogs']);

  /// `[skill_pins]`: the tree digest the person requires of a skill from elsewhere, by its name in lower case. A skill is
  /// fetched only when its digest equals this and the pin its entry or catalog gives.
  Map<String, String> get skillPins => _skillPins(tables['skill_pins']);

  static Map<String, String> _skillPins(Object? raw) {
    if (raw == null) return const {};
    if (raw is! Map) throw const FormatException('canoa.cfg: skill_pins must be a table');
    final out = <String, String>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      final name = '$key'.toLowerCase();
      if (!validSkillName('$key')) {
        throw FormatException('canoa.cfg: skill_pins.$key is not a skill name');
      }
      if (value is! String || !RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(value)) {
        throw FormatException('canoa.cfg: skill_pins.$key is a sha256: tree digest');
      }
      if (out.containsKey(name)) throw FormatException('canoa.cfg: skill_pins.$key repeats a name');
      out[name] = value.toLowerCase();
    }
    return out;
  }

  /// `[loadouts.NAME]`: the named collections of whole overlays, chosen skills and MCP servers a session starts with, in the order
  /// written. A loadout cannot reorder its overlays: they apply in the order `[overlays]` composes them.
  Map<String, Loadout> get loadouts => _loadouts(tables['loadouts']);

  /// `[defaults] loadout`: the name of the loadout a session that names none starts with; null when none is set, which starts a
  /// session with every overlay `[overlays]` names and every MCP server, as before there were loadouts.
  String? get defaultLoadout => _defaultLoadout(tables['defaults'], loadouts);

  /// What a session that names no loadout starts with: the default loadout, or when there is none, everything the profile
  /// holds. This is what the editors change, and a `Default` loadout is written the first time one of them does.
  Loadout get startingLoadout {
    if (defaultLoadout case final name?) return loadouts[name]!;
    return Loadout(overlays: overlays.keys.toList(), mcp: [
      for (final MapEntry(:key, :value) in profile.mcp.entries)
        if (value != null) key,
    ]);
  }

  /// The loadout [name] (the default when null) as this file says it now, the way the host resolves it when a session starts:
  /// its overlays in the order `[overlays]` composes them, those `[overlays]` lacks apart. Null when none is named and there is
  /// no default. Throws [UnknownLoadout] for a name `[loadouts]` does not hold.
  ResolvedLoadout? resolveLoadout([String? name]) {
    final chosen = name ?? defaultLoadout;
    if (chosen == null) return null;
    final loadout = loadouts[chosen];
    if (loadout == null) throw UnknownLoadout(chosen);
    return ResolvedLoadout(
      name: chosen,
      overlays: [for (final installed in overlays.keys) if (loadout.overlays.contains(installed)) installed],
      missingOverlays: [for (final named in loadout.overlays) if (!overlays.containsKey(named)) named],
      skills: loadout.skills,
      mcp: loadout.mcp,
    );
  }

  static Map<String, Loadout> _loadouts(Object? raw) {
    if (raw == null) return const {};
    if (raw is! Map) throw const FormatException('canoa.cfg: loadouts must be a table');
    final out = <String, Loadout>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      if (!isEntryName('$key')) {
        throw FormatException('canoa.cfg: loadouts.$key: a loadout is named with 1 to 64 letters, digits, - or _');
      }
      if (value is! Map) throw FormatException('canoa.cfg: loadouts.$key must be a table');
      out['$key'] = Loadout.read('$key', value);
    }
    return out;
  }

  static String? _defaultLoadout(Object? raw, Map<String, Loadout> loadouts) {
    if (raw == null) return null;
    if (raw is! Map || raw.keys.any((key) => key != 'loadout')) {
      throw const FormatException('canoa.cfg: defaults is a table with the key loadout');
    }
    final name = raw['loadout'];
    if (name == null) return null;
    if (name is! String || !loadouts.containsKey(name)) {
      throw const FormatException('canoa.cfg: defaults.loadout is the name of a loadout in loadouts');
    }
    return name;
  }

  /// This configuration with [loadouts] as `[loadouts]`, and [defaultName] as `[defaults] loadout` (kept as it is when omitted;
  /// dropped when it names a loadout that is gone).
  CanoaConfig withLoadouts(Map<String, Loadout> loadouts, {String? defaultName}) {
    final keep = defaultName ?? defaultLoadout;
    final named = keep != null && loadouts.containsKey(keep) ? keep : null;
    final defaults = {...((tables['defaults'] as Map?) ?? const {}).cast<String, dynamic>()}..remove('loadout');
    return _withTable('loadouts', {for (final MapEntry(:key, :value) in loadouts.entries) key: value.toTable()})
        ._withTable('defaults', {...defaults, 'loadout': ?named});
  }

  /// This configuration with [change] applied to the loadout a session that names none starts with; the first change writes it
  /// as `Default`, holding what the session started with until now.
  CanoaConfig _withStarting(Loadout Function(Loadout loadout) change) {
    final name = defaultLoadout ?? 'Default';
    return withLoadouts({...loadouts, name: change(startingLoadout)}, defaultName: name);
  }

  /// The skills the person chose that a new session starts with, by name.
  Set<String> get loadoutSkills => startingLoadout.skills.toSet();

  /// The person's own layer with the chosen skills that are on, each taken from where the app keeps them. A profile entry the
  /// person wrote for the same name stays.
  Profile get profileWithChosenSkills {
    final chosen = loadoutSkills;
    if (chosen.isEmpty) return profile;
    return profile.withSkills({
      for (final name in chosen) name: Skill('overlays/$skillsDirectory/${skillKey(name)}'),
      ...profile.skills,
    });
  }

  /// `[store.mcp.NAME]`: where each MCP server the store installed came from: the registry's address, the server's name
  /// and version there, and the digest of the `server.json` that was shown and accepted.
  Map<String, McpOrigin> get storeMcp => _storeExtras(tables['store']).mcp;

  /// `[store] registries`: the MCP registries the person added beside the official one, in the order added. Each is the address of
  /// a registry that serves the MCP Registry's API, or a git source (`<repository>@<ref>#<file>`) of a file in that API's list
  /// shape. Whether an address can be read is for the page to say when it is used.
  List<String> get storeRegistries => _storeExtras(tables['store']).registries;

  static ({Map<String, McpOrigin> mcp, List<String> registries}) _storeExtras(Object? raw) {
    if (raw == null) return (mcp: const {}, registries: const []);
    if (raw is! Map) throw const FormatException('canoa.cfg: store must be a table');
    final registries = raw['registries'];
    if (registries != null &&
        (registries is! List ||
            registries.length > 16 ||
            registries.toSet().length != registries.length ||
            registries.any((address) => address is! String || address.isEmpty || address.runes.any((rune) => rune <= 0x20 || rune == 0x7f)))) {
      throw const FormatException('canoa.cfg: store.registries is a list of at most 16 different registry addresses');
    }
    final mcp = <String, McpOrigin>{};
    final rawMcp = raw['mcp'];
    if (rawMcp != null && rawMcp is! Map) throw const FormatException('canoa.cfg: store.mcp must be a table');
    for (final MapEntry(:key, :value) in ((rawMcp as Map?) ?? const {}).entries) {
      final origin = value is Map ? McpOrigin.fromMap(value) : null;
      if (!isEntryName('$key') || origin == null) {
        throw FormatException('canoa.cfg: store.mcp.$key is a table of registry, server, version and digest');
      }
      mcp['$key'] = origin;
    }
    return (mcp: mcp, registries: [for (final address in (registries as List?) ?? const []) address as String]);
  }

  /// This configuration with [mcp] and [registries] (each left as it is when omitted) in `[store]`, beside its limits.
  CanoaConfig _withStoreExtras({Map<String, McpOrigin>? mcp, List<String>? registries}) {
    final now = _storeExtras(tables['store']);
    final mcpNow = mcp ?? now.mcp, registriesNow = registries ?? now.registries;
    final table = {...((tables['store'] as Map?) ?? const {}).cast<String, dynamic>()}
      ..remove('mcp')
      ..remove('registries');
    return _withTable('store', {
      ...table,
      if (registriesNow.isNotEmpty) 'registries': registriesNow,
      if (mcpNow.isNotEmpty) 'mcp': {for (final MapEntry(:key, :value) in mcpNow.entries) key: value.toMap()},
    });
  }

  /// This configuration with the MCP server [name] defined as [definition] in `[profile.mcp]` and [origin] noted in
  /// `[store.mcp]`; with it off for each of [agents], so that no agent starts it until the person allows it.
  CanoaConfig withInstalledMcp(String name, Map<String, dynamic> definition, McpOrigin origin, {required List<String> agents}) {
    if (!isEntryName(name)) throw ArgumentError.value(name, 'name', 'An MCP server name has 1 to 64 letters, digits, - or _');
    var out = _withProfileEntry('mcp', name, definition)._withStoreExtras(mcp: {...storeMcp, name: origin});
    for (final agent in agents) {
      out = out.withAgentMcp(agent, {...?out.agentMcp[agent], name: false});
    }
    return out.withStartingMcp(name, true);
  }

  /// This configuration without the MCP server [name]: its definition, its note, every agent's choice for it and its place in
  /// every loadout.
  CanoaConfig withoutMcp(String name) {
    var out = _withProfileEntry('mcp', name, null)._withStoreExtras(mcp: {...storeMcp}..remove(name));
    for (final MapEntry(key: agent, value: choices) in agentMcp.entries) {
      if (choices.containsKey(name)) out = out.withAgentMcp(agent, {...choices}..remove(name));
    }
    return out._withEachLoadout((loadout) => loadout.withMcp(name, false));
  }

  /// This configuration with [registries] as `[store] registries`, in the order given.
  CanoaConfig withStoreRegistries(List<String> registries) => _withStoreExtras(registries: registries);

  /// This configuration with [name] in, or out of, the servers a new session starts with. Nothing changes while no loadout is the
  /// default, since a session then starts with every server the agent allows.
  CanoaConfig withStartingMcp(String name, bool on) =>
      defaultLoadout == null ? this : _withStarting((loadout) => loadout.withMcp(name, on));

  /// This configuration with [skills] as the chosen skills a new session starts with, sorted so the file does not churn.
  CanoaConfig withLoadoutSkills(Set<String> skills) =>
      _withStarting((loadout) => loadout.copyWith(skills: skills.toList()..sort()));

  /// This configuration with the overlay [name] in, or out of, the overlays a new session starts with.
  CanoaConfig withStartingOverlay(String name, bool on) =>
      _withStarting((loadout) => loadout.copyWith(overlays: [
            for (final each in loadout.overlays) if (each != name) each,
            if (on) name,
          ]));

  CanoaConfig _withEachLoadout(Loadout Function(Loadout loadout) change) {
    if (loadouts.isEmpty) return this;
    return withLoadouts({for (final MapEntry(:key, :value) in loadouts.entries) key: change(value)});
  }

  CanoaConfig _withProfileEntry(String section, String name, Map<String, dynamic>? entry) {
    final raw = _deepCopy((tables['profile'] as Map?) ?? const {});
    final table = (raw[section] as Map<String, dynamic>?) ?? <String, dynamic>{};
    if (entry == null) {
      table.remove(name);
    } else {
      table[name] = entry;
    }
    if (table.isEmpty) {
      raw.remove(section);
    } else {
      raw[section] = table;
    }
    return withProfileTables(overlays: overlays, tables: {...tables, 'profile': raw},
        profile: _profile('profile', raw, LayerKind.user), agentProfiles: agentProfiles);
  }

  /// The overlays a new session that names no loadout starts with, in the order `[overlays]` composes them.
  List<String> get defaultOverlays => resolveLoadout()?.overlays ?? overlays.keys.toList();

  /// `[store]`: the store's limits.
  StoreSettings get store => StoreSettings.parse((tables['store'] as Map?) ?? const {});

  /// This configuration with [limits] as `[store]`: only a value that differs from its default is written.
  CanoaConfig withStore(StoreSettings limits) {
    const d = StoreSettings();
    final table = <String, dynamic>{
      if (limits.catalogMaxBytes != d.catalogMaxBytes) 'catalog_max_bytes': limits.catalogMaxBytes,
      if (limits.catalogMaxDepth != d.catalogMaxDepth) 'catalog_max_depth': limits.catalogMaxDepth,
      if (limits.catalogMaxEntries != d.catalogMaxEntries) 'catalog_max_entries': limits.catalogMaxEntries,
      if (limits.catalogMaxIcons != d.catalogMaxIcons) 'catalog_max_icons': limits.catalogMaxIcons,
      if (limits.iconMaxBytes != d.iconMaxBytes) 'icon_max_bytes': limits.iconMaxBytes,
      if (limits.iconMaxWidth != d.iconMaxWidth) 'icon_max_width': limits.iconMaxWidth,
      if (limits.iconMaxHeight != d.iconMaxHeight) 'icon_max_height': limits.iconMaxHeight,
      if (limits.contentsTtlSeconds != d.contentsTtlSeconds) 'contents_ttl_seconds': limits.contentsTtlSeconds,
    };
    return _withTable('store', {...table, ...?_storeExtrasTable()});
  }

  Map<String, dynamic>? _storeExtrasTable() {
    final raw = tables['store'];
    if (raw is! Map) return null;
    return {for (final key in const ['registries', 'mcp']) if (raw[key] != null) key: raw[key]};
  }

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

  /// `[agents.NAME.hooks]`: for each agent, by label, the overlays whose hooks the person allowed, each at the digest of the
  /// overlay's files (`sha256:...`) it was shown.
  Map<String, Map<String, String>> get agentHooks => _agentHooks(tables['agents']);

  static Map<String, Map<String, String>> _agentHooks(Object? agents) {
    final out = <String, Map<String, String>>{};
    if (agents is! Map) return out;
    for (final MapEntry(key: agent, :value) in agents.entries) {
      final table = value is Map ? value['hooks'] : null;
      if (table == null) continue;
      if (table is! Map) throw FormatException('canoa.cfg: agents.$agent.hooks must be a table');
      final allowed = <String, String>{};
      for (final MapEntry(:key, value: digest) in table.entries) {
        if (!isEntryName('$key') || digest is! String || !RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(digest)) {
          throw FormatException('canoa.cfg: agents.$agent.hooks.$key is the sha256: digest of the overlay\'s files that were '
              'allowed');
        }
        allowed['$key'] = digest;
      }
      out['$agent'] = allowed;
    }
    return out;
  }

  /// This configuration with [allowed] as `[agents.NAME.hooks]` of [agent], as [agentHooks] reads them.
  CanoaConfig withAgentHooks(String agent, Map<String, String> allowed) {
    final raw = _deepCopy((tables['agents'] as Map?) ?? const {});
    final entry = (raw[agent] as Map<String, dynamic>?) ?? <String, dynamic>{};
    if (allowed.isEmpty) {
      entry.remove('hooks');
    } else {
      entry['hooks'] = <String, dynamic>{...allowed};
    }
    if (entry.isEmpty) {
      raw.remove(agent);
    } else {
      raw[agent] = entry;
    }
    _agentHooks(raw);
    return _withTable('agents', raw);
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

  /// `[connections.NAME]`: the hosts and agents elsewhere the person saved, in name order.
  List<Connection> get connections => _connections(tables['connections']);

  static List<Connection> _connections(Object? raw) {
    if (raw == null) return const [];
    if (raw is! Map) throw const FormatException('canoa.cfg: connections must be a table');
    return [for (final MapEntry(:key, :value) in raw.entries) Connection.read('$key', value)]..sort((a, b) => a.name.compareTo(b.name));
  }

  /// This configuration with [connection] saved, taking the place of the one named [replacing] when that is given.
  CanoaConfig withConnection(Connection connection, {String? replacing}) {
    final raw = _deepCopy((tables['connections'] as Map?) ?? const {});
    if (replacing != null) raw.remove(replacing);
    if (raw.containsKey(connection.name)) throw FormatException('A connection named ${connection.name} is already saved.');
    raw[connection.name] = connection.toTable();
    return _withTable('connections', raw);
  }

  /// This configuration without the connection [name].
  CanoaConfig withoutConnection(String name) =>
      _withTable('connections', _deepCopy((tables['connections'] as Map?) ?? const {})..remove(name));

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
    final added = _withTable('overlays', raw, overlays: {...overlays, name: source.pinnedTo(pin)});
    // A new overlay starts on, in the loadout new sessions start with, when there is one written.
    return overlays.containsKey(name) || defaultLoadout == null ? added : added.withStartingOverlay(name, true);
  }

  /// This configuration without the overlay [name] in `[overlays]`.
  CanoaConfig withoutOverlay(String name) {
    final removed = _withTable('overlays', _deepCopy((tables['overlays'] as Map?) ?? const {})..remove(name),
        overlays: {for (final MapEntry(:key, :value) in overlays.entries) if (key != name) key: value});
    // An overlay put back later starts as a new one does, not in a loadout because an earlier one of its name was.
    return removed._withEachLoadout((loadout) => loadout.copyWith(overlays: [for (final each in loadout.overlays) if (each != name) each]));
  }

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
    followProvider: followProvider,
    brightness: brightness,
    density: density,
    agents: agents,
    host: host,
    sessions: sessions,
    overlays: overlays,
    profile: profile ?? this.profile,
    agentProfiles: agentProfiles ?? this.agentProfiles,
    tables: tables,
    defaultMode: defaultMode,
    allowOnceOnly: allowOnceOnly,
    protectedPaths: protectedPaths,
    notifications: notifications,
    voice: voice,
    hiddenSources: hiddenSources,
    pasteAsFile: pasteAsFile,
    embedLimit: embedLimit,
    warnPermille: warnPermille,
    limitPhrases: limitPhrases,
    source: source,
  );

  static Map<String, dynamic> _deepCopy(Map<dynamic, dynamic> map) => {
    for (final MapEntry(:key, :value) in map.entries)
      '$key': switch (value) {
        final Map<dynamic, dynamic> inner => _deepCopy(inner),
        final List<dynamic> list => List<dynamic>.of(list),
        _ => value,
      },
  };

  /// Reads [text], the file called [name] if it has one, as the person's own canoa.cfg over [below], the layers beneath it
  /// (lowest first). Throws [FormatException] naming no value.
  factory CanoaConfig.parse(String text, {String? name, List<ConfigLayer> below = const []}) =>
      CanoaConfig.fromLayers(decodeConfig(text, name: name), below);

  /// The person's own file [own] merged over [below]: what the app shows and edits.
  factory CanoaConfig.fromLayers(ConfigText own, List<ConfigLayer> below) {
    var lower = <String, dynamic>{};
    for (final layer in below) {
      lower = mergeConfig(lower, layer.data);
    }
    final data = mergeConfig(lower, own.data);
    final readFrom = ConfigSource(format: own.format, marked: own.marked, layers: below, own: own.data);
    for (final name in data.keys) {
      if (!{'desktop', 'network', 'assets', 'appearance', 'agents', 'host', 'sessions', 'harnesses', 'overlays', 'profile',
            'permissions', 'notifications', 'catalogs', 'store', 'skill_pins', 'loadouts', 'defaults', 'sources', 'attachments', 'usage', 'voice', 'speech', 'connections'}
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
    final appearance = table('appearance', {'look', 'follow_provider', 'brightness', 'density'});
    final sessions = table('sessions', {'keep', 'keep_text', 'text_messages', 'workspace', 'while_working', 'queue_delivery'});
    final permissions = table('permissions', {'default_mode', 'full_access', 'allow_once_only', 'protected'});
    final notifications = table('notifications', {'permission', 'question', 'finished'});
    final sources = table('sources', {'hidden'});
    final attachments = table('attachments', {'paste_as_file', 'embed_limit'});
    final usage = table('usage', {'warn_permille', 'limit_phrases'});
    final voiceTable = table('voice', {'backend', 'model', 'language', 'device', 'mode'});
    String? voiceText(String key) {
      final raw = voiceTable[key];
      if (raw == null) return null;
      if (raw is! String || raw.trim().isEmpty) throw FormatException('canoa.cfg: voice.$key must be text');
      return raw;
    }
    final voiceMode = voiceTable['mode'] ?? 'toggle';
    if (voiceMode is! String || !{'toggle', 'hold'}.contains(voiceMode)) {
      throw const FormatException('canoa.cfg: voice.mode must be toggle or hold');
    }
    int limit(String key, int fallback) {
      final raw = attachments[key] ?? fallback;
      if (raw is! int || raw < 0) throw FormatException('canoa.cfg: attachments.$key must be a whole number, 0 or more');
      return raw;
    }
    final warn = usage['warn_permille'] ?? defaultWarnPermille;
    if (warn is! int || warn < 0 || warn > 1000) {
      throw const FormatException('canoa.cfg: usage.warn_permille must be a whole number from 0 to 1000');
    }
    final phrases = usage['limit_phrases'];
    if (phrases != null && (phrases is! List || phrases.any((phrase) => phrase is! String))) {
      throw const FormatException('canoa.cfg: usage.limit_phrases must be a list of text');
    }
    final hidden = sources['hidden'] ?? const <Object>[];
    if (hidden is! List || hidden.any((name) => name is! String || name.trim().isEmpty)) {
      throw const FormatException('canoa.cfg: sources.hidden must be a list of preset names');
    }
    final defaultMode = permissions['default_mode'];
    if (defaultMode != null && (defaultMode is! String || !modeOrder.contains(defaultMode))) {
      throw FormatException('canoa.cfg: permissions.default_mode must be one of ${modeOrder.join(', ')}');
    }
    final dropped = permissions['full_access'] ?? const <Object>[];
    if (dropped is! List || dropped.any((name) => name is! String || name.trim().isEmpty)) {
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
    final lookName = appearance['look'] ?? LookId.canoa.name;
    final look = lookName is String ? LookId.named(lookName) : null;
    if (look == null) {
      throw FormatException('canoa.cfg: appearance.look must be one of ${LookId.values.map((id) => id.name).join(', ')}');
    }
    final brightness = appearance['brightness'] ?? 'dark';
    if (brightness is! String || !{'light', 'dark', 'system'}.contains(brightness)) {
      throw const FormatException('canoa.cfg: appearance.brightness must be light, dark or system');
    }
    final densityName = appearance['density'] ?? ChatDensity.normal.name;
    final density = densityName is String ? ChatDensity.named(densityName) : null;
    if (density == null) {
      throw FormatException('canoa.cfg: appearance.density must be one of ${ChatDensity.values.map((each) => each.name).join(', ')}');
    }
    final agents = <AgentCommand>[];
    final agentProfiles = <String, Profile>{};
    final agentTables = <String, Object?>{};
    final rawAgents = data['agents'];
    if (rawAgents != null) {
      if (rawAgents is! Map) throw const FormatException('canoa.cfg: agents must be a table of agents');
      for (final entry in rawAgents.entries) {
        final agent = entry.value;
        if (agent is Map && (agent['profile'] != null || agent['mcp'] != null || agent['hooks'] != null)) {
          final table = agent['profile'];
          if (table != null && table is! Map) throw FormatException('canoa.cfg: agents.${entry.key}.profile must be a table');
          if (table is Map) agentProfiles['${entry.key}'] = _profile('agents.${entry.key}.profile', table, LayerKind.agent);
          agentTables['${entry.key}'] = {'profile': ?table, 'mcp': ?agent['mcp'], 'hooks': ?agent['hooks']};
          if (!agent.containsKey('program') && agent.keys.every(const {'profile', 'mcp', 'hooks'}.contains)) continue;
        }
        if (agent is! Map || agent['program'] is! String || (agent['program'] as String).isEmpty) {
          throw FormatException('canoa.cfg: agents.${entry.key} needs a program');
        }
        for (final key in agent.keys) {
          if (key != 'program' && key != 'args' && key != 'profile' && key != 'mcp' && key != 'hooks') {
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
    final workspace = sessions['workspace'] ?? defaultWorkspace;
    if (workspace is! String) throw const FormatException('canoa.cfg: sessions.workspace must be a string');
    if (workspaceProblem(workspace) case final problem?) throw FormatException('canoa.cfg: sessions.workspace: $problem');
    final whileWorking = switch (sessions['while_working']) {
      null || 'queue' => WhileWorking.queue,
      'steer' => WhileWorking.steer,
      _ => throw const FormatException('canoa.cfg: sessions.while_working must be queue or steer'),
    };
    final queueDelivery = switch (sessions['queue_delivery']) {
      null || 'merge' => QueueDelivery.merge,
      'each' => QueueDelivery.each,
      _ => throw const FormatException('canoa.cfg: sessions.queue_delivery must be merge or each'),
    };
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
      // The values of the overlay's declared options, which the host reads, and how its knowledge reaches the agent (`kb`); only
      // their shape is checked here.
      Map<dynamic, dynamic> options(Map<dynamic, dynamic> entry) {
        if (entry['options'] != null && entry['options'] is! Map) {
          throw FormatException('canoa.cfg: overlays.$key.options is a table of values');
        }
        if (entry['kb'] != null && entry['kb'] != 'index' && entry['kb'] != 'files') {
          throw FormatException('canoa.cfg: overlays.$key.kb is "index" or "files"');
        }
        return entry;
      }
      overlays['$key'] = switch (value) {
        final String spec => git(spec),
        {'source': final String spec} when value.keys.every(const {'source', 'pin', 'options', 'kb'}.contains) =>
          git(spec).pinnedTo(pin(options(value))),
        {'dir': final String dir} when dir.isNotEmpty && value.keys.every(const {'dir', 'pin', 'options', 'kb'}.contains) =>
          DirOverlay(dir, pin: pin(options(value))),
        _ => throw FormatException('canoa.cfg: overlays.$key is a git source, { source, pin } or { dir, pin }'),
      };
    }
    final rawProfile = data['profile'];
    if (rawProfile != null && rawProfile is! Map) throw const FormatException('canoa.cfg: profile must be a table');
    _catalogs(data['catalogs']);
    _connections(data['connections']);
    _skillPins(data['skill_pins']);
    _defaultLoadout(data['defaults'], _loadouts(data['loadouts']));
    _storeExtras(data['store']);
    _agentMcp(agentTables);
    _agentHooks(agentTables);
    final protectedPaths = permissions['protected'] == null ? null : protectedLinesOf(permissions['protected']);
    if (permissions['protected'] != null && protectedPaths == null) {
      throw const FormatException('canoa.cfg: permissions.protected must be text, or a list of text, with a path pattern a line');
    }
    final rawStore = data['store'];
    if (rawStore != null && rawStore is! Map) throw const FormatException('canoa.cfg: store must be a table');
    if (rawStore is Map) StoreSettings.parse(rawStore);
    return CanoaConfig(
      source: readFrom,
      overlays: overlays,
      profile: rawProfile == null ? const Profile() : _profile('profile', rawProfile as Map, LayerKind.user),
      agentProfiles: agentProfiles,
      tables: {
        if (data['overlays'] != null) 'overlays': rawOverlays,
        'profile': ?rawProfile,
        if (agentTables.isNotEmpty) 'agents': agentTables,
        'catalogs': ?data['catalogs'],
        'connections': ?data['connections'],
        // The host's speech providers: programs and keys the app does not edit here but must write back as it found them.
        'speech': ?data['speech'],
        'skill_pins': ?data['skill_pins'],
        'loadouts': ?data['loadouts'],
        'defaults': ?data['defaults'],
        'store': ?rawStore,
      },
      agents: agents,
      host: host == null ? null : Map<String, dynamic>.from(host as Map),
      look: look, followProvider: flag(appearance, 'follow_provider', false), brightness: brightness, density: density,
      startHidden: flag(desktop, 'start_hidden', false),
      closeToTray: flag(desktop, 'close_to_tray', true),
      minimizeToTray: flag(desktop, 'minimize_to_tray', false),
      localHost: flag(desktop, 'local_host', true),
      preferWebTransport: flag(network, 'prefer_webtransport', true),
      connectTimeout: timeout, cdn: cdn,
      defaultMode: defaultMode as String?,
      allowOnceOnly: flag(permissions, 'allow_once_only', false),
      protectedPaths: protectedPaths,
      hiddenSources: {for (final name in hidden) name as String},
      pasteAsFile: limit('paste_as_file', defaultPasteAsFile),
      embedLimit: limit('embed_limit', defaultEmbedLimit),
      warnPermille: warn,
      limitPhrases: phrases == null ? null : [for (final phrase in phrases as List) phrase as String],
      voice: VoiceSettings(
        backend: voiceText('backend') ?? defaultVoiceBackend,
        model: voiceText('model'),
        language: voiceText('language'),
        device: voiceText('device'),
        mode: voiceMode == 'hold' ? DictationMode.hold : DictationMode.toggle,
      ),
      notifications: NotificationSettings(
        permission: flag(notifications, 'permission', true),
        question: flag(notifications, 'question', true),
        finished: flag(notifications, 'finished', true),
      ),
      sessions: LedgerSettings(
        keep: count('keep', LedgerSettings.defaultKeep),
        keepText: flag(sessions, 'keep_text', false),
        textMessages: count('text_messages', LedgerSettings.defaultTextMessages),
        workspace: workspace,
        harnesses: {for (final MapEntry(:key, :value) in harnesses.entries) '$key': value as String},
        whileWorking: whileWorking,
        queueDelivery: queueDelivery,
      ),
    );
  }

  /// The `[voice]` table, written only when something in it is not the default.
  String _voiceToml() {
    final lines = [
      if (voice.backend != defaultVoiceBackend) 'backend = ${jsonEncode(voice.backend)}',
      if (voice.model != null) 'model = ${jsonEncode(voice.model)}',
      if (voice.language != null) 'language = ${jsonEncode(voice.language)}',
      if (voice.device != null) 'device = ${jsonEncode(voice.device)}',
      if (voice.mode != DictationMode.toggle) 'mode = ${jsonEncode(voice.mode.name)}',
    ];
    return lines.isEmpty ? '' : '\n[voice]\n${lines.join('\n')}\n';
  }

  String _encodeToml() => '''
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
follow_provider = $followProvider
brightness = "$brightness"
density = "${density.name}"

[sessions]
keep = ${sessions.keep}
keep_text = ${sessions.keepText}
text_messages = ${sessions.textMessages}
${sessions.workspace == defaultWorkspace ? '' : 'workspace = ${jsonEncode(sessions.workspace)}\n'}${sessions.whileWorking == WhileWorking.queue ? '' : 'while_working = "${sessions.whileWorking.name}"\n'}${sessions.queueDelivery == QueueDelivery.merge ? '' : 'queue_delivery = "${sessions.queueDelivery.name}"\n'}
[notifications]
permission = ${notifications.permission}
question = ${notifications.question}
finished = ${notifications.finished}

[attachments]
paste_as_file = $pasteAsFile
embed_limit = $embedLimit
${_voiceToml()}${warnPermille == defaultWarnPermille && limitPhrases == null ? '' : '\n[usage]\n${warnPermille == defaultWarnPermille ? '' : 'warn_permille = $warnPermille\n'}${limitPhrases == null ? '' : 'limit_phrases = ${jsonEncode(limitPhrases)}\n'}'}${defaultMode == null && !allowOnceOnly && protectedPaths == null ? '' : '\n[permissions]\n${defaultMode == null ? '' : 'default_mode = ${jsonEncode(defaultMode)}\n'}${allowOnceOnly ? 'allow_once_only = true\n' : ''}${protectedPaths == null ? '' : 'protected = [\n${protectedPaths!.map((line) => '  ${jsonEncode(line)},\n').join()}]\n'}'}${hiddenSources.isEmpty ? '' : '\n[sources]\nhidden = ${jsonEncode(hiddenSources.toList())}\n'}${sessions.harnesses.isEmpty ? '' : '\n[harnesses]\n${sessions.harnesses.entries.map((name) => '${jsonEncode(name.key)} = ${jsonEncode(name.value)}\n').join()}'}${agents.map((agent) => '''
[agents.${jsonEncode(agent.label)}]
program = ${jsonEncode(agent.program)}
args = ${jsonEncode(agent.args)}
''').join()}${tables.isEmpty ? '' : '\n${TomlDocument.fromMap(tables)}'}${host == null ? '' : '\n${TomlDocument.fromMap({'host': host})}'}''';

  /// canoa.cfg as the person's own file is written: in the format and with the first line it was read with, holding the
  /// keys the person changed or already had there and none that a lower layer gives the same.
  String encode() {
    final toml = _encodeToml();
    if (source.layers.isEmpty && source.format == ConfigFormat.toml) return source.marked ? '#!toml\n$toml' : toml;
    return encodeConfig(_ownDocument(), source.format, marked: source.marked);
  }

  static final _ownDocuments = Expando<Map<String, dynamic>>('own document');

  Map<String, dynamic> _ownDocument() => _ownDocuments[this] ??=
      ownDocument(TomlDocument.parse(_encodeToml()).toMap(), _lowerModel, source.own, _defaults);

  static final _lowerModels = Expando<Map<String, dynamic>>('lower layers as the model writes them');

  /// What the layers below the person's file hold, written the way this app writes a configuration, so that an entry of a
  /// lower layer that the model fills in (an agent with no `args`) still reads as the same entry and is not copied into the
  /// person's file.
  Map<String, dynamic> get _lowerModel => source.layers.isEmpty
      ? const {}
      : _lowerModels[source] ??= TomlDocument.parse(
          CanoaConfig.fromLayers((format: ConfigFormat.toml, marked: false, data: const <String, dynamic>{}), source.layers)._encodeToml(),
        ).toMap();

  static final Map<String, dynamic> _defaults = TomlDocument.parse(const CanoaConfig()._encodeToml()).toMap();

  /// Where the value at [path] (`['desktop', 'start_hidden']`) comes from when a lower layer gives it and the person's file
  /// does not, as Settings says it ("the system file"); null when the person's file holds it or nothing does.
  String? lowerLayerOf(List<String> path) {
    if (source.layers.isEmpty || originOf([(origin: ConfigOrigin.own, data: _ownDocument())], path) != null) return null;
    return originOf(source.layers, path)?.words;
  }

  /// The lower layer that gives the value at [path], by the words Settings uses; null when none does.
  String? lowerValueFrom(List<String> path) => originOf(source.layers, path)?.words;

  /// Whether the person's file holds the value at [path] over one a lower layer gives.
  bool overridesLowerLayer(List<String> path) =>
      source.layers.isNotEmpty &&
      originOf([(origin: ConfigOrigin.own, data: _ownDocument())], path) != null &&
      originOf(source.layers, path) != null;

  /// This configuration with the key at [path] taken out of the person's file, so that the lower layers' value (or the
  /// default) shows again.
  CanoaConfig resetKey(List<String> path) => CanoaConfig.fromLayers(
    (format: source.format, marked: source.marked, data: withoutKey(_ownDocument(), path)),
    source.layers,
  );

  /// A profile table of canoa.cfg, its refusal named by [where] and never quoting a value.
  static Profile _profile(String where, Map<dynamic, dynamic> table, LayerKind kind) {
    try {
      return Profile.fromMap(table, kind);
    } on ProfileException catch (error) {
      throw FormatException('canoa.cfg: $where: $error');
    }
  }
}

/// One `[loadouts.NAME]`: a named collection a session starts with. Its overlays are whole and apply in the order `[overlays]`
/// composes them.
final class Loadout {
  const Loadout({this.description = '', this.overlays = const [], this.skills = const [], this.mcp = const []});

  /// What the loadout is for.
  final String description;

  /// Overlays by the name `[overlays]` gives them.
  final List<String> overlays;

  /// The skills the person chose, by name.
  final List<String> skills;

  /// The servers of `[profile.mcp]` it starts.
  final List<String> mcp;

  static const _keys = {'description', 'overlays', 'skills', 'mcp'};

  /// The loadout [name] as canoa.cfg has it. Throws [FormatException] for a key it does not have or a name that is not one.
  factory Loadout.read(String name, Map<dynamic, dynamic> table) {
    List<String> names(String key, bool Function(String) valid) {
      final raw = table[key];
      if (raw == null) return const [];
      if (raw is! List || raw.any((item) => item is! String || !valid(item))) {
        throw FormatException('canoa.cfg: loadouts.$name.$key is a list of names');
      }
      final out = <String>[];
      for (final item in raw.cast<String>()) {
        if (!out.any((held) => held.toLowerCase() == item.toLowerCase())) out.add(item);
      }
      return out;
    }

    for (final key in table.keys) {
      if (!_keys.contains(key)) {
        throw FormatException('canoa.cfg: loadouts.$name.$key is not a loadout setting: description, overlays, skills and mcp are');
      }
    }
    final description = table['description'];
    if (description != null && description is! String) throw FormatException('canoa.cfg: loadouts.$name.description is text');
    return Loadout(
      description: (description as String?) ?? '',
      overlays: names('overlays', validOverlayName),
      skills: names('skills', validSkillName),
      mcp: names('mcp', isEntryName),
    );
  }

  Loadout copyWith({String? description, List<String>? overlays, List<String>? skills, List<String>? mcp}) => Loadout(
    description: description ?? this.description,
    overlays: overlays ?? this.overlays,
    skills: skills ?? this.skills,
    mcp: mcp ?? this.mcp,
  );

  /// This loadout with the server [name] in it, or without.
  Loadout withMcp(String name, bool on) => copyWith(mcp: [
        for (final each in mcp) if (each != name) each,
        if (on) name,
      ]);

  Map<String, dynamic> toTable() => {
    if (description.isNotEmpty) 'description': description,
    if (overlays.isNotEmpty) 'overlays': overlays,
    if (skills.isNotEmpty) 'skills': skills,
    if (mcp.isNotEmpty) 'mcp': mcp,
  };
}

/// What a session starts with once its loadout is resolved, as the host resolves it when the session starts.
final class ResolvedLoadout {
  const ResolvedLoadout({required this.name, required this.overlays, required this.missingOverlays, required this.skills, required this.mcp});

  final String name;

  /// Its installed overlays, in the order `[overlays]` composes them.
  final List<String> overlays;

  /// Overlays it names that `[overlays]` does not list.
  final List<String> missingOverlays;
  final List<String> skills, mcp;

  Map<String, Object?> toJson() =>
      {'name': name, 'overlays': overlays, 'missing_overlays': missingOverlays, 'skills': skills, 'mcp': mcp};
}

/// A session named a loadout `[loadouts]` does not hold.
final class UnknownLoadout implements Exception {
  const UnknownLoadout(this.name);

  final String name;

  @override
  String toString() => 'there is no loadout named $name';
}

/// Where an MCP server the store installed came from: a registry, the server's name and version there, and the digest of the
/// `server.json` that was shown before it was installed.
final class McpOrigin {
  const McpOrigin({required this.registry, required this.server, required this.version, required this.digest});

  /// The registry's address, as the person added it.
  final String registry;
  final String server, version;

  /// `sha256:` and 64 hex digits of the canonical `server.json`.
  final String digest;

  static McpOrigin? fromMap(Map<dynamic, dynamic> map) {
    if (map.keys.any((key) => !const {'registry', 'server', 'version', 'digest'}.contains(key))) return null;
    final registry = map['registry'], server = map['server'], version = map['version'], digest = map['digest'];
    if (registry is! String || server is! String || version is! String || digest is! String) return null;
    if (!RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(digest)) return null;
    return McpOrigin(registry: registry, server: server, version: version, digest: digest);
  }

  Map<String, dynamic> toMap() => {'registry': registry, 'server': server, 'version': version, 'digest': digest};
}
