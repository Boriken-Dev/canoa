import 'dart:convert';

import '../chat/agent_events.dart';
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
    );
  }
}

/// What a source's provider lists before a session starts: its models and each one's efforts. A source whose provider cannot
/// say before it starts is [known] false, and the composer says its choices come after it starts.
final class SourceChoices {
  const SourceChoices({required this.known, this.unknownBecause, this.models = const [], this.programVersion, DateTime? fetchedAt})
      : _fetchedAt = fetchedAt;

  final bool known;
  final String? unknownBecause;
  final List<ModelChoice> models;

  /// The first line of the program's `--version`, when it answered.
  final String? programVersion;
  final DateTime? _fetchedAt;

  /// When this was asked; the epoch for choices that were not.
  DateTime get fetchedAt => _fetchedAt ?? DateTime.fromMillisecondsSinceEpoch(0, isUtc: true);

  SourceChoices at(DateTime when) =>
      SourceChoices(known: known, unknownBecause: unknownBecause, models: models, programVersion: programVersion, fetchedAt: when);

  /// The object `canoa options --format json` prints, in [text]; null when it holds none.
  static SourceChoices? parse(String text) {
    for (final line in const LineSplitter().convert(text).reversed) {
      try {
        final value = jsonDecode(line);
        if (value is! Map || value['known'] is! bool) continue;
        return SourceChoices(
          known: value['known'] as bool,
          unknownBecause: value['unknown_because'] is String ? value['unknown_because'] as String : null,
          models: [if (value['models'] is List) for (final model in value['models'] as List) ?ModelChoice._parse(model)],
          programVersion: value['program_version'] is String ? value['program_version'] as String : null,
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
    if (!known || models.isEmpty) return const [];
    final chosen = models.where((each) => each.id == model).firstOrNull ?? defaultModel;
    final efforts = chosen?.efforts ?? const <String>[];
    // The provider's own Default entry is what a draft's Default stands for, so it is that and not a second choice.
    final standsFor = models.where((each) => each.id == 'default').firstOrNull;
    return [
      ConfigOption(
        id: 'model', label: 'Model', current: '', category: 'model', defaultLabel: standsFor?.name,
        choices: [for (final each in models) if (each != standsFor) Choice(each.id, each.name)],
      ),
      if (efforts.isNotEmpty)
        ConfigOption(
          id: 'effort', label: 'Effort', current: '', category: 'thought_level',
          choices: [for (final each in efforts) Choice(each, each)],
        ),
    ];
  }
}

/// Asks the host for a source's choices; null for a source whose provider cannot say before it starts, or when the host could
/// not be asked.
typedef ChoicesLoader = Future<SourceChoices?> Function(AgentSource source);

/// How long the choices asked of a source are kept before a new draft of it asks again.
const Duration choicesLife = Duration(hours: 1);
