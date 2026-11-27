import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart';
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import '../selectable.dart';
import 'entry_details.dart';
import 'programs.dart';
import 'programs_view.dart';
import 'store_banner.dart' show StoreEngine;
import 'store_extension.dart';

// The models of a Store type, as its library lists them: what the host's `store/models` answers (rows with a line, tags, what + does,
// the page's sections and the form of a model's settings), moved by `store/model` and kept by `store/model_setting`. The host runs
// the type's program and says what it refused in its own words; this draws only what the host answered and names no model.

/// How long a download may take before the app stops waiting; the host ends it sooner than this.
const Duration modelWait = Duration(hours: 4, minutes: 10);

/// The id and plural title of the page that lists the models of every Store type, in the Store's index.
const String modelsTypeId = 'models';
const String modelsTypeLabel = 'Models';

Map<String, Object?> _map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};

List<T> _list<T>(Object? value, T? Function(Map<Object?, Object?> each) read) => [
  for (final each in value is List ? value : const []) if (each is Map) ?read(each),
];

String? _text(Object? value) => value is String && value.isNotEmpty ? value : null;

/// One thing a page states of a model.
final class ModelFact {
  const ModelFact(this.label, this.value);

  final String label, value;
}

/// One section of a model's page.
final class ModelSection {
  const ModelSection({required this.title, this.facts = const [], this.paragraphs = const [], this.settings = false});

  final String title;
  final List<ModelFact> facts;
  final List<String> paragraphs;

  /// Whether the form of the model's settings is drawn in it.
  final bool settings;
}

/// One form an engine's file is written in, which the install dialog offers.
final class ModelVariant {
  const ModelVariant({required this.id, required this.label, this.detail});

  final String id, label;

  /// What the dialog says beside it.
  final String? detail;
}

/// One way a model runs: a pill on its tile, installed once what it reads is there and available until then.
final class ModelEngine {
  const ModelEngine({required this.id, required this.label, required this.installed, this.variants = const []});

  final String id, label;
  final bool installed;

  /// The forms its file is written in at install, the first the default; empty when the install offers no choice.
  final List<ModelVariant> variants;
}

/// What + does for a model once the person has read what it takes.
final class ModelAction {
  const ModelAction({required this.title, required this.confirm, required this.label, required this.steps});

  /// The confirmation's title and text, what its button says, and the actions the host is asked for in order.
  final String title, confirm, label;
  final List<String> steps;
}

/// The steps of an action the host does itself: it downloads the files the model is made of, which needs the type's program, and
/// takes them away.
const String fetchFilesStep = 'fetch_files';
const String deleteFilesStep = 'delete_files';

/// The program a type's models run on, as the host says it: the store that installs it, its name and whether it is there.
final class ModelProgram {
  const ModelProgram({required this.provider, required this.title, required this.installed});

  final String provider, title;
  final bool installed;
}

/// How a model may be the person's speech-to-text.
final class ModelSpeech {
  const ModelSpeech({required this.text, this.caution});

  final String text;

  /// What the page warns of now, when it does.
  final String? caution;
}

/// One model, as the host listed it.
final class StoreModel {
  const StoreModel({
    required this.id,
    required this.summary,
    this.by,
    this.tags = const [],
    this.on = false,
    this.state,
    this.verified,
    this.engines = const [],
    this.facts = const [],
    this.actions = const [],
    this.sections = const [],
    this.speech,
    this.values = const {},
  });

  factory StoreModel.fromJson(Map<Object?, Object?> json) => StoreModel(
    id: json['id']! as String,
    by: _text(json['by']),
    summary: json['summary'] is String ? json['summary']! as String : '',
    tags: [for (final tag in json['tags'] is List ? json['tags']! as List : const []) if (tag is String) tag],
    on: json['on'] == true,
    state: _text(json['state']),
    verified: _text(json['verified']),
    engines: _list(
      json['engines'],
      (engine) => engine['id'] is String
          ? ModelEngine(
              id: engine['id']! as String,
              label: _text(engine['label']) ?? engine['id']! as String,
              installed: engine['state'] == 'installed',
              variants: _list(
                engine['variants'],
                (variant) => variant['id'] is String
                    ? ModelVariant(id: variant['id']! as String, label: _text(variant['label']) ?? variant['id']! as String, detail: _text(variant['detail']))
                    : null,
              ),
            )
          : null,
    ),
    facts: _list(json['facts'], (fact) => fact['label'] is String ? ModelFact(fact['label']! as String, '${fact['value'] ?? ''}') : null),
    actions: _list(
      json['actions'],
      (action) => ModelAction(
        title: '${action['title'] ?? ''}',
        confirm: '${action['confirm'] ?? ''}',
        label: '${action['label'] ?? ''}',
        steps: [for (final step in action['steps'] is List ? action['steps']! as List : const []) if (step is String) step],
      ),
    ),
    sections: _list(
      json['sections'],
      (section) => ModelSection(
        title: '${section['title'] ?? ''}',
        facts: _list(section['facts'], (fact) => fact['label'] is String ? ModelFact(fact['label']! as String, '${fact['value'] ?? ''}') : null),
        paragraphs: [for (final each in section['paragraphs'] is List ? section['paragraphs']! as List : const []) if (each is String) each],
        settings: section['settings'] == true,
      ),
    ),
    speech: json['speech'] is Map ? ModelSpeech(text: '${(json['speech']! as Map)['text'] ?? ''}', caution: _text((json['speech']! as Map)['caution'])) : null,
    values: _map(json['values']),
  );

  /// The name the host's calls take.
  final String id;
  final String? by;

  /// Where it stands and what it is, in a line.
  final String summary;
  final List<String> tags;

  /// Whether it is in use: the tile is marked.
  final bool on;

  /// Its state in a word, when it has one worth a badge.
  final String? state;

  /// The algorithm its files' digests are made with (`SHA-256`, `MD5`), the checks the download makes; null when a file has none.
  final String? verified;

  /// The ways it runs, as the type's data says: each is a pill on its tile.
  final List<ModelEngine> engines;
  final List<ModelFact> facts;

  /// What it can do; + does the first that is not a removal.
  final List<ModelAction> actions;

  /// What + does: the first action that does not take the model away, if there is one.
  ModelAction? get installAction => removeAction != null ? null : actions.firstOrNull;

  /// What an installed model offers besides its removal: writing the files an engine reads, from the ones already downloaded.
  List<ModelAction> get addActions => removeAction == null ? const [] : [for (final action in actions) if (!action.steps.contains(deleteFilesStep)) action];

  /// What the tile's remove does: the action that takes the model away, if there is one.
  ModelAction? get removeAction => actions.where((action) => action.steps.contains(deleteFilesStep)).firstOrNull;
  final List<ModelSection> sections;

  /// Present when it may be used as the person's speech-to-text.
  final ModelSpeech? speech;

  /// What canoa.cfg keeps for it, by the members of the type's settings.
  final Map<String, Object?> values;
}

/// The settings a type keeps for each model: a guatiao schema and a guatiao-intake form, drawn by the one renderer.
final class ModelSettings {
  const ModelSettings({required this.schema, required this.form, this.help});

  final Map<String, Object?> schema, form;
  final String? help;
}

/// The models of one Store type and what the person asks the host to do with one.
class TypeModels extends ChangeNotifier {
  TypeModels({
    required this.type,
    required this.title,
    required this._control,
    required this._open,
    this.programs,
  });

  /// The host's programs, which a model whose program is not there installs first.
  final ProgramStores? programs;

  /// The id of the Store type whose models these are, and the page's name in the plural.
  final String type;
  final String title;
  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;
  Future<void>? _asking;
  bool _loaded = false;
  List<StoreModel> _models = const [];
  String? _problem, _unavailable, _catalog, _publisher, _missing;
  ModelSettings? _settings;
  ModelProgram? _program;
  bool _speech = false;

  /// Whether the host has answered yet.
  bool get loaded => _loaded;
  List<StoreModel> get models => _models;

  /// What the host said when it could not list them, in its words.
  String? get problem => _problem;

  /// Why the host lists none (the program is not installed), in the library's words.
  String? get unavailable => _unavailable;

  /// What the page says without rows, when the host said nothing of its own.
  String get missing => _missing ?? 'The host lists no models now.';

  /// Where the rows come from, and who they are by when a row names no one.
  String get catalog => _catalog ?? title;
  String get publisher => _publisher ?? '';

  /// The settings a model keeps, when the type keeps any.
  ModelSettings? get settings => _settings;

  /// The program the models run on, when the type has one.
  ModelProgram? get program => _program;

  /// Whether a model of the type may be the person's speech-to-text.
  bool get speech => _speech;

  /// Asks the host for the models, once; the host answers from what it kept of its last listing while that is recent. [refresh]
  /// has it list them again.
  Future<void> ensure() => _asking ??= _ask(refresh: false);

  Future<void> refresh() => _ask(refresh: true);

  /// Asks the host again, which answers from what it kept while that is recent: what a page does each time it opens.
  Future<void> load() => _ask(refresh: false);

  Future<void> _ask({required bool refresh}) async {
    try {
      final client = await _control();
      final answer = await client.call('store/models', {'type': type, if (refresh) 'refresh': true}, null, const Duration(minutes: 3));
      final map = answer is Map ? answer : const {};
      _catalog = _text(map['catalog']);
      _publisher = _text(map['publisher']);
      _missing = _text(map['missing']);
      _unavailable = _text(map['unavailable']);
      _speech = map['speech'] == true;
      final program = _map(map['program']);
      _program = program['provider'] is String
          ? ModelProgram(provider: program['provider']! as String, title: _text(program['title']) ?? program['provider']! as String, installed: program['installed'] == true)
          : null;
      final settings = _map(map['settings']);
      _settings = settings['schema'] is Map ? ModelSettings(schema: _map(settings['schema']), form: _map(settings['form']), help: _text(settings['help'])) : null;
      _models = [
        for (final each in map['models'] is List ? map['models'] as List : const [])
          if (each is Map && each['id'] is String) StoreModel.fromJson(each),
      ];
      _problem = null;
      _loaded = true;
    } on Object catch (error) {
      _models = const [];
      _unavailable = null;
      _problem = hostWords(error);
      _loaded = true;
      _asking = null;
    }
    notifyListeners();
  }

  /// Makes [model] a speech-to-text provider on the host, which calls the type's service with it. The model is not loaded here:
  /// loading one is the person's choice. Returns the provider's name; throws what the host refuses with.
  Future<String> useForSpeech(String model) async {
    final client = await _control();
    final answer = await client.call('speech/enable', {'offer': type, 'model': model}, null, const Duration(seconds: 30));
    return answer is Map && answer['name'] is String ? answer['name'] as String : type;
  }

  /// Keeps [values] as the settings of [model] (none takes them away). Returns the host's note and what it kept; throws what the host
  /// refuses with. The list is read again after.
  Future<({String? note, Map<String, Object?> values})> setSettings(String model, Map<String, Object?> values) async {
    final client = await _control();
    try {
      final answer = await client.call('store/model_setting', {'type': type, 'model': model, 'values': values}, null, const Duration(seconds: 30));
      final map = _map(answer);
      return (note: _text(map['note']), values: _map(map['values']));
    } finally {
      await refresh();
    }
  }

  /// Asks the host to run [action] on [model], on a link of its own since a download takes long; throws what it refuses with. The
  /// list is read again after, whatever came of it.
  Future<void> run(String model, String action, {Map<String, String> options = const {}}) async {
    final client = await _open();
    try {
      await client.call('store/model', {'type': type, 'model': model, 'action': action, if (options.isNotEmpty) 'options': options}, null, modelWait);
    } finally {
      unawaited(client.close());
      await refresh();
    }
  }
}

/// The one page of the models every Store type's library lists, each grouped under the catalog its library names.
ExtensionStoreType modelsStoreType({required List<TypeModels> models, IconData icon = Icons.memory}) => ExtensionStoreType(
  id: modelsTypeId,
  label: modelsTypeLabel,
  icon: icon,
  changes: Listenable.merge([...models, ...{for (final each in models) ?each.programs}]),
  load: () => Future.wait([for (final each in models) each.load()]),
  items: () => [
    for (final of in models)
      if (of.loaded) ..._itemsOf(of),
  ],
  notices: () => [
    for (final of in models)
      if (of.loaded) ?_noticeOf(of),
  ],
);

/// What a type says in place of its models when the host could not list them or the program is not there: never an item.
ExtensionStoreNotice? _noticeOf(TypeModels models) {
  final problem = models.problem;
  if (problem != null) {
    return ExtensionStoreNotice(
      id: models.type,
      problem: true,
      words: (text) => text('store.type_problem', {'type': models.title, 'why': problem}),
      retry: models.refresh,
    );
  }
  final unavailable = models.unavailable;
  if (unavailable == null) return null;
  final program = models.program;
  final programs = models.programs;
  final status = program == null || program.installed ? null : programs?.status(program.provider);
  return ExtensionStoreNotice(
    id: models.type,
    words: (_) => unavailable,
    actionLabel: status == null ? null : (text) => text('store.install_program', {'program': status.info.title}),
    action: status == null || programs == null
        ? null
        : (context) async {
            final done = await runProgramAction(context, programs, status, 'install');
            if (done) await models.refresh();
          },
  );
}

List<ExtensionStoreItem> _itemsOf(TypeModels models) {
  if (models.problem != null || models.unavailable != null) return const [];
  return [
    for (final model in models.models)
      ExtensionStoreItem(
        catalog: models.catalog,
        name: model.id,
        by: model.by ?? models.publisher,
        summary: model.summary,
        tags: model.tags,
        installed: model.on,
        verified: model.verified,
        engines: [for (final engine in model.engines) StoreEngine(id: engine.id, label: engine.label, installed: engine.installed)],
        install: model.installAction == null ? null : (context) => _choose(context, models, model, model.installAction!),
        remove: model.removeAction == null ? null : (context) => _choose(context, models, model, model.removeAction!),
        details: (context, config) => ModelPage(models: models, id: model.id),
      ),
  ];
}

/// The confirmation of what + takes. An engine whose file the install writes in a form the person picks is asked about here, so the
/// answer maps each such engine's id to the variant picked; null when the person cancels.
Future<Map<String, String>?> _confirm(
  BuildContext context, {
  required String title,
  required String text,
  required String action,
  List<ModelEngine> engines = const [],
}) async {
  final picks = {for (final engine in engines) if (engine.variants.isNotEmpty) engine.id: engine.variants.first.id};
  final answer = await showSelectableDialog<bool>(
    context: context,
    builder: (dialog) => StatefulBuilder(
      builder: (dialog, setPicked) => AlertDialog(
        key: const Key('model-confirm'),
        title: Text(title),
        content: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(text),
            for (final engine in engines)
              if (engine.variants.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Column(key: Key('model-choice-${engine.id}'), crossAxisAlignment: CrossAxisAlignment.start, children: [
                    Text(dialog.t('store.engine_file', {'engine': engine.label}), style: Theme.of(dialog).textTheme.titleSmall),
                    RadioGroup<String>(
                      groupValue: picks[engine.id],
                      onChanged: (value) => setPicked(() => picks[engine.id] = value ?? picks[engine.id]!),
                      child: Column(children: [
                        for (final variant in engine.variants)
                          RadioListTile<String>(
                            key: Key('model-variant-${engine.id}-${variant.id}'),
                            contentPadding: EdgeInsets.zero,
                            dense: true,
                            value: variant.id,
                            title: Text(variant.label),
                            subtitle: variant.detail == null ? null : Text(variant.detail!),
                          ),
                      ]),
                    ),
                  ]),
                ),
          ]),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(dialog, false), child: const Text('Cancel')),
          FilledButton(key: const Key('model-confirm-yes'), onPressed: () => Navigator.pop(dialog, true), child: Text(action)),
        ],
      ),
    ),
  );
  return answer == true ? picks : null;
}

Future<void> _failed(BuildContext context, String title, Object error) => showSelectableDialog<void>(
  context: context,
  builder: (dialog) => AlertDialog(
    key: const Key('model-failed'),
    title: Text(title),
    content: SelectableText(hostWords(error)),
    actions: [TextButton(onPressed: () => Navigator.pop(dialog), child: const Text('Close'))],
  ),
);

/// What + does for [model]: [action], once the person has read what it takes, one step at a time. A model that is downloaded
/// for a program that is not on the host asks for that program first, in the one confirmation that shows the program's plan.
Future<void> _choose(BuildContext context, TypeModels models, StoreModel model, ModelAction action) async {
  final program = models.program;
  final programs = models.programs;
  final needs = program != null && !program.installed && action.steps.contains(fetchFilesStep);
  if (needs && programs != null) {
    try {
      await programs.ensure();
    } on Object catch (error) {
      if (context.mounted) await _failed(context, action.title, error);
      return;
    }
    final status = programs.status(program.provider);
    if (status != null) {
      if (!context.mounted) return;
      final installed = await runProgramAction(context, programs, status, 'install', also: '${action.title}. ${action.confirm}');
      if (!installed || !context.mounted) return;
      await _run(context, models, model, action, const {});
      return;
    }
  }
  if (!context.mounted) return;
  // A removal asks nothing of the engines: only what + installs is written in a form.
  final installs = !action.steps.contains(deleteFilesStep);
  final picks = await _confirm(context, title: action.title, text: action.confirm, action: action.label, engines: installs ? [for (final engine in model.engines) if (!engine.installed) engine] : const []);
  if (picks == null || !context.mounted) return;
  await _run(context, models, model, action, picks);
}

Future<void> _run(BuildContext context, TypeModels models, StoreModel model, ModelAction action, Map<String, String> options) async {
  try {
    for (final step in action.steps) {
      await models.run(model.id, step, options: options);
    }
  } on Object catch (error) {
    if (context.mounted) await _failed(context, action.title, error);
  }
}

/// One model's page: what the library says of it, section by section, the form of its settings where a section puts it, and, for a
/// model that may be the person's speech-to-text, "Use as speech provider", which makes the host's provider that sends dictation to
/// it. The settings are kept by the host in canoa.cfg.
class ModelPage extends StatefulWidget {
  const ModelPage({super.key, required this.models, required this.id});

  final TypeModels models;
  final String id;

  @override
  State<ModelPage> createState() => _ModelPageState();
}

class _ModelPageState extends State<ModelPage> {
  String? _said;
  bool _busy = false;
  String? _made;
  String? _speechProblem;

  StoreModel? get _model {
    for (final each in widget.models.models) {
      if (each.id == widget.id) return each;
    }
    return null;
  }

  Future<String?> _save(Map<String, Object?> values) async {
    setState(() => _said = null);
    try {
      final kept = await widget.models.setSettings(widget.id, values);
      if (mounted) setState(() => _said = kept.note ?? (kept.values.isEmpty ? 'The settings are removed.' : 'Saved.'));
      return null;
    } on Object catch (error) {
      return hostWords(error);
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

  Widget _settingsForm(StoreModel model, ModelSettings settings) => Padding(
    padding: const EdgeInsets.only(top: 8),
    child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      if (settings.help case final help?) Padding(padding: const EdgeInsets.only(bottom: 4), child: Text(help, key: const Key('model-setting-help'))),
      SchemaSettingsForm(
        key: ValueKey('${model.id} ${model.values}'),
        schema: settings.schema,
        form: settings.form,
        name: model.id,
        values: model.values,
        keyPrefix: 'model-setting',
        onSave: (values, cleared, from) => _save(values),
      ),
      if (_said != null) Text(_said!, key: const Key('model-setting-said')),
    ]),
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.models,
    builder: (context, _) {
      final model = _model;
      if (model == null) {
        return Scaffold(
          appBar: AppBar(title: const Text('Model')),
          body: const Padding(padding: EdgeInsets.all(16), child: Text('The host does not list this model now.', key: Key('model-gone'))),
        );
      }
      final settings = widget.models.settings;
      final speech = model.speech;
      return EntryDetails(
        key: Key('model-${model.id}'),
        title: model.id,
        leading: const Icon(Icons.memory, size: 56),
        from: widget.models.catalog,
        summary: model.summary,
        state: model.state,
        overview: {
          for (final fact in model.facts) fact.label: fact.value,
          context.t('store.verified'): model.verified ?? context.t('store.verified_no'),
        },
        contents: false,
        sections: [
          if (speech != null && widget.models.speech)
            DetailSection(key: const Key('model-speech'), title: 'Speech to text', children: [
              Padding(padding: const EdgeInsets.only(top: 4), child: Text(speech.text)),
              if (speech.caution case final caution?)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(caution, key: const Key('model-speech-caution'), style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant)),
                ),
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: FilledButton(key: const Key('model-use-speech'), onPressed: _busy ? null : () => unawaited(_use()), child: const Text('Use as speech provider')),
                ),
              ),
              if (_made case final made?)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text('Added as the provider "$made". Choose it in Settings, Voice.', key: const Key('model-speech-done')),
                ),
              if (_speechProblem case final problem?)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(problem, key: const Key('model-speech-failed'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ),
            ]),
          if (model.addActions.isNotEmpty)
            DetailSection(key: const Key('model-engines'), title: context.t('store.engines'), children: [
              for (final (at, action) in model.addActions.indexed)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: FilledButton.tonal(
                      key: Key('model-add-engine-$at'),
                      onPressed: () => unawaited(_choose(context, widget.models, model, action)),
                      child: Text(action.title),
                    ),
                  ),
                ),
            ]),
          for (final (at, section) in model.sections.indexed)
            DetailSection(key: Key('model-section-$at'), title: section.title, children: [
              for (final fact in section.facts) DetailField(label: fact.label, value: fact.value),
              for (final paragraph in section.paragraphs) Padding(padding: const EdgeInsets.only(top: 8), child: Text(paragraph)),
              if (section.settings && settings != null) _settingsForm(model, settings),
            ]),
        ],
      );
    },
  );
}
