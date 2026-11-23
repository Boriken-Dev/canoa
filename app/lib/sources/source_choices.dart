import 'dart:convert';

import '../chat/agent_events.dart';
import '../chat/cache_lifetime.dart';
import '../chat/modes.dart' show choiceLabel;
import '../chat/session_options.dart' show OptionRole;
import 'sources.dart';

/// One model a source's provider offers before a session starts, as `canoa options` prints it.
final class ModelChoice {
  const ModelChoice({
    required this.id,
    required this.name,
    this.description,
    this.resolved,
    this.version,
    this.efforts = const [],
    this.defaultEffort,
    this.isDefault = false,
    this.accounts = const [],
  });

  /// What a session is told to use.
  final String id;

  /// What a person is shown, with the version where the provider knows it.
  final String name;
  final String? description;

  /// The model an alias or the default resolves to, as the harness names it.
  final String? resolved;

  /// The model and its version for people, as the provider words it.
  final String? version;

  /// The efforts this model takes, in the provider's order.
  final List<String> efforts;
  final String? defaultEffort;

  /// Whether the harness uses it when none is named.
  final bool isDefault;

  /// For the models of a pool: the accounts of the pool that list it. Empty for one account's list.
  final List<String> accounts;

  static ModelChoice? _parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    String? text(String key) => json[key] is String ? json[key] as String : null;
    final id = json['id'] as String;
    return ModelChoice(
      id: id,
      name: text('name') ?? id,
      description: text('description'),
      resolved: text('resolved'),
      version: text('version'),
      efforts: [if (json['efforts'] is List) for (final effort in json['efforts'] as List) if (effort is String) effort],
      defaultEffort: text('default_effort'),
      isDefault: json['is_default'] == true,
      accounts: [if (json['accounts'] is List) for (final each in json['accounts'] as List) if (each is String) each],
    );
  }
}

/// An account of a pool, and whether it listed its models; a member that could not says why.
final class PoolListing {
  const PoolListing({required this.name, this.listed = true, this.why});

  final String name;
  final bool listed;
  final String? why;
}

/// Why a source's provider could not list its models when it could not be started at all. A provider that lists only in its
/// session has none.
enum UnknownKind {
  /// The program is not on the machine the host runs on.
  notInstalled,

  /// The program is there and could not be asked; [SourceChoices.unknownBecause] has its words.
  failed;

  static UnknownKind? named(Object? name) => switch (name) {
    'not_installed' => notInstalled,
    'failed' => failed,
    _ => null,
  };
}

/// What a source's provider lists before a session starts: its models and each one's efforts. A source whose provider cannot
/// say before it starts is [known] false, and the composer says its choices come after it starts, or, when the provider could
/// not be started, why.
final class SourceChoices {
  const SourceChoices({
    required this.known,
    this.unknownBecause,
    this.unknownKind,
    this.signedOut = false,
    this.models = const [],
    this.modes = const [],
    this.defaultMode,
    this.programVersion,
    this.cacheLifetime,
    this.cacheLifetimeSetting,
    this.cacheChoices = const [],
    this.members = const [],
    this._fetchedAt,
  });

  final bool known;
  final String? unknownBecause;

  /// Why the provider could not be asked at all, when it could not be started.
  final UnknownKind? unknownKind;

  /// Whether the provider listed its models and says nobody is signed in on the machine's own login. Says nothing of an
  /// account the host applies.
  final bool signedOut;
  final List<ModelChoice> models;

  /// The modes a program that lists them in its session listed, as (id, name), and the one it started in.
  final List<(String, String)> modes;
  final String? defaultMode;

  /// The first line of the program's `--version`, when it answered.
  final String? programVersion;

  /// What the provider says it does with a session's cache lifetime, in its own sentence (`ignored: ...` where it has no
  /// control for it); null when it did not say, and then the chat offers no choice.
  final String? cacheLifetime;

  /// The lifetime canoa.cfg's entry sets for its sessions, when it sets one.
  final CacheLifetime? cacheLifetimeSetting;

  /// The lifetimes beyond the agent's default that the provider honours; empty where it ignores the choice. Only these are offered.
  final List<CacheLifetime> cacheChoices;

  /// For the choices of a pool: each of its accounts and whether it listed its models. Empty for one account's.
  final List<PoolListing> members;

  /// Whether the provider honours [lifetime]: the agent's default always, the rest only as [cacheChoices] lists them.
  bool offersCache(CacheLifetime lifetime) =>
      lifetime == CacheLifetime.harness || cacheChoices.contains(lifetime);

  /// What [lifetime] is called in a menu and on the pill.
  String cacheLabel(CacheLifetime lifetime) => lifetime.choiceLabel;
  final DateTime? _fetchedAt;

  /// When this was asked; the epoch for choices that were not.
  DateTime get fetchedAt => _fetchedAt ?? DateTime.fromMillisecondsSinceEpoch(0, isUtc: true);

  SourceChoices at(DateTime when) =>
      SourceChoices(
        known: known,
        unknownBecause: unknownBecause,
        unknownKind: unknownKind,
        signedOut: signedOut,
        models: models,
        modes: modes,
        defaultMode: defaultMode,
        programVersion: programVersion,
        cacheLifetime: cacheLifetime,
        cacheLifetimeSetting: cacheLifetimeSetting,
        cacheChoices: cacheChoices,
        members: members,
        fetchedAt: when,
      );

  /// The object `canoa options --format json` prints, in [text]; null when it holds none.
  static SourceChoices? parse(String text) {
    for (final line in const LineSplitter().convert(text).reversed) {
      try {
        final value = jsonDecode(line);
        if (value is! Map || value['known'] is! bool) continue;
        return SourceChoices(
          known: value['known'] as bool,
          unknownBecause: value['unknown_because'] is String ? value['unknown_because'] as String : null,
          unknownKind: UnknownKind.named(value['unknown_kind']),
          signedOut: value['signed_out'] == true,
          models: [if (value['models'] is List) for (final model in value['models'] as List) ?ModelChoice._parse(model)],
          modes: [
            if (value['modes'] case final List<dynamic> modes)
              for (final mode in modes)
                if (mode is Map && mode['id'] is String) (mode['id'] as String, mode['name'] is String ? mode['name'] as String : mode['id'] as String),
          ],
          defaultMode: value['default_mode'] is String ? value['default_mode'] as String : null,
          programVersion: value['program_version'] is String ? value['program_version'] as String : null,
          cacheLifetime: value['cache_lifetime'] is String ? value['cache_lifetime'] as String : null,
          cacheLifetimeSetting: CacheLifetime.named(value['cache_lifetime_setting']),
          cacheChoices: [if (value['cache_choices'] is List) for (final choice in value['cache_choices'] as List) if (choice is Map) ?CacheLifetime.named(choice['id'])],
          members: [
            if (value['accounts'] is List)
              for (final each in value['accounts'] as List)
                if (each is Map && each['name'] is String)
                  PoolListing(name: each['name'] as String, listed: each['listed'] != false, why: each['why'] is String ? each['why'] as String : null),
          ],
        );
      } on FormatException {
        continue;
      }
    }
    return null;
  }

  /// The model the harness uses when none is named, else the first listed.
  ModelChoice? get defaultModel => models.where((model) => model.isDefault).firstOrNull ?? models.firstOrNull;

  /// The model option and the effort option for [model] (the one a person chose, else the default model's): each model's own
  /// efforts, so the list follows the model. Nothing when the provider listed no model.
  List<ConfigOption> options({String? model}) {
    if (!known || (models.isEmpty && modes.isEmpty)) return const [];
    final chosen = models.where((each) => each.id == model).firstOrNull ?? defaultModel;
    final efforts = chosen?.efforts ?? const <String>[];
    // The provider's own Default entry is what a draft's Default stands for, so it is that and not a second choice.
    final standsFor = models.where((each) => each.id == 'default').firstOrNull;
    final used = standsFor ?? defaultModel;
    final shortName = used == null
        ? null
        : (used.version ?? used.name.replaceFirstMapped(RegExp(r'^Default \((.*)\)$'), (match) => match[1]!));
    final defaultEffort = chosen?.defaultEffort;
    final effortName = defaultEffort == null ? null : choiceLabel(OptionRole.effort, Choice(defaultEffort, defaultEffort));
    return [
      if (modes.isNotEmpty)
        ConfigOption(
          id: 'mode', label: 'Mode', current: '', category: 'mode',
          choices: [for (final (id, name) in modes) Choice(id, name)],
        ),
      if (models.isNotEmpty) ConfigOption(
        id: 'model', label: 'Model', current: '', category: 'model', defaultLabel: standsFor?.name, defaultName: shortName,
        choices: [for (final each in models) if (each != standsFor) Choice(each.id, each.name)],
      ),
      if (efforts.isNotEmpty)
        ConfigOption(
          id: 'effort', label: 'Effort', current: '', category: 'thought_level',
          defaultLabel: effortName == null ? null : 'Default ($effortName)', defaultName: effortName,
          choices: [for (final each in efforts) Choice(each, each)],
        ),
    ];
  }
}

/// Asks the host for a source's choices; null for a source whose provider cannot say before it starts, or when the host could
/// not be asked.
typedef ChoicesLoader = Future<SourceChoices?> Function(AgentSource source);

/// Asks the host for [source]'s choices as its account [account].
typedef AccountChoicesLoader = Future<SourceChoices?> Function(AgentSource source, String account);

/// How long the choices asked of a source are kept before a new draft of it asks again.
const Duration choicesLife = Duration(hours: 1);
