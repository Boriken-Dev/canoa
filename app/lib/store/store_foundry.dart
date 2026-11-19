import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import '../selectable.dart';
import 'entry_details.dart';
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
  const FoundryModel({
    required this.id,
    this.alias,
    this.task,
    this.device,
    this.sizeMb,
    this.description = '',
    this.cached = false,
    this.loaded = false,
    this.contextLength,
    this.maxLength,
    this.chosenWindow,
  });

  factory FoundryModel.fromJson(Map<Object?, Object?> json) => FoundryModel(
    id: json['id'] as String,
    alias: json['alias'] as String?,
    task: json['task'] as String?,
    device: json['device'] as String?,
    sizeMb: json['size_mb'] is num ? (json['size_mb'] as num).toInt() : null,
    description: json['description'] is String ? json['description'] as String : '',
    cached: json['cached'] == true,
    loaded: json['loaded'] == true,
    contextLength: _tokens(json['window'], 'context_length'),
    maxLength: _tokens(json['window'], 'max_length'),
    chosenWindow: _tokens(json['window'], 'chosen'),
  );

  static int? _tokens(Object? window, String key) => window is Map && window[key] is num ? (window[key] as num).toInt() : null;

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

  /// The window the model's own files say, once it is downloaded: the most it was built for (`context_length`) and the most a
  /// prompt and its answer take together as it is cached (`max_length`).
  final int? contextLength, maxLength;

  /// The window the person set for it in canoa.cfg, or null when they set none.
  final int? chosenWindow;

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

  /// Sets the window [model] is run with to [tokens], or takes the setting away with null. Returns the host's note when it lowered
  /// the number to the model's own context length; throws what the host refuses with. The list is read again after.
  Future<String?> setWindow(String model, int? tokens) async {
    final client = await _control();
    try {
      final answer = await client.call('foundry/window', {'model': model, 'window': tokens}, null, const Duration(seconds: 30));
      return answer is Map && answer['note'] is String ? answer['note'] as String : null;
    } finally {
      await refresh();
    }
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
          details: (context, config) => FoundryModelPage(models: models, id: model.id),
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

/// One model's page: what Foundry Local says of it, the window its own files give it and the window the person may set instead, and,
/// for a Whisper model, "Use as speech provider", which makes the host's provider that sends dictation to the model.
/// The setting is kept in canoa.cfg by the host and is used when an account runs the model with the sdk engine, which makes
/// Canoa's own folder for it; Foundry Local's cache and the service's run of the model are not changed.
class FoundryModelPage extends StatefulWidget {
  const FoundryModelPage({super.key, required this.models, required this.id});

  final FoundryModels models;
  final String id;

  @override
  State<FoundryModelPage> createState() => _FoundryModelPageState();
}

class _FoundryModelPageState extends State<FoundryModelPage> {
  final _field = TextEditingController();
  int? _shown;
  String? _said;
  String? _failed;
  bool _busy = false;
  String? _made;
  String? _speechProblem;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  FoundryModel? get _model {
    for (final each in widget.models.models) {
      if (each.id == widget.id) return each;
    }
    return null;
  }

  Future<void> _set(int? tokens) async {
    setState(() {
      _busy = true;
      _said = null;
      _failed = null;
    });
    try {
      final note = await widget.models.setWindow(widget.id, tokens);
      if (!mounted) return;
      setState(() => _said = note ?? (tokens == null ? 'The setting is removed: the model runs with its own window.' : 'Kept: $tokens tokens.'));
    } on Object catch (error) {
      if (mounted) setState(() => _failed = hostWords(error));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _use() async {
    setState(() {
      _busy = true;
      _speechProblem = null;
    });
    try {
      final name = await widget.models.useForSpeech(widget.id);
      if (mounted) setState(() => _made = name);
    } on Object catch (error) {
      if (mounted) setState(() => _speechProblem = hostWords(error));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.models,
    builder: (context, _) {
      final model = _model;
      if (model == null) {
        return Scaffold(
          appBar: AppBar(title: const Text('Model')),
          body: const Padding(padding: EdgeInsets.all(16), child: Text('Foundry Local does not list this model now.', key: Key('foundry-model-gone'))),
        );
      }
      final chosen = model.chosenWindow;
      if (_shown != chosen) {
        _shown = chosen;
        _field.text = chosen == null ? '' : '$chosen';
      }
      String tokens(int? value) => value == null ? 'not known' : '$value tokens';
      final own = model.contextLength != null || model.maxLength != null;
      return EntryDetails(
        key: Key('foundry-model-${model.id}'),
        title: model.id,
        leading: const Icon(Icons.memory, size: 56),
        from: 'Foundry Local',
        summary: model.summary,
        state: model.loaded ? 'Loaded' : model.cached ? 'Downloaded' : null,
        overview: {
          'Alias': ?model.alias,
          'Task': ?model.task,
          'Device': ?model.device,
        },
        contents: false,
        sections: [
          if (model.speech)
            DetailSection(key: const Key('foundry-speech'), title: 'Speech to text', children: [
              const Padding(
                padding: EdgeInsets.only(top: 4),
                child: Text(
                  'Dictation can send its audio to this model through the Foundry Local service on the host: the audio is written to a temporary file there, '
                  'handed to the service and removed afterwards, and nothing is sent to another service. Canoa never loads a model for you, so load it '
                  'with + on the list first.',
                ),
              ),
              if (!model.loaded)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    'It is not loaded now.',
                    key: const Key('foundry-speech-not-loaded'),
                    style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant),
                  ),
                ),
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: FilledButton(key: const Key('foundry-use-speech'), onPressed: _busy ? null : () => unawaited(_use()), child: const Text('Use as speech provider')),
                ),
              ),
              if (_made case final made?)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text('Added as the provider "$made". Choose it in Settings, Voice.', key: const Key('foundry-speech-done')),
                ),
              if (_speechProblem case final problem?)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(problem, key: const Key('foundry-speech-failed'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ),
            ]),
          DetailSection(key: const Key('foundry-window'), title: 'Window', children: [
            if (!own)
              Text(
                model.cached ? 'The model\'s own files did not say.' : 'Not downloaded: its own window is read once it is.',
                key: const Key('foundry-window-unknown'),
              )
            else ...[
              DetailField(label: 'Most the model takes (context length)', value: tokens(model.contextLength)),
              DetailField(label: 'Prompt and answer together, as cached (max length)', value: tokens(model.maxLength)),
            ],
            DetailField(label: 'Your setting', value: chosen == null ? 'none: the model runs with its own' : tokens(chosen)),
            const Padding(
              padding: EdgeInsets.only(top: 8),
              child: Text(
                'A longer window lets a longer prompt through and keeps more memory for the text so far; a shorter one keeps less. It cannot go above the model\'s context length.',
                key: Key('foundry-window-about'),
              ),
            ),
            const Padding(
              padding: EdgeInsets.only(top: 4),
              child: Text(
                'It applies when an account runs this model with the sdk engine: Canoa then makes a folder of its own for the model, with links to Foundry Local\'s files and its own configuration, and runs the model in a models folder of its own as well, so Foundry Local\'s cache and its catalog file are not changed (only when Canoa\'s folder is on another drive than Foundry Local\'s cache does it use Foundry Local\'s folder and put its catalog file back afterwards). It takes effect the next time the account\'s gateway starts. The Foundry Local service engine runs the model as it is cached and does not use this setting.',
                key: Key('foundry-window-engine'),
              ),
            ),
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Row(children: [
                SizedBox(
                  width: 160,
                  child: TextField(
                    key: const Key('foundry-window-field'),
                    controller: _field,
                    keyboardType: TextInputType.number,
                    decoration: const InputDecoration(labelText: 'Window in tokens', isDense: true),
                  ),
                ),
                const SizedBox(width: 8),
                FilledButton(
                  key: const Key('foundry-window-set'),
                  onPressed: _busy
                      ? null
                      : () {
                          final tokens = int.tryParse(_field.text.trim());
                          if (tokens == null || tokens < 1) {
                            setState(() {
                              _said = null;
                              _failed = 'A window is a whole number of tokens, at least 1.';
                            });
                            return;
                          }
                          unawaited(_set(tokens));
                        },
                  child: const Text('Set'),
                ),
                const SizedBox(width: 8),
                OutlinedButton(
                  key: const Key('foundry-window-clear'),
                  onPressed: _busy || chosen == null ? null : () => unawaited(_set(null)),
                  child: const Text('Remove setting'),
                ),
              ]),
            ),
            if (_said != null) Padding(padding: const EdgeInsets.only(top: 4), child: Text(_said!, key: const Key('foundry-window-said'))),
            if (_failed != null)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text(_failed!, key: const Key('foundry-window-failed'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
          ]),
        ],
      );
    },
  );
}
