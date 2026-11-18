import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import '../selectable.dart';
import 'programs_view.dart' show hostWords;
import 'store_extension.dart';

// Foundry Local's models in the Store: the catalog its command line lists, which are downloaded and which are in memory, and the
// + that downloads a model and loads it once the person has read what that takes. The host runs the command line and says what
// it refused in its own words; this holds only what the host answered. Foundry Local itself is a row of Runtimes.

/// The id and plural title of the Foundry models type in the Store's index.
const String foundryTypeId = 'foundrymodels';
const String foundryTypeLabel = 'Foundry models';

/// The catalog name its rows come from.
const String foundryCatalog = 'Foundry Local';

/// How long a download may take before the app stops waiting; the host ends it sooner than this.
const Duration modelWait = Duration(hours: 4, minutes: 10);

/// One model, as the host listed it.
final class FoundryModel {
  const FoundryModel({required this.id, this.alias, this.task, this.device, this.sizeMb, this.description = '', this.cached = false, this.loaded = false});

  factory FoundryModel.fromJson(Map<Object?, Object?> json) => FoundryModel(
    id: json['id'] as String,
    alias: json['alias'] as String?,
    task: json['task'] as String?,
    device: json['device'] as String?,
    sizeMb: json['size_mb'] is num ? (json['size_mb'] as num).toInt() : null,
    description: json['description'] is String ? json['description'] as String : '',
    cached: json['cached'] == true,
    loaded: json['loaded'] == true,
  );

  /// The name the command line takes.
  final String id;
  final String? alias, task, device;
  final int? sizeMb;

  /// What the host says of it in a line: the task, the hardware and the size.
  final String description;

  /// Whether it is downloaded, and whether it is in memory.
  final bool cached, loaded;

  /// Whether Canoa can use it to turn speech into text: Foundry Local transcribes with Whisper models, which take a file.
  bool get speech => '$id ${alias ?? ''}'.toLowerCase().contains('whisper');

  /// What the row says of the model: where it stands, then what it is.
  String get summary => [loaded ? 'Loaded' : cached ? 'Downloaded' : 'Not downloaded', if (description.isNotEmpty) description].join(' · ');
}

/// The models the host lists and what the person asks it to do with one.
class FoundryModels extends ChangeNotifier {
  FoundryModels({required Future<ControlClient> Function() control, required Future<ControlClient> Function() open})
    : _control = control,
      _open = open;

  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;
  Future<void>? _asking;
  bool _installed = false;
  List<FoundryModel> _models = const [];
  String? _problem;

  /// Whether the host found Foundry Local, which is the Runtimes row's to install.
  bool get installed => _installed;
  List<FoundryModel> get models => _models;

  /// What the host said when it could not list them, in its words.
  String? get problem => _problem;

  /// Asks the host for the models, once; [refresh] asks again.
  Future<void> ensure() => _asking ??= refresh();

  Future<void> refresh() async {
    try {
      final client = await _control();
      final answer = await client.call('foundry/models', const {}, null, const Duration(minutes: 3));
      final map = answer is Map ? answer : const {};
      _installed = map['installed'] == true;
      _models = [
        for (final each in map['models'] is List ? map['models'] as List : const [])
          if (each is Map && each['id'] is String) FoundryModel.fromJson(each),
      ];
      _problem = null;
    } on Object catch (error) {
      _installed = false;
      _models = const [];
      _problem = hostWords(error);
      _asking = null;
    }
    notifyListeners();
  }

  /// Makes [model] a speech-to-text provider on the host, which calls Foundry Local's service with it. The model is not loaded
  /// here: loading one is the person's choice. Returns the provider's name; throws what the host refuses with.
  Future<String> useForSpeech(String model) async {
    final client = await _control();
    final answer = await client.call('speech/enable', {'offer': 'foundry', 'model': model}, null, const Duration(seconds: 30));
    return answer is Map && answer['name'] is String ? answer['name'] as String : 'foundry';
  }

  /// Asks the host to `download`, `load` or `unload` [model], on a link of its own since a download takes long; throws what it
  /// refuses with. The list is read again after, whatever came of it.
  Future<void> run(String model, String action) async {
    final client = await _open();
    try {
      await client.call('foundry/model', {'model': model, 'action': action}, null, modelWait);
    } finally {
      unawaited(client.close());
      await refresh();
    }
  }
}

/// The Foundry models type, listed after Runtimes.
ExtensionStoreType foundryModelsType(FoundryModels models) => ExtensionStoreType(
  id: foundryTypeId,
  label: foundryTypeLabel,
  icon: Icons.memory,
  changes: models,
  load: models.refresh,
  items: () {
    if (!models.installed) {
      return [
        ExtensionStoreItem(
          catalog: foundryCatalog,
          name: 'Foundry Local',
          by: 'Microsoft',
          summary: models.problem ?? 'Not installed: install it from Runtimes, and its models are listed here.',
        ),
      ];
    }
    return [
      for (final model in models.models)
        ExtensionStoreItem(
          catalog: foundryCatalog,
          name: model.id,
          by: model.alias ?? 'Microsoft',
          summary: model.summary,
          tags: [?model.task, ?model.device],
          installed: model.loaded,
          install: (context) => _choose(context, models, model),
          details: model.speech ? (context, config) => FoundrySpeechPage(models: models, model: model) : null,
        ),
    ];
  },
);

Future<bool> _confirm(BuildContext context, {required String title, required String text, required String action}) async {
  final answer = await showSelectableDialog<bool>(
    context: context,
    builder: (dialog) => AlertDialog(
      key: const Key('foundry-confirm'),
      title: Text(title),
      content: Text(text),
      actions: [
        TextButton(onPressed: () => Navigator.pop(dialog, false), child: const Text('Cancel')),
        FilledButton(key: const Key('foundry-confirm-yes'), onPressed: () => Navigator.pop(dialog, true), child: Text(action)),
      ],
    ),
  );
  return answer == true;
}

Future<void> _failed(BuildContext context, String title, Object error) => showSelectableDialog<void>(
  context: context,
  builder: (dialog) => AlertDialog(
    key: const Key('foundry-failed'),
    title: Text(title),
    content: SelectableText(hostWords(error)),
    actions: [TextButton(onPressed: () => Navigator.pop(dialog), child: const Text('Close'))],
  ),
);

/// What + does for [model]: unload it when it is in memory, load it when it is downloaded and download then load it when it is
/// not, each after the person has read what it takes.
Future<void> _choose(BuildContext context, FoundryModels models, FoundryModel model) async {
  final mb = model.sizeMb;
  final size = mb == null ? '' : mb >= 1024 ? ' (${(mb / 1024).toStringAsFixed(1)} GB)' : ' ($mb MB)';
  final memory = 'A loaded model stays in memory until Foundry Local\'s service is stopped (`foundry server stop`), and takes about as much as its download.';
  if (model.loaded) {
    if (!await _confirm(context, title: 'Unload ${model.id}', text: 'Foundry Local takes the model out of memory. The service may keep what it reserved until it is stopped.', action: 'Unload')) return;
    if (!context.mounted) return;
    try {
      await models.run(model.id, 'unload');
    } on Object catch (error) {
      if (context.mounted) await _failed(context, 'Unload ${model.id}', error);
    }
    return;
  }
  final text = model.cached
      ? 'Foundry Local loads ${model.id}. $memory'
      : 'Foundry Local downloads ${model.id}$size into its cache, which can take long, and then loads it. $memory';
  if (!await _confirm(context, title: model.cached ? 'Load ${model.id}' : 'Download and load ${model.id}', text: text, action: model.cached ? 'Load' : 'Download and load')) return;
  if (!context.mounted) return;
  try {
    if (!model.cached) await models.run(model.id, 'download');
    await models.run(model.id, 'load');
  } on Object catch (error) {
    if (context.mounted) await _failed(context, 'Load ${model.id}', error);
  }
}

/// The page of a Whisper model: what Foundry Local says of it and "Use as speech provider", which makes the host's provider that
/// sends dictation to the model through Foundry Local's service. The audio stays on the host's computer.
class FoundrySpeechPage extends StatefulWidget {
  const FoundrySpeechPage({super.key, required this.models, required this.model});

  final FoundryModels models;
  final FoundryModel model;

  @override
  State<FoundrySpeechPage> createState() => _FoundrySpeechPageState();
}

class _FoundrySpeechPageState extends State<FoundrySpeechPage> {
  String? _made;
  String? _problem;
  bool _busy = false;

  Future<void> _use() async {
    setState(() {
      _busy = true;
      _problem = null;
    });
    try {
      final name = await widget.models.useForSpeech(widget.model.id);
      if (mounted) setState(() => _made = name);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = hostWords(error));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(title: Text(model.id)),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          Text(model.summary, key: const Key('foundry-speech-summary')),
          const SizedBox(height: 12),
          const Text(
            'Dictation can send its audio to this model through the Foundry Local service on the host: the audio is written to a temporary file there, '
            'handed to the service and removed afterwards, and nothing is sent to another service. Canoa never loads a model for you, so load it '
            'with + on the list first.',
          ),
          if (!model.loaded)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text('It is not loaded now.', key: const Key('foundry-speech-not-loaded'), style: TextStyle(color: scheme.onSurfaceVariant)),
            ),
          const SizedBox(height: 16),
          Align(
            alignment: Alignment.centerLeft,
            child: FilledButton(key: const Key('foundry-use-speech'), onPressed: _busy ? null : () => unawaited(_use()), child: const Text('Use as speech provider')),
          ),
          if (_made case final made?)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text('Added as the provider "$made". Choose it in Settings, Voice.', key: const Key('foundry-speech-done')),
            ),
          if (_problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(problem, key: const Key('foundry-speech-failed'), style: TextStyle(color: scheme.error)),
            ),
        ],
      ),
    );
  }
}
