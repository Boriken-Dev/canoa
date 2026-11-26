import 'dart:convert' show jsonEncode;

import 'package:coqui_aos/coqui_aos.dart' show normalizeName, validOverlayName;
import 'package:material_ui/material_ui.dart' show ThemeMode;
import 'package:toml/toml.dart';

import '../accounts/account_models.dart' show AccountEntry;
import '../acp/agent_commands.dart';
import '../brand.dart';
import '../providers/provider_catalog.dart';
import '../voice/speech_backend.dart' show VoiceProfile;
import 'catalog_specs.dart';
import 'config_layers.dart';
import 'connection.dart';
import '../chat/modes.dart' show modeOrder;
import '../ledger/session_ledger.dart';
import '../profile/aos_support.dart' show holdsPassword;
import '../permissions/protected_paths.dart' show protectedLinesOf;
import '../profile/skill_copies.dart' show skillKey, skillsDirectory, validSkillName;
import '../profile/profile.dart';
import '../store/skill_sources.dart';
import '../store/store_settings.dart';
import '../popout/open_in.dart';
import '../theme/interface_zoom.dart' show maxZoom, minZoom;
import '../theme/look.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity, defaultWarnPermille;
import '../workspace/workspace_template.dart' show defaultWorkspace, workspaceProblem;


part 'canoa_config_models.dart';
part 'canoa_config_loadouts.dart';

/// canoa.cfg is one file in TOML, YAML or JSON, named by its first line, found and merged by layers (see
/// `config_layers.dart`). This file must never contain secrets or private keys.
final class CanoaConfig {
  const CanoaConfig({
    this.startHidden = false,
    this.closeToTray = true,
    this.minimizeToTray = false,
    this.localHost = true,
    this.startAtLogin,
    this.preferWebTransport = true,
    this.connectTimeout = 10,
    this.look = LookId.canoa,
    this.followProvider = false,
    this.brightness = 'dark',
    this.density = ChatDensity.normal,
    this.openIn = OpenIn.thisWindow,
    this.language = '',
    this.viewers = const {},
    this.zoom = 1,
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
    this.voice = const VoiceSettings(),
    this.hiddenSources = const {},
    this.shortcuts = const {},
    this.pasteAsFile = defaultPasteAsFile,
    this.embedLimit = defaultEmbedLimit,
    this.warnPermille = defaultWarnPermille,
    this.limitPhrases,
    this.source = const ConfigSource(),
  });

  /// The defaults, for a canoa.cfg that could not be read for [reason]: [unreadable] says so, and saving is refused.
  factory CanoaConfig.unreadable(String reason) => CanoaConfig(source: ConfigSource(unreadable: reason));

  /// Where this configuration was read from, which a save writes back to.
  final ConfigSource source;

  /// Why canoa.cfg could not be read, when this is the defaults the app runs on instead; null for a configuration that was read.
  String? get unreadable => source.unreadable;

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

  /// `[desktop] start_at_login`: whether the app starts, hidden in the tray, when this person signs in to the computer. Null until
  /// the first run decided it, so that choosing once is never undone by a later start.
  final bool? startAtLogin;
  /// `[network] connect_timeout`: seconds a connection may take before the app gives up; any number above 0.
  final num connectTimeout;

  /// [connectTimeout] as a duration.
  Duration get connectTimeoutDuration => Duration(milliseconds: (connectTimeout * 1000).round());
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

  /// `[voice]`: the speech service and how dictation is held.
  final VoiceSettings voice;

  /// `[sources] hidden`: the presets, by label, that Settings keeps out of the new-session picker.
  final Set<String> hiddenSources;

  /// `[shortcuts]`: per-shortcut key chord overrides by shortcut id (`palette = "Ctrl+Shift+K"`).
  final Map<String, String> shortcuts;

  /// `light`, `dark`, or `system` to follow the machine.
  final String brightness;

  /// `[appearance] density`: how much of an agent's work a conversation shows, `normal`, `thinking` or `verbose`.
  final ChatDensity density;

  /// `[appearance] open_in`: where a plain click on a conversation, and Open on a file or a terminal, shows it.
  final OpenIn openIn;

  /// `[appearance] language`: the language the app's own words are in, as its code (`en`, `es`); empty follows the system's.
  final String language;

  /// `[appearance.viewers]`: the viewer the person chose for each type of file, by MIME type (`"application/json" = "text"`).
  final Map<String, String> viewers;
  /// `[appearance] zoom`: the factor the whole interface is drawn at; 1 is the size the app was designed for.
  final double zoom;

  ThemeMode get themeMode => switch (brightness) {
    'light' => ThemeMode.light,
    'system' => ThemeMode.system,
    _ => ThemeMode.dark,
  };

  CanoaConfig copyWith({bool? startHidden, bool? closeToTray,
      bool? minimizeToTray, bool? startAtLogin, bool? preferWebTransport, num? connectTimeout,
      LookId? look, bool? followProvider, String? brightness, ChatDensity? density, OpenIn? openIn, String? language, Map<String, String>? viewers, double? zoom, LedgerSettings? sessions, String? defaultMode, bool clearDefaultMode = false, bool? allowOnceOnly,
      List<String>? protectedPaths, bool clearProtectedPaths = false,
      VoiceSettings? voice, List<AgentCommand>? agents, Set<String>? hiddenSources, Map<String, String>? shortcuts, int? warnPermille}) =>
      CanoaConfig(
        startHidden: startHidden ?? this.startHidden,
        closeToTray: closeToTray ?? this.closeToTray,
        minimizeToTray: minimizeToTray ?? this.minimizeToTray,
        localHost: localHost,
        startAtLogin: startAtLogin ?? this.startAtLogin,
        preferWebTransport: preferWebTransport ?? this.preferWebTransport,
        connectTimeout: connectTimeout ?? this.connectTimeout,
        look: look ?? this.look,
        followProvider: followProvider ?? this.followProvider,
        brightness: brightness ?? this.brightness,
        density: density ?? this.density,
        openIn: openIn ?? this.openIn,
        language: language ?? this.language,
        viewers: viewers ?? this.viewers,
        zoom: zoom ?? this.zoom,
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
        voice: voice ?? this.voice,
        hiddenSources: hiddenSources ?? this.hiddenSources,
        shortcuts: shortcuts ?? this.shortcuts,
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
    startAtLogin: startAtLogin,
    preferWebTransport: preferWebTransport,
    connectTimeout: connectTimeout,
    look: look,
    followProvider: followProvider,
    brightness: brightness,
    density: density,
    openIn: openIn,
    language: language,
    viewers: viewers,
    zoom: zoom,
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
    voice: voice,
    hiddenSources: hiddenSources,
    shortcuts: shortcuts,
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

  /// This configuration with the agent entry [name] added to `[host.agents]`, which canoa serves on [listen]: what runs ([program] and
  /// [args], or a `harness` in [more]), and [more], the other keys of an entry as the host describes them.
  CanoaConfig withHostAgent({
    String? program,
    required String name,
    List<String> args = const [],
    required String listen,
    List<String> workingRoots = const [],
    String? usage,
    Map<String, Object?> more = const {},
  }) {
    if (hostEntries.any((entry) => entry.name == name)) {
      throw ArgumentError.value(name, 'name', 'Another entry has that name');
    }
    if ((program ?? '').trim().isEmpty && more['harness'] is! String) throw ArgumentError.value(program, 'program', 'Name a program or a harness');
    final copy = _deepCopy(host ?? const {});
    final table = (copy['agents'] as Map<String, dynamic>?) ?? <String, dynamic>{};
    table[name] = <String, dynamic>{
      'program': ?program,
      if (args.isNotEmpty) 'args': args,
      'listen': listen,
      if (workingRoots.isNotEmpty) 'working_roots': workingRoots,
      'usage': ?usage,
      ...more,
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

  /// The entry that is the agent itself for the provider [harness]: the source the host serves of its own for it when canoa.cfg or the
  /// host names it, else the first entry that runs the provider, else the name the host makes one with (a source's own, or the
  /// provider's with a capital).
  String ownEntryOf(String harness) {
    final running = hostEntriesRunning(harness);
    final source = providerCatalog.byId(harness)?.source;
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

  /// What the knowledge panel edits: `[host.kb.embedder]` and `[host.kb.ranker]` (the backend each `use`s and the model it asks for)
  /// and the one server of the person's own the panel keeps, `[sdk.knowledge]`. The keys are the panel's own choice of fields,
  /// which the host does not say; `host_constants_test.dart` holds each to a key the host reads.
  static const hostKbKeys = [
    'embedder_use',
    'embedder_model',
    'ranker_use',
    'ranker_model',
    'server',
  ];

  /// The name of the server table the panel keeps (`[sdk.knowledge]`).
  static const knowledgeServer = 'knowledge';

  /// The values of [hostKbKeys] the file holds.
  Map<String, String> get hostKb {
    final kb = host?['kb'];
    final out = <String, String>{};
    for (final role in const ['embedder', 'ranker']) {
      final table = kb is Map ? kb[role] : null;
      if (table is! Map) continue;
      for (final key in const ['use', 'model']) {
        if (table[key] case final String value when value.isNotEmpty) {
          out['${role}_$key'] = value;
        }
      }
    }
    final server = (tables['sdk'] as Map?)?[knowledgeServer];
    if (server is Map) {
      if (server['address'] case final String address when address.isNotEmpty) {
        out['server'] = address;
      }
    }
    return out;
  }

  /// This configuration with the [hostKbKeys] set to [values]. A role with no model is off; a role with a model and no `use` uses
  /// the server when one is named and the operation's own choice (`default`) when not. Every other key of `[host.kb]`, of `[host]`
  /// and of `[sdk]` stays as it was; a table with nothing left goes.
  CanoaConfig withHostKb(Map<String, String> values) {
    final copy = _deepCopy(host ?? const {});
    final kb = copy['kb'] is Map
        ? Map<String, dynamic>.of((copy['kb'] as Map).cast<String, dynamic>())
        : <String, dynamic>{};
    final server = values['server']?.trim() ?? '';
    for (final role in const ['embedder', 'ranker']) {
      final model = values['${role}_model']?.trim() ?? '';
      if (model.isEmpty) {
        kb.remove(role);
        continue;
      }
      var use = values['${role}_use']?.trim() ?? '';
      if (use.isEmpty) use = server.isEmpty ? 'default' : knowledgeServer;
      final table = kb[role] is Map
          ? Map<String, dynamic>.of((kb[role] as Map).cast<String, dynamic>())
          : <String, dynamic>{};
      table['use'] = use;
      table['model'] = model;
      kb[role] = table;
    }
    kb.isEmpty ? copy.remove('kb') : copy['kb'] = kb;
    final sdk = _deepCopy((tables['sdk'] as Map?) ?? const {});
    if (server.isEmpty) {
      sdk.remove(knowledgeServer);
    } else {
      final held = sdk[knowledgeServer] is Map
          ? Map<String, dynamic>.of(
              (sdk[knowledgeServer] as Map).cast<String, dynamic>(),
            )
          : <String, dynamic>{};
      sdk[knowledgeServer] = {
        ...held,
        'provider': 'server',
        'address': server,
        'capabilities': held['capabilities'] ?? const ['embed', 'chat'],
      };
    }
    return withHost(copy.isEmpty ? null : copy)._withTable('sdk', sdk);
  }

  /// `[assist.OPERATION]`: for each operation (`complete`, `embed`, `rerank`) the backend the host asks first (`use`) and the ones it
  /// asks after it (`fallback`), in order.
  Map<String, ({String? use, List<String> fallback})> get assistOperations {
    final table = tables['assist'];
    return {
      if (table is Map)
        for (final MapEntry(:key, :value) in table.entries)
          if (value is Map)
            '$key': (
              use: value['use'] is String ? value['use'] as String : null,
              fallback: [if (value['fallback'] is List) for (final each in value['fallback'] as List) if (each is String) each],
            ),
    };
  }

  /// This configuration with `[assist.OPERATION]` as [operations] says; an operation that names no backend and no fallback goes.
  CanoaConfig withAssistOperations(Map<String, ({String? use, List<String> fallback})> operations) {
    final raw = _deepCopy((tables['assist'] as Map?) ?? const {});
    for (final MapEntry(:key, :value) in operations.entries) {
      final held = raw[key] is Map ? Map<String, dynamic>.of((raw[key] as Map).cast<String, dynamic>()) : <String, dynamic>{};
      final use = value.use?.trim() ?? '';
      use.isEmpty ? held.remove('use') : held['use'] = use;
      value.fallback.isEmpty ? held.remove('fallback') : held['fallback'] = [...value.fallback];
      held.isEmpty ? raw.remove(key) : raw[key] = held;
    }
    return _withTable('assist', raw);
  }

  /// The keys of an `[assist_harness.NAME]` table, which the Settings page edits.
  static const assistHarnessKeys = ['source', 'title', 'profile', 'account', 'model', 'effort'];

  /// `[assist_harness.NAME]`: each agent harness offered as a backend, by name, with the keys it sets (text only).
  Map<String, Map<String, String>> get assistHarnesses {
    final table = tables['assist_harness'];
    return {
      if (table is Map)
        for (final MapEntry(:key, :value) in table.entries)
          if (value is Map)
            '$key': {
              for (final field in assistHarnessKeys)
                if (value[field] is String && (value[field] as String).isNotEmpty) field: value[field] as String,
            },
    };
  }

  /// This configuration with `[assist_harness]` as [harnesses] says, the keys the page does not edit of a table kept as they were.
  CanoaConfig withAssistHarnesses(Map<String, Map<String, String>> harnesses) {
    final held = _deepCopy((tables['assist_harness'] as Map?) ?? const {});
    final raw = <String, dynamic>{};
    for (final MapEntry(:key, :value) in harnesses.entries) {
      final fields = held[key] is Map ? Map<String, dynamic>.of((held[key] as Map).cast<String, dynamic>()) : <String, dynamic>{};
      for (final field in assistHarnessKeys) {
        final text = value[field]?.trim() ?? '';
        text.isEmpty ? fields.remove(field) : fields[field] = text;
      }
      raw[key] = fields;
    }
    return _withTable('assist_harness', raw);
  }

  /// `[assist_pool]`: how long a warm harness session is kept unasked (`idleSeconds`) and how many are kept (`maxSessions`).
  ({int? idleSeconds, int? maxSessions}) get assistPool {
    final table = tables['assist_pool'];
    int? number(String key) => table is Map && table[key] is int ? table[key] as int : null;
    return (idleSeconds: number('idle_seconds'), maxSessions: number('max_sessions'));
  }

  /// This configuration with `[assist_pool]` as given; a value that is null goes, and a table with none goes.
  CanoaConfig withAssistPool({int? idleSeconds, int? maxSessions}) => _withTable('assist_pool', {
    'idle_seconds': ?idleSeconds,
    'max_sessions': ?maxSessions,
  });

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

  /// `[store.overlays.catalogs]`: each catalog of overlays the Store lists that the `read` provider reads, by the label the
  /// person gives it, a git source whose `#path` names the catalog file. A catalog another provider reads is in
  /// [overlayCatalogs] only.
  Map<String, GitOverlay> get catalogs => _catalogs(catalogTable('overlays'));

  /// `[store.extensions.catalogs]`: each extensions catalog the Store lists that the `read` provider reads, by the label the
  /// person gives it, a git source (`<repository>@<ref>#<file>`) whose file lists extensions in Canoa's own format. The host reads
  /// them; none is listed until the table names one, so this is empty by default.
  Map<String, String> get extensionCatalogs => _extensionCatalogs(catalogTable('extensions'));

  static Map<String, String> _extensionCatalogs(Object? raw) {
    final out = <String, String>{};
    for (final MapEntry(:key, :value) in parseCatalogSpecs('extensions', raw).entries) {
      if (value.source case final source?) out[key] = source;
    }
    return out;
  }

  /// `[store.<type>.catalogs]` as the person's file and the layers below it merge: the raw table, or null.
  Object? catalogTable(String type) => _catalogsIn(tables['store'], type);

  static Object? _catalogsIn(Object? store, String type) {
    final kind = store is Map ? store[type] : null;
    return kind is Map ? kind['catalogs'] : null;
  }

  /// The catalogs of the Store type [type] as canoa.cfg names them, whichever provider reads each, in the file's order. A label
  /// set to `false` lists nothing. Nothing a default adds is here: the host says which defaults a type has.
  Map<String, CatalogSpec> catalogSpecs(String type) => parseCatalogSpecs(type, catalogTable(type));

  /// Every catalog of overlays: the default, unless the table names its label, then the table's own, whichever provider reads each.
  Map<String, CatalogSpec> get overlayCatalogs {
    final own = catalogSpecs('overlays');
    final named = own.keys.any((label) => normalizeName(label) == normalizeName(defaultCatalogLabel));
    final offFirst = (catalogTable('overlays') as Map?)?.keys.any((key) => normalizeName('$key') == normalizeName(defaultCatalogLabel)) ?? false;
    return {
      if (!named && !offFirst) defaultCatalogLabel: CatalogSpec.read(defaultCatalogSpec),
      ...own,
    };
  }

  /// Reads the table of catalogs of [type]. Throws [FormatException] for a label that is no label, one given twice as AOS compares
  /// names, or a value that is no catalog.
  static Map<String, CatalogSpec> parseCatalogSpecs(String type, Object? raw) {
    if (raw == null) return const {};
    if (raw is! Map) throw FormatException('${Brand.configFile}: store.$type.catalogs must be a table');
    final out = <String, CatalogSpec>{};
    final normalized = <String>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      final label = '$key';
      if (!isCatalogLabel(label) || !normalized.add(normalizeName(label))) {
        throw FormatException('${Brand.configFile}: store.$type.catalogs.$label: a label is a letter, then letters, digits, - or _, at most '
            '64, unique when compared as AOS compares names');
      }
      if (value == false) continue;
      out[label] = CatalogSpec.parse('store.$type.catalogs.$label', value);
    }
    return out;
  }

  /// This configuration with the catalog [spec] of the Store type [type] labelled [label]. Throws [FormatException].
  CanoaConfig withCatalogOf(String type, String label, CatalogSpec spec) {
    final raw = _deepCopy((catalogTable(type) as Map?) ?? const {})..[label] = spec.toRaw();
    parseCatalogSpecs(type, raw);
    return _withCatalogTable(type, raw);
  }

  /// This configuration without the catalog [label] of the Store type [type]. A [fallback] label (the type's default) is recorded
  /// as `false` in this layer, so it stays gone without touching a lower layer.
  CanoaConfig withoutCatalogOf(String type, String label, {String? fallback}) {
    final raw = _deepCopy((catalogTable(type) as Map?) ?? const {});
    final key = raw.keys.cast<String>().where((each) => normalizeName(each) == normalizeName(label)).firstOrNull;
    if (fallback != null && normalizeName(label) == normalizeName(fallback)) {
      raw[key ?? fallback] = false;
    } else {
      raw.remove(key ?? label);
    }
    return _withCatalogTable(type, raw);
  }

  /// This configuration with [raw] as the table of catalogs of [type], and every other key of `[store]` as it was.
  CanoaConfig _withCatalogTable(String type, Map<String, dynamic> raw) {
    final store = {...((tables['store'] as Map?) ?? const {}).cast<String, dynamic>()};
    final kind = {...((store[type] as Map?) ?? const {}).cast<String, dynamic>()}..remove('catalogs');
    if (raw.isNotEmpty) kind['catalogs'] = raw;
    kind.isEmpty ? store.remove(type) : store[type] = kind;
    return _withTable('store', store);
  }

  /// `[skill_pins]`: the tree digest the person requires of a skill from elsewhere, by its name in lower case. A skill is
  /// fetched only when its digest equals this and the pin its entry or catalog gives.
  Map<String, String> get skillPins => _skillPins(tables['skill_pins']);

  static Map<String, String> _skillPins(Object? raw) {
    if (raw == null) return const {};
    if (raw is! Map) throw const FormatException('${Brand.configFile}: skill_pins must be a table');
    final out = <String, String>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      final name = '$key'.toLowerCase();
      if (!validSkillName('$key')) {
        throw FormatException('${Brand.configFile}: skill_pins.$key is not a skill name');
      }
      if (value is! String || !RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(value)) {
        throw FormatException('${Brand.configFile}: skill_pins.$key is a sha256: tree digest');
      }
      if (out.containsKey(name)) throw FormatException('${Brand.configFile}: skill_pins.$key repeats a name');
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

  /// `[defaults.clients]`: the loadout a session starts with when it names none and its client is of that kind,
  /// by kind, as written (which kinds the host knows is the host's to say): a name `[loadouts]` no longer holds (deleted by hand) is kept, and is no default. A kind with none starts
  /// with [defaultLoadout].
  Map<String, String> get clientLoadouts => _clientLoadouts(tables['defaults']);

  /// The loadout a session that names none starts with when its client is of kind [client] (null for a client with none): the kind's
  /// own when canoa.cfg gives it one that it holds, else [defaultLoadout].
  String? defaultLoadoutFor(String? client) {
    final named = client == null ? null : clientLoadouts[client];
    return named != null && loadouts.containsKey(named) ? named : defaultLoadout;
  }

  /// The loadout canoa.cfg gives clients of kind [client] that it no longer holds, which the default replaces; null when there is
  /// none such.
  String? lostClientLoadout(String? client) {
    final named = client == null ? null : clientLoadouts[client];
    return named != null && !loadouts.containsKey(named) ? named : null;
  }

  /// The loadout a session starts with, as the host resolves it: [loadout] when canoa.cfg holds it, else the one the profile
  /// [preset] names (the default when canoa.cfg no longer holds that), else the one for a client of kind [client], else the default.
  String? loadoutOfStart({String? loadout, String? preset, String? client}) {
    if (loadout != null && loadouts.containsKey(loadout)) return loadout;
    final named = presets[preset]?.loadout;
    if (named != null) return loadouts.containsKey(named) ? named : defaultLoadout;
    return defaultLoadoutFor(client);
  }

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
    if (raw is! Map) throw const FormatException('${Brand.configFile}: loadouts must be a table');
    final out = <String, Loadout>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      if (!isEntryName('$key')) {
        throw FormatException('${Brand.configFile}: loadouts.$key: a loadout is named with 1 to 64 letters, digits, - or _');
      }
      if (value is! Map) throw FormatException('${Brand.configFile}: loadouts.$key must be a table');
      out['$key'] = Loadout.read('$key', value);
    }
    return out;
  }

  static String? _defaultLoadout(Object? raw, Map<String, Loadout> loadouts) {
    if (raw == null) return null;
    if (raw is! Map || raw.keys.any((key) => key != 'loadout' && key != 'clients')) {
      throw const FormatException('${Brand.configFile}: defaults is a table with the keys loadout and clients');
    }
    _clientLoadouts(raw);
    final name = raw['loadout'];
    if (name == null) return null;
    if (name is! String || !loadouts.containsKey(name)) {
      throw const FormatException('${Brand.configFile}: defaults.loadout is the name of a loadout in loadouts');
    }
    return name;
  }

  static Map<String, String> _clientLoadouts(Object? raw) {
    final clients = raw is Map ? raw['clients'] : null;
    if (clients == null) return const {};
    if (clients is! Map) throw const FormatException('${Brand.configFile}: defaults.clients must be a table');
    final out = <String, String>{};
    for (final MapEntry(:key, :value) in clients.entries) {
      if (value is! String) throw FormatException('${Brand.configFile}: defaults.clients.$key is the name of a loadout');
      out['$key'] = value;
    }
    return out;
  }

  /// This configuration with [loadouts] as `[loadouts]`, and [defaultName] as `[defaults] loadout` (kept as it is when omitted;
  /// dropped when it names a loadout that is gone). A client's default that names a loadout that is gone is dropped too, and one
  /// that names a loadout in [renamed] (old name to new) follows it.
  CanoaConfig withLoadouts(Map<String, Loadout> loadouts, {String? defaultName, Map<String, String> renamed = const {}}) {
    final keep = defaultName ?? defaultLoadout;
    final named = keep != null && loadouts.containsKey(keep) ? keep : null;
    final defaults = {...((tables['defaults'] as Map?) ?? const {}).cast<String, dynamic>()}
      ..remove('loadout')
      ..remove('clients');
    final clients = {
      for (final MapEntry(:key, :value) in clientLoadouts.entries)
        if (loadouts.containsKey(renamed[value] ?? value)) key: renamed[value] ?? value,
    };
    return _withTable('loadouts', {for (final MapEntry(:key, :value) in loadouts.entries) key: value.toTable()})
        ._withTable('defaults', {...defaults, 'loadout': ?named, if (clients.isNotEmpty) 'clients': clients});
  }

  /// `[profiles.NAME]`: the named presets of a session's own values (source, model, effort, mode, starting instructions, cache
  /// lifetime and a loadout), in the order written. `[profile]` stays the person's own layer that every session gets.
  ///
  /// The default profile, [presetDefault], is always the first: it is the one a session that names no other starts with, and it
  /// is there, setting nothing, until the person edits it.
  Map<String, Preset> get presets => _presets(tables['profiles']);

  static Map<String, Preset> _presets(Object? raw) {
    if (raw != null && raw is! Map) throw const FormatException('${Brand.configFile}: profiles must be a table');
    final read = <String, Preset>{};
    for (final MapEntry(:key, :value) in (raw as Map? ?? const {}).entries) {
      if (!isPresetName('$key')) {
        throw FormatException('${Brand.configFile}: profiles.$key: $presetNameRule');
      }
      if (value is! Map) throw FormatException('${Brand.configFile}: profiles.$key must be a table');
      read['$key'] = Preset.read('$key', value);
    }
    return {presetDefault: read.remove(presetDefault) ?? const Preset(), ...read};
  }

  /// This configuration with [presets] as `[profiles]`. The default profile is written whatever is given: it cannot be removed.
  CanoaConfig withPresets(Map<String, Preset> presets) => _withTable('profiles', {
    presetDefault: (presets[presetDefault] ?? const Preset()).toTable(),
    for (final MapEntry(:key, :value) in presets.entries)
      if (key != presetDefault) key: value.toTable(),
  });

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

  /// `[store.mcps.catalogs]`: the MCP registries the person added beside the official one, in the order added, that the `read`
  /// provider reads. Each is the address of a registry that serves the MCP Registry's API, or a git source
  /// (`<repository>@<ref>#<file>`) of a file in that API's list shape. Whether an address can be read is for the page to say when
  /// it is used.
  List<String> get storeRegistries => _storeExtras(tables['store']).registries;

  /// The registries by the label each is kept under.
  Map<String, String> get storeRegistryLabels => _registryLabels(catalogTable('mcps'));

  static Map<String, String> _registryLabels(Object? raw) {
    final out = <String, String>{};
    for (final MapEntry(:key, :value) in parseCatalogSpecs('mcps', raw).entries) {
      final address = value.source;
      if (address == null) continue;
      if (address.runes.any((rune) => rune <= 0x20 || rune == 0x7f)) {
        throw FormatException('${Brand.configFile}: store.mcps.catalogs.$key is the address of a registry, with no spaces or control characters');
      }
      out[key] = address;
    }
    if (out.length > 16) throw const FormatException('${Brand.configFile}: store.mcps.catalogs lists at most 16 registries beside the official one');
    if (out.values.toSet().length != out.length) throw const FormatException('${Brand.configFile}: store.mcps.catalogs lists a registry twice');
    return out;
  }

  static ({Map<String, McpOrigin> mcp, List<String> registries}) _storeExtras(Object? raw) {
    if (raw == null) return (mcp: const {}, registries: const []);
    if (raw is! Map) throw const FormatException('${Brand.configFile}: store must be a table');
    final registries = _registryLabels(_catalogsIn(raw, 'mcps'));
    final mcp = <String, McpOrigin>{};
    final rawMcp = raw['mcp'];
    if (rawMcp != null && rawMcp is! Map) throw const FormatException('${Brand.configFile}: store.mcp must be a table');
    for (final MapEntry(:key, :value) in ((rawMcp as Map?) ?? const {}).entries) {
      final origin = value is Map ? McpOrigin.fromMap(value) : null;
      if (!isEntryName('$key') || origin == null) {
        throw FormatException('${Brand.configFile}: store.mcp.$key is a table of registry, server, version and digest');
      }
      mcp['$key'] = origin;
    }
    return (mcp: mcp, registries: registries.values.toList());
  }

  /// This configuration with [mcp] and [registries] (each left as it is when omitted) in `[store]`, beside its limits. A registry
  /// already kept keeps its label; a new one is labelled for its host.
  CanoaConfig _withStoreExtras({Map<String, McpOrigin>? mcp, List<String>? registries}) {
    final now = _storeExtras(tables['store']);
    final mcpNow = mcp ?? now.mcp;
    var out = this;
    if (registries != null) {
      final raw = _deepCopy((catalogTable('mcps') as Map?) ?? const {});
      final kept = _registryLabels(raw);
      // Registries no longer wanted go, those the `read` provider does not read stay.
      for (final MapEntry(:key, :value) in kept.entries) {
        if (!registries.contains(value)) raw.remove(key);
      }
      final taken = {for (final key in raw.keys) normalizeName(key)};
      for (final address in registries) {
        if (kept.containsValue(address)) continue;
        final label = _registryLabel(address, taken);
        taken.add(normalizeName(label));
        raw[label] = address;
      }
      out = out._withCatalogTable('mcps', raw);
    }
    final table = {...((out.tables['store'] as Map?) ?? const {}).cast<String, dynamic>()}..remove('mcp');
    return out._withTable('store', {
      ...table,
      if (mcpNow.isNotEmpty) 'mcp': {for (final MapEntry(:key, :value) in mcpNow.entries) key: value.toMap()},
    });
  }

  /// A label for the registry at [address]: its host, as a catalog label, made different from those [taken].
  static String _registryLabel(String address, Set<String> taken) {
    final host = Uri.tryParse(address)?.host ?? '';
    var base = host.replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '_');
    if (base.isEmpty || !RegExp(r'^[A-Za-z]').hasMatch(base)) base = 'registry_$base';
    if (base.length > 60) base = base.substring(0, 60);
    var label = base;
    for (var n = 2; taken.contains(normalizeName(label)) || !isCatalogLabel(label); n++) {
      label = '${base}_$n';
    }
    return label;
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

  /// `[store.skills.catalogs]`: where the Skills page looks for skills, by the label the person gives each, as `npx skills add` takes a
  /// source (`owner/repo`, a git URL, a folder). Anthropic's `anthropics/skills` is first unless the table names its label: `false`
  /// removes it, a source replaces it. A source another provider reads is in [catalogSpecs] only.
  Map<String, SkillSource> get skillSources => _skillSources(catalogTable('skills'));

  /// The most skill sources the person adds beside the default.
  static const maxSkillSources = 32;

  static Map<String, SkillSource> _skillSources(Object? raw) {
    final specs = parseCatalogSpecs('skills', raw);
    final named = {for (final key in (raw as Map? ?? const {}).keys) normalizeName('$key')};
    final given = <String, SkillSource>{};
    for (final MapEntry(:key, :value) in specs.entries) {
      final spec = value.source;
      if (spec == null) continue;
      try {
        given[key] = SkillSource.parse(spec);
      } on FormatException catch (error) {
        throw FormatException('${Brand.configFile}: store.skills.catalogs.$key: ${error.message}');
      }
    }
    if (given.length > maxSkillSources) {
      throw const FormatException('${Brand.configFile}: store.skills.catalogs holds at most $maxSkillSources sources');
    }
    final own = given.keys.where((label) => normalizeName(label) == normalizeName(defaultSkillSourceLabel));
    return {
      if (!named.contains(normalizeName(defaultSkillSourceLabel)))
        defaultSkillSourceLabel: SkillSource.parse(defaultSkillSourceSpec),
      for (final label in own) label: given[label]!,
      for (final MapEntry(:key, :value) in given.entries)
        if (!own.contains(key)) key: value,
    };
  }

  /// This configuration with the skill source [spec] labelled [label]. Throws [FormatException].
  CanoaConfig withSkillSource(String label, String spec) {
    final raw = _deepCopy((catalogTable('skills') as Map?) ?? const {})..[label] = spec;
    _skillSources(raw);
    return _withCatalogTable('skills', raw);
  }

  /// This configuration without the skill source [label]. The default one is recorded as `false` in this layer, so it stays gone
  /// without touching a lower layer.
  CanoaConfig withoutSkillSource(String label) => withoutCatalogOf('skills', label, fallback: defaultSkillSourceLabel);

  /// `[store] skills_search`: the skills directory the Skills page searches, as the host asks it. Absent or `false` is off, which
  /// it is until the person turns it on; `true` is [defaultSkillsSearch]; an address is another directory that answers the same.
  /// Null when off.
  String? get skillsSearch => skillsSearchWithDefault(defaultSkillsSearch);

  /// Like [skillsSearch], resolving `true` with [defaultAddress] (for example, the host's reported default).
  String? skillsSearchWithDefault(String defaultAddress) => _skillsSearch((tables['store'] as Map?)?['skills_search'], defaultAddress: defaultAddress);

  /// The directory `skills_search = true` names, which the Skills page shows by name while the host is not asked. The host reads
  /// the same address; `host_constants_test.dart` holds them equal.
  static const defaultSkillsSearch = 'https://skills.sh';

  static String? _skillsSearch(Object? raw, {String defaultAddress = defaultSkillsSearch}) {
    switch (raw) {
      case null || false:
        return null;
      case true:
        return defaultAddress;
      case final String address:
        final uri = Uri.tryParse(address);
        final loopback = uri != null && (uri.host == 'localhost' || uri.host == '127.0.0.1' || uri.host == '[::1]' || uri.host == '::1');
        if (uri == null ||
            address.runes.any((rune) => rune <= 0x20 || rune == 0x7f || rune == 0x5c) ||
            !(uri.scheme == 'https' || (uri.scheme == 'http' && loopback)) ||
            uri.host.isEmpty ||
            uri.userInfo.isNotEmpty ||
            uri.hasQuery ||
            uri.hasFragment) {
          throw const FormatException('${Brand.configFile}: store.skills_search is true, false, or the address of a skills directory: https, or http to this computer, with no user information, query or fragment');
        }
        return address.replaceAll(RegExp(r'/+$'), '');
      default:
        throw const FormatException('${Brand.configFile}: store.skills_search is true, false, or the address of a skills directory');
    }
  }

  /// This configuration with the skills search turned [on] (`true` in `[store]`) or off (the key removed).
  CanoaConfig withSkillsSearch(bool on) {
    final table = {...((tables['store'] as Map?) ?? const {}).cast<String, dynamic>()}..remove('skills_search');
    return _withTable('store', {...table, if (on) 'skills_search': true});
  }

  /// This configuration with [registries] as `[store.mcps.catalogs]`, in the order given.
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
    return {for (final MapEntry(:key, :value) in raw.entries) if (!StoreSettings.keys.contains(key)) '$key': value};
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
      if (table is! Map) throw FormatException('${Brand.configFile}: agents.$agent.mcp must be a table');
      final choices = <String, Object>{};
      for (final MapEntry(:key, value: choice) in table.entries) {
        if (!isEntryName('$key') ||
            !(choice is bool || (choice is String && RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(choice)))) {
          throw FormatException('${Brand.configFile}: agents.$agent.mcp.$key is true, false, or the sha256: digest of what was '
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
      if (table is! Map) throw FormatException('${Brand.configFile}: agents.$agent.hooks must be a table');
      final allowed = <String, String>{};
      for (final MapEntry(:key, value: digest) in table.entries) {
        if (!isEntryName('$key') || digest is! String || !RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(digest)) {
          throw FormatException('${Brand.configFile}: agents.$agent.hooks.$key is the sha256: digest of the overlay\'s files that were '
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

  /// The catalogs of overlays the `read` provider reads, in effect: the default first, unless the table names its label (`false`
  /// removes it, a source replaces it), then the table's own, a label set to `false` listing nothing.
  static Map<String, GitOverlay> _catalogs(Object? raw) {
    final specs = parseCatalogSpecs('overlays', raw);
    final named = {for (final key in (raw as Map? ?? const {}).keys) normalizeName('$key')};
    final given = <String, GitOverlay>{};
    for (final MapEntry(:key, :value) in specs.entries) {
      final spec = value.source;
      if (spec != null) given[key] = _catalogSource(key, spec);
    }
    final own = given.keys.where((label) => normalizeName(label) == normalizeName(defaultCatalogLabel));
    return {
      if (!named.contains(normalizeName(defaultCatalogLabel))) defaultCatalogLabel: _catalogSource(defaultCatalogLabel, defaultCatalogSpec),
      for (final label in own) label: given[label]!,
      for (final MapEntry(:key, :value) in given.entries)
        if (!own.contains(key)) key: value,
    };
  }

  /// Whether [label] can name a catalog: a valid overlay name that is also a name Canoa gives a directory.
  static bool isCatalogLabel(String label) => isEntryName(label) && validOverlayName(label);

  static GitOverlay _catalogSource(String label, String spec) {
    final GitOverlay source;
    try {
      source = OverlaySource.parseGit(spec);
    } on ProfileException {
      throw FormatException('${Brand.configFile}: store.overlays.catalogs.$label is not <repository>@<ref>#<catalog file> of a source git may '
          'fetch');
    }
    if (source.path.isEmpty) throw FormatException('${Brand.configFile}: store.overlays.catalogs.$label names no catalog file after #');
    if (holdsPassword(source.repo)) {
      throw FormatException('${Brand.configFile}: store.overlays.catalogs.$label holds a password, which ${Brand.configFile} may not; use a credential '
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
    if (raw is! Map) throw const FormatException('${Brand.configFile}: connections must be a table');
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

  /// This configuration with the catalog [spec] labelled [label] in `[store.overlays.catalogs]`. Throws [FormatException].
  CanoaConfig withCatalog(String label, String spec) {
    final raw = _deepCopy((catalogTable('overlays') as Map?) ?? const {})..[label] = spec;
    _catalogs(raw);
    return _withCatalogTable('overlays', raw);
  }

  /// This configuration without the catalog [label] of overlays. The default one is recorded as `false` in this layer, so it stays
  /// gone without touching a lower layer.
  CanoaConfig withoutCatalog(String label) => withoutCatalogOf('overlays', label, fallback: defaultCatalogLabel);

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
    startAtLogin: startAtLogin,
    preferWebTransport: preferWebTransport,
    connectTimeout: connectTimeout,
    look: look,
    followProvider: followProvider,
    brightness: brightness,
    density: density,
    openIn: openIn,
    language: language,
    viewers: viewers,
    zoom: zoom,
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
    voice: voice,
    hiddenSources: hiddenSources,
    shortcuts: shortcuts,
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

  /// The keys of each table the app edits through its own fields. Any other key of one of them is the host's or a later version's:
  /// kept as found, never refused here (the host says what it refuses, through `config/check`).
  static const _editedKeys = <String, Set<String>>{
    'desktop': {'start_hidden', 'close_to_tray', 'minimize_to_tray', 'local_host', 'start_at_login'},
    'network': {'prefer_webtransport', 'connect_timeout'},
    'appearance': {'look', 'follow_provider', 'brightness', 'density', 'open_in', 'language', 'viewers', 'zoom'},
    'sessions': {'keep', 'keep_text', 'text_messages', 'workspace', 'while_working', 'queue_delivery'},
    'permissions': {'default_mode', 'full_access', 'allow_once_only', 'protected'},
    'sources': {'hidden'},
    'attachments': {'paste_as_file', 'embed_limit'},
    'usage': {'warn_permille', 'limit_phrases'},
    'voice': {
      'backend', 'model', 'language', 'device', 'mode', 'accent', 'voice', 'vocabulary', 'acknowledged',
      'output', 'output_voice', 'output_model', 'output_style', 'output_speed', 'read_replies',
    },
  };

  /// The keys the layers and the person's file hold in the tables of [_editedKeys] that the app has no field for, by table.
  Map<String, Map<String, dynamic>> get _unknownKeys {
    var merged = <String, dynamic>{};
    for (final layer in source.layers) {
      merged = mergeConfig(merged, layer.data);
    }
    merged = mergeConfig(merged, source.own);
    return {
      for (final MapEntry(key: name, value: known) in _editedKeys.entries)
        if (merged[name] case final Map<dynamic, dynamic> table)
          if (table.keys.any((key) => !known.contains(key)))
            name: {for (final MapEntry(:key, :value) in table.entries) if (!known.contains(key)) '$key': value},
    };
  }

  /// What this configuration writes, as a document: its own fields and, in their tables, the keys it did not read.
  Map<String, dynamic> _modelDocument() {
    final model = TomlDocument.parse(_encodeToml()).toMap();
    for (final MapEntry(:key, :value) in _unknownKeys.entries) {
      final held = model[key];
      model[key] = {...(held is Map ? held.cast<String, dynamic>() : const <String, dynamic>{}), ...value};
    }
    return model;
  }

  /// The keys of each table the app edits through its own fields, by table.
  static Map<String, Set<String>> get editedKeys => _editedKeys;

  /// The tables of canoa.cfg this app reads.
  static Set<String> get ownTables => _ownTables;

  /// The tables of canoa.cfg this app reads; any other is kept as it was found.
  static const _ownTables = {
    'desktop', 'network', 'appearance', 'agents', 'host', 'sessions', 'harnesses', 'overlays', 'profile', 'permissions',
    'store', 'skill_pins', 'loadouts', 'defaults', 'profiles', 'sources', 'shortcuts', 'attachments', 'usage', 'voice',
    'speech', 'connections',
  };

  /// The person's own file [own] merged over [below]: what the app shows and edits.
  factory CanoaConfig.fromLayers(ConfigText own, List<ConfigLayer> below) {
    var lower = <String, dynamic>{};
    for (final layer in below) {
      lower = mergeConfig(lower, layer.data);
    }
    final data = mergeConfig(lower, own.data);
    final readFrom = ConfigSource(format: own.format, marked: own.marked, layers: below, own: own.data);
    // Every other table is the host's or an extension's (`[secrets]`, `[settings.ID]`, `[say.NAME]`, ...): kept as read and written
    // back as found.
    final foreign = {
      for (final MapEntry(:key, :value) in data.entries)
        if (!_ownTables.contains(key)) key: value,
    };
    // A key of these tables the app does not name is not read and not refused: [_unknownKeys] gives it back when the file is written.
    Map<String, dynamic> table(String name) {
      final raw = data[name];
      if (raw == null) return {};
      if (raw is! Map) throw FormatException('${Brand.configFile}: expected a table');
      return Map<String, dynamic>.from(raw);
    }
    final desktop = table('desktop');
    final network = table('network');
    final appearance = table('appearance');
    final sessions = table('sessions');
    final permissions = table('permissions');
    final sources = table('sources');
    final attachments = table('attachments');
    final usage = table('usage');
    final voiceTable = table('voice');
    final outputSpeed = voiceTable['output_speed'];
    if (outputSpeed != null && (outputSpeed is! num || !outputSpeed.isFinite || !(outputSpeed > 0))) {
      throw const FormatException('${Brand.configFile}: voice.output_speed must be a number above 0');
    }
    String? voiceText(String key) {
      final raw = voiceTable[key];
      if (raw == null) return null;
      if (raw is! String || raw.trim().isEmpty) throw FormatException('${Brand.configFile}: voice.$key must be text');
      return raw;
    }
    final vocabulary = voiceTable['vocabulary'] ?? const [];
    if (vocabulary is! List || vocabulary.any((word) => word is! String || word.trim().isEmpty)) {
      throw const FormatException('${Brand.configFile}: voice.vocabulary must be a list of text');
    }
    final acknowledged = voiceTable['acknowledged'] ?? const [];
    if (acknowledged is! List || acknowledged.any((id) => id is! String || id.trim().isEmpty)) {
      throw const FormatException('${Brand.configFile}: voice.acknowledged must be a list of text');
    }
    final voiceMode = voiceTable['mode'] ?? 'toggle';
    if (voiceMode is! String || !{'toggle', 'hold'}.contains(voiceMode)) {
      throw const FormatException('${Brand.configFile}: voice.mode must be toggle or hold');
    }
    int limit(String key, int fallback) {
      final raw = attachments[key] ?? fallback;
      if (raw is! int || raw < 0) throw FormatException('${Brand.configFile}: attachments.$key must be a whole number, 0 or more');
      return raw;
    }
    final warn = usage['warn_permille'] ?? defaultWarnPermille;
    if (warn is! int || warn < 0 || warn > 1000) {
      throw const FormatException('${Brand.configFile}: usage.warn_permille must be a whole number from 0 to 1000');
    }
    final phrases = usage['limit_phrases'];
    if (phrases != null && (phrases is! List || phrases.any((phrase) => phrase is! String))) {
      throw const FormatException('${Brand.configFile}: usage.limit_phrases must be a list of text');
    }
    final hidden = sources['hidden'] ?? const <Object>[];
    if (hidden is! List || hidden.any((name) => name is! String || name.trim().isEmpty)) {
      throw const FormatException('${Brand.configFile}: sources.hidden must be a list of preset names');
    }
    final defaultMode = permissions['default_mode'];
    if (defaultMode != null && (defaultMode is! String || !modeOrder.contains(defaultMode))) {
      throw FormatException('${Brand.configFile}: permissions.default_mode must be one of ${modeOrder.join(', ')}');
    }
    final dropped = permissions['full_access'] ?? const <Object>[];
    if (dropped is! List || dropped.any((name) => name is! String || name.trim().isEmpty)) {
      throw const FormatException('${Brand.configFile}: permissions.full_access must be a list of agent names');
    }
    bool flag(Map<String, dynamic> map, String key, bool fallback) {
      final raw = map[key];
      if (raw == null) return fallback;
      if (raw is! bool) throw FormatException('${Brand.configFile}: invalid boolean $key');
      return raw;
    }
    final timeout = network['connect_timeout'] ?? 10;
    if (timeout is! num || !timeout.isFinite || timeout <= 0) {
      throw const FormatException('${Brand.configFile}: network.connect_timeout must be a number of seconds above 0');
    }
    final lookName = appearance['look'] ?? LookId.canoa.name;
    final look = lookName is String ? LookId.named(lookName) : null;
    if (look == null) {
      throw const FormatException('${Brand.configFile}: appearance.look must be the id of a look family: lower-case letters, digits, hyphens and underscores');
    }
    final brightness = appearance['brightness'] ?? 'dark';
    if (brightness is! String || !{'light', 'dark', 'system'}.contains(brightness)) {
      throw const FormatException('${Brand.configFile}: appearance.brightness must be light, dark or system');
    }
    final densityName = appearance['density'] ?? ChatDensity.normal.name;
    final density = densityName is String ? ChatDensity.named(densityName) : null;
    if (density == null) {
      throw FormatException('${Brand.configFile}: appearance.density must be one of ${ChatDensity.values.map((each) => each.name).join(', ')}');
    }
    final openInName = appearance['open_in'] ?? OpenIn.thisWindow.key;
    final openIn = openInName is String ? OpenIn.named(openInName) : null;
    if (openIn == null) {
      throw FormatException('${Brand.configFile}: appearance.open_in must be one of ${OpenIn.values.map((each) => each.key).join(', ')}');
    }
    final languageName = appearance['language'] ?? '';
    if (languageName is! String) throw FormatException('${Brand.configFile}: appearance.language must be text');
    final rawViewers = appearance['viewers'] ?? const <String, dynamic>{};
    if (rawViewers is! Map || rawViewers.values.any((each) => each is! String)) {
      throw FormatException('${Brand.configFile}: appearance.viewers must be a table of viewer ids by MIME type');
    }
    final viewers = {for (final MapEntry(:key, :value) in rawViewers.entries) '$key': value as String};
    final zoom = appearance['zoom'] ?? 1;
    if (zoom is! num || !zoom.isFinite || zoom < minZoom || zoom > maxZoom) {
      throw FormatException('canoa.cfg: appearance.zoom must be a number from $minZoom to $maxZoom');
    }
    final agents = <AgentCommand>[];
    final agentProfiles = <String, Profile>{};
    final agentTables = <String, Object?>{};
    final rawAgents = data['agents'];
    if (rawAgents != null) {
      if (rawAgents is! Map) throw const FormatException('${Brand.configFile}: agents must be a table of agents');
      for (final entry in rawAgents.entries) {
        final agent = entry.value;
        if (agent is Map && (agent['profile'] != null || agent['mcp'] != null || agent['hooks'] != null)) {
          final table = agent['profile'];
          if (table != null && table is! Map) throw FormatException('${Brand.configFile}: agents.${entry.key}.profile must be a table');
          if (table is Map) agentProfiles['${entry.key}'] = _profile('agents.${entry.key}.profile', table, LayerKind.agent);
          agentTables['${entry.key}'] = {'profile': ?table, 'mcp': ?agent['mcp'], 'hooks': ?agent['hooks']};
          if (!agent.containsKey('program') && agent.keys.every(const {'profile', 'mcp', 'hooks'}.contains)) continue;
        }
        if (agent is! Map || agent['program'] is! String || (agent['program'] as String).isEmpty) {
          throw FormatException('${Brand.configFile}: agents.${entry.key} needs a program');
        }
        for (final key in agent.keys) {
          if (key != 'program' && key != 'args' && key != 'profile' && key != 'mcp' && key != 'hooks') {
            throw FormatException('${Brand.configFile}: agents.${entry.key}: invalid key $key');
          }
        }
        final args = agent['args'] ?? const <Object>[];
        if (args is! List || args.any((arg) => arg is! String)) {
          throw FormatException('${Brand.configFile}: agents.${entry.key}.args must be a list of strings');
        }
        agents.add(AgentCommand(label: '${entry.key}', program: agent['program'] as String, args: args.cast<String>()));
      }
    }
    final host = data['host'];
    if (host != null && host is! Map) throw const FormatException('${Brand.configFile}: host must be a table');
    int count(String key, int fallback) {
      final raw = sessions[key] ?? fallback;
      if (raw is! int || raw < 0) throw FormatException('${Brand.configFile}: sessions.$key must be a whole number, 0 or more');
      return raw;
    }
    final workspace = sessions['workspace'] ?? defaultWorkspace;
    if (workspace is! String) throw const FormatException('${Brand.configFile}: sessions.workspace must be a string');
    if (workspaceProblem(workspace) case final problem?) throw FormatException('${Brand.configFile}: sessions.workspace: $problem');
    final whileWorking = switch (sessions['while_working']) {
      null || 'queue' => WhileWorking.queue,
      'steer' => WhileWorking.steer,
      _ => throw const FormatException('${Brand.configFile}: sessions.while_working must be queue or steer'),
    };
    final queueDelivery = switch (sessions['queue_delivery']) {
      null || 'merge' => QueueDelivery.merge,
      'each' => QueueDelivery.each,
      _ => throw const FormatException('${Brand.configFile}: sessions.queue_delivery must be merge or each'),
    };
    final harnesses = data['harnesses'] ?? const <String, Object?>{};
    if (harnesses is! Map || harnesses.values.any((label) => label is! String || label.trim().isEmpty)) {
      throw const FormatException('${Brand.configFile}: harnesses must map each reported agent name to a label');
    }
    final overlays = <String, OverlaySource>{};
    final rawOverlays = data['overlays'] ?? const <String, Object?>{};
    if (rawOverlays is! Map) throw const FormatException('${Brand.configFile}: overlays must be a table');
    for (final MapEntry(:key, :value) in rawOverlays.entries) {
      if (!isEntryName('$key')) {
        throw const FormatException('${Brand.configFile}: an overlay name has 1 to 64 letters, digits, - or _');
      }
      GitOverlay git(String spec) {
        try {
          return OverlaySource.parseGit(spec);
        } on ProfileException {
          throw FormatException('${Brand.configFile}: overlays.$key is not <repository>@<ref>[#<path>] of a source git may fetch');
        }
      }
      String? pin(Map<dynamic, dynamic> entry) => switch (entry['pin']) {
        null => null,
        final String digest when RegExp(r'^sha256:[0-9a-fA-F]{64}$').hasMatch(digest) => digest,
        _ => throw FormatException('${Brand.configFile}: overlays.$key.pin is sha256: and 64 hexadecimal digits'),
      };
      // The values of the overlay's declared options, which the host reads, and how its knowledge reaches the agent (`kb`); only
      // their shape is checked here.
      Map<dynamic, dynamic> options(Map<dynamic, dynamic> entry) {
        if (entry['options'] != null && entry['options'] is! Map) {
          throw FormatException('${Brand.configFile}: overlays.$key.options is a table of values');
        }
        if (entry['kb'] != null && entry['kb'] != 'index' && entry['kb'] != 'files') {
          throw FormatException('${Brand.configFile}: overlays.$key.kb is "index" or "files"');
        }
        return entry;
      }
      overlays['$key'] = switch (value) {
        final String spec => git(spec),
        {'source': final String spec} when value.keys.every(const {'source', 'pin', 'options', 'kb'}.contains) =>
          git(spec).pinnedTo(pin(options(value))),
        {'dir': final String dir} when dir.isNotEmpty && value.keys.every(const {'dir', 'pin', 'options', 'kb'}.contains) =>
          DirOverlay(dir, pin: pin(options(value))),
        _ => throw FormatException('${Brand.configFile}: overlays.$key is a git source, { source, pin } or { dir, pin }'),
      };
    }
    final rawProfile = data['profile'];
    if (rawProfile != null && rawProfile is! Map) throw const FormatException('${Brand.configFile}: profile must be a table');
    _catalogs(_catalogsIn(data['store'], 'overlays'));
    _connections(data['connections']);
    _skillPins(data['skill_pins']);
    _defaultLoadout(data['defaults'], _loadouts(data['loadouts']));
    _presets(data['profiles']);
    _storeExtras(data['store']);
    _skillSources(_catalogsIn(data['store'], 'skills'));
    _skillsSearch((data['store'] as Map?)?['skills_search']);
    _extensionCatalogs(_catalogsIn(data['store'], 'extensions'));
    if (data['store'] is Map) {
      for (final MapEntry(:key, :value) in (data['store'] as Map).entries) {
        if (value is Map && value.containsKey('catalogs')) parseCatalogSpecs('$key', value['catalogs']);
      }
    }
    _agentMcp(agentTables);
    _agentHooks(agentTables);
    final protectedPaths = permissions['protected'] == null ? null : protectedLinesOf(permissions['protected']);
    if (permissions['protected'] != null && protectedPaths == null) {
      throw const FormatException('${Brand.configFile}: permissions.protected must be text, or a list of text, with a path pattern a line');
    }
    final rawStore = data['store'];
    if (rawStore != null && rawStore is! Map) throw const FormatException('${Brand.configFile}: store must be a table');
    if (rawStore is Map) StoreSettings.parse(rawStore);
    final rawShortcuts = data['shortcuts'];
    if (rawShortcuts != null && rawShortcuts is! Map) throw const FormatException('${Brand.configFile}: shortcuts must be a table');
    final shortcuts = <String, String>{
      if (rawShortcuts is Map)
        for (final MapEntry(:key, :value) in rawShortcuts.entries)
          '$key': value is String && value.trim().isNotEmpty
              ? value.trim()
              : throw FormatException('${Brand.configFile}: shortcuts.$key must be a key chord'),
    };
    return CanoaConfig(
      source: readFrom,
      overlays: overlays,
      profile: rawProfile == null ? const Profile() : _profile('profile', rawProfile as Map, LayerKind.user),
      agentProfiles: agentProfiles,
      tables: {
        if (data['overlays'] != null) 'overlays': rawOverlays,
        'profile': ?rawProfile,
        if (agentTables.isNotEmpty) 'agents': agentTables,
        'connections': ?data['connections'],
        // The host's speech providers: programs and keys the app does not edit here but must write back as it found them.
        'speech': ?data['speech'],
        'skill_pins': ?data['skill_pins'],
        'loadouts': ?data['loadouts'],
        'defaults': ?data['defaults'],
        'profiles': ?data['profiles'],
        'store': ?rawStore,
        ...foreign,
      },
      agents: agents,
      host: host == null ? null : Map<String, dynamic>.from(host as Map),
      look: look, followProvider: flag(appearance, 'follow_provider', false), brightness: brightness, density: density, openIn: openIn, language: languageName, viewers: viewers, zoom: zoom.toDouble(),
      startHidden: flag(desktop, 'start_hidden', false),
      closeToTray: flag(desktop, 'close_to_tray', true),
      minimizeToTray: flag(desktop, 'minimize_to_tray', false),
      localHost: flag(desktop, 'local_host', true),
      startAtLogin: desktop.containsKey('start_at_login') ? flag(desktop, 'start_at_login', false) : null,
      preferWebTransport: flag(network, 'prefer_webtransport', true),
      connectTimeout: timeout,
      defaultMode: defaultMode as String?,
      allowOnceOnly: flag(permissions, 'allow_once_only', false),
      protectedPaths: protectedPaths,
      hiddenSources: {for (final name in hidden) name as String},
      shortcuts: shortcuts,
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
        profile: VoiceProfile(accent: voiceText('accent'), voice: voiceText('voice'), vocabulary: [for (final word in vocabulary) (word as String).trim()]),
        acknowledged: {for (final id in acknowledged) id as String},
        speak: SpeakSettings(
          output: voiceText('output'),
          voice: voiceText('output_voice'),
          model: voiceText('output_model'),
          style: voiceText('output_style'),
          speed: outputSpeed is num ? outputSpeed.toDouble() : null,
          readReplies: flag(voiceTable, 'read_replies', false),
        ),
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
      if (voice.profile.accent != null) 'accent = ${jsonEncode(voice.profile.accent)}',
      if (voice.profile.voice != null) 'voice = ${jsonEncode(voice.profile.voice)}',
      if (voice.profile.vocabulary.isNotEmpty) 'vocabulary = ${jsonEncode(voice.profile.vocabulary)}',
      if (voice.acknowledged.isNotEmpty) 'acknowledged = ${jsonEncode(voice.acknowledged.toList()..sort())}',
      if (voice.speak.output != null) 'output = ${jsonEncode(voice.speak.output)}',
      if (voice.speak.voice != null) 'output_voice = ${jsonEncode(voice.speak.voice)}',
      if (voice.speak.model != null) 'output_model = ${jsonEncode(voice.speak.model)}',
      if (voice.speak.style != null) 'output_style = ${jsonEncode(voice.speak.style)}',
      if (voice.speak.speed != null) 'output_speed = ${voice.speak.speed}',
      if (voice.speak.readReplies) 'read_replies = true',
    ];
    return lines.isEmpty ? '' : '\n[voice]\n${lines.join('\n')}\n';
  }

  /// The top-level values of [tables] that are not tables, which belong before the first table's header.
  Map<String, Object?> get _loose => {for (final MapEntry(:key, :value) in tables.entries) if (value is! Map) key: value};

  Map<String, Object?> get _tabled => {for (final MapEntry(:key, :value) in tables.entries) if (value is Map) key: value};

  String _encodeToml() => '''
# ${Brand.name} deployment settings — Boriken Dev
${_loose.isEmpty ? '' : '${TomlDocument.fromMap(_loose)}\n'}[desktop]
start_hidden = $startHidden
close_to_tray = $closeToTray
minimize_to_tray = $minimizeToTray
local_host = $localHost
${startAtLogin == null ? '' : 'start_at_login = $startAtLogin\n'}
[network]
prefer_webtransport = $preferWebTransport
connect_timeout = $connectTimeout

[appearance]
look = "${look.name}"
follow_provider = $followProvider
brightness = "$brightness"
density = "${density.name}"
${openIn == OpenIn.thisWindow ? '' : 'open_in = "${openIn.key}"\n'}${language.isEmpty ? '' : 'language = ${jsonEncode(language)}\n'}${zoom == 1 ? '' : 'zoom = $zoom\n'}${viewers.isEmpty ? '' : '\n[appearance.viewers]\n${viewers.entries.map((e) => '${jsonEncode(e.key)} = ${jsonEncode(e.value)}\n').join()}'}
[sessions]
keep = ${sessions.keep}
keep_text = ${sessions.keepText}
text_messages = ${sessions.textMessages}
${sessions.workspace == defaultWorkspace ? '' : 'workspace = ${jsonEncode(sessions.workspace)}\n'}${sessions.whileWorking == WhileWorking.queue ? '' : 'while_working = "${sessions.whileWorking.name}"\n'}${sessions.queueDelivery == QueueDelivery.merge ? '' : 'queue_delivery = "${sessions.queueDelivery.name}"\n'}
[attachments]
paste_as_file = $pasteAsFile
embed_limit = $embedLimit
${_voiceToml()}${warnPermille == defaultWarnPermille && limitPhrases == null ? '' : '\n[usage]\n${warnPermille == defaultWarnPermille ? '' : 'warn_permille = $warnPermille\n'}${limitPhrases == null ? '' : 'limit_phrases = ${jsonEncode(limitPhrases)}\n'}'}${defaultMode == null && !allowOnceOnly && protectedPaths == null ? '' : '\n[permissions]\n${defaultMode == null ? '' : 'default_mode = ${jsonEncode(defaultMode)}\n'}${allowOnceOnly ? 'allow_once_only = true\n' : ''}${protectedPaths == null ? '' : 'protected = [\n${protectedPaths!.map((line) => '  ${jsonEncode(line)},\n').join()}]\n'}'}${hiddenSources.isEmpty ? '' : '\n[sources]\nhidden = ${jsonEncode(hiddenSources.toList())}\n'}${shortcuts.isEmpty ? '' : '\n[shortcuts]\n${shortcuts.entries.map((e) => '${jsonEncode(e.key)} = ${jsonEncode(e.value)}\n').join()}'}${sessions.harnesses.isEmpty ? '' : '\n[harnesses]\n${sessions.harnesses.entries.map((name) => '${jsonEncode(name.key)} = ${jsonEncode(name.value)}\n').join()}'}${agents.map((agent) => '''
[agents.${jsonEncode(agent.label)}]
program = ${jsonEncode(agent.program)}
args = ${jsonEncode(agent.args)}
''').join()}${_tabled.isEmpty ? '' : '\n${TomlDocument.fromMap(_tabled)}'}${host == null ? '' : '\n${TomlDocument.fromMap({'host': host})}'}''';

  /// canoa.cfg as the person's own file is written: in the format and with the first line it was read with, holding the
  /// keys the person changed or already had there and none that a lower layer gives the same.
  String encode() {
    final toml = _encodeToml();
    if (source.layers.isEmpty && source.format == ConfigFormat.toml) {
      final text = _unknownKeys.isEmpty ? toml : '${toml.split('\n').first}\n${TomlDocument.fromMap(_modelDocument())}';
      return source.marked ? '#!toml\n$text' : text;
    }
    return encodeConfig(_ownDocument(), source.format, marked: source.marked);
  }

  static final _ownDocuments = Expando<Map<String, dynamic>>('own document');

  /// The tables of the person's own file as this configuration writes it: the keys the person changed or already had there, and none
  /// that a lower layer gives the same.
  Map<String, dynamic> get documentToWrite => _ownDocument();

  Map<String, dynamic> _ownDocument() => _ownDocuments[this] ??=
      ownDocument(_modelDocument(), _lowerModel, source.own, _defaults);

  static final _lowerModels = Expando<Map<String, dynamic>>('lower layers as the model writes them');

  /// What the layers below the person's file hold, written the way this app writes a configuration, so that an entry of a
  /// lower layer that the model fills in (an agent with no `args`) still reads as the same entry and is not copied into the
  /// person's file.
  Map<String, dynamic> get _lowerModel => source.layers.isEmpty
      ? const {}
      : _lowerModels[source] ??=
            CanoaConfig.fromLayers((format: ConfigFormat.toml, marked: false, data: const <String, dynamic>{}), source.layers)._modelDocument();

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
  CanoaConfig resetKey(List<String> path) {
    final reset = CanoaConfig.fromLayers(
      (format: source.format, marked: source.marked, data: withoutKey(_ownDocument(), path)),
      source.layers,
    );
    final why = unreadable;
    if (why == null) return reset;
    return reset.withHost(reset.host, source: ConfigSource(
      format: reset.source.format,
      marked: reset.source.marked,
      layers: reset.source.layers,
      own: reset.source.own,
      unreadable: why,
    ));
  }

  /// A profile table of canoa.cfg, its refusal named by [where] and never quoting a value.
  static Profile _profile(String where, Map<dynamic, dynamic> table, LayerKind kind) {
    try {
      return Profile.fromMap(table, kind);
    } on ProfileException catch (error) {
      throw FormatException('${Brand.configFile}: $where: $error');
    }
  }
}

