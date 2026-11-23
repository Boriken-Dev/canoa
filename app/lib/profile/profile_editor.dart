/// The editor of one profile of canoa.cfg's `[profiles]`: its name and the values a new session starts with when it is chosen: the
/// source it runs on, its model, effort and mode, its starting instructions (text added to the session's initial context after
/// every other layer's), the prompt cache lifetime and the loadout. The account and the permissions are the session's own and
/// are not part of one.
library;

import 'package:material_ui/material_ui.dart';

import '../chat/cache_lifetime.dart';
import '../config/canoa_config.dart';
import '../forms/schema_fields.dart';
import '../sources/source_choices.dart' show ModelChoice;
import 'profile.dart' show ProfileAccess;
import 'profile_items.dart';

/// The items the editor draws a row of its own for, which the host also lists: the rest of the host's list is drawn from the forms
/// the providers describe.
const _ownRows = {'model', 'effort', 'mode', 'context'};

/// What a source's agent is known to offer before a session starts: its models, each with the efforts it takes, and its modes as
/// (id, name). The editor offers them beside a box that takes any text, since an agent may accept what it did not list.
final class ProfileChoices {
  const ProfileChoices({this.models = const [], this.modes = const []});

  final List<ModelChoice> models;
  final List<(String, String)> modes;

  /// The efforts to offer for [model]: those that model takes when it is listed, else every effort any listed model takes.
  List<String> effortsFor(String? model) {
    final listed = models.where((each) => each.id == model).firstOrNull;
    if (listed != null && listed.efforts.isNotEmpty) return listed.efforts;
    return {for (final each in models) ...each.efforts}.toList();
  }
}

class ProfileEditor extends StatefulWidget {
  const ProfileEditor({
    super.key,
    required this.original,
    required this.preset,
    required this.taken,
    required this.sources,
    required this.loadouts,
    required this.onSave,
    required this.onCancel,
    this.catalog,
    this.certificates,
    this.choicesFor,
  });

  /// What the agent a profile runs on offers, asked when the source is chosen; none offers nothing to pick from.
  final Future<ProfileChoices?> Function(String source)? choicesFor;

  /// The items the host lists, which the editor draws a form for; [profileItems] when none is given.
  final ProfileItemCatalog? catalog;

  /// The client certificates the host lists, which a form that asks for one offers; [profileCertificates] when none is given.
  final ClientCertificateChoices? certificates;

  /// The name the profile has in canoa.cfg; null for a new one.
  final String? original;

  /// What it holds now.
  final Preset preset;

  /// The names the other profiles have.
  final Set<String> taken;

  /// The sources a profile may run on, by label.
  final List<String> sources;

  /// The loadouts canoa.cfg names.
  final List<String> loadouts;

  /// Writes the profile; a refusal is the answer, shown beside Save.
  final Future<String?> Function(String name, Preset preset) onSave;
  final VoidCallback onCancel;

  @override
  State<ProfileEditor> createState() => _ProfileEditorState();
}

class _ProfileEditorState extends State<ProfileEditor> {
  late final _name = TextEditingController(text: widget.original ?? '');
  late final _model = TextEditingController(text: widget.preset.model ?? '');
  late final _effort = TextEditingController(text: widget.preset.effort ?? '');
  late final _mode = TextEditingController(text: widget.preset.mode ?? '');
  late final _context = TextEditingController(text: widget.preset.context ?? '');
  late String? _source = widget.preset.source;
  late String? _cache = widget.preset.cacheLifetime;
  late String? _loadout = widget.preset.loadout;
  late ProfileAccess _access = widget.preset.access;
  late Map<String, Map<String, Object?>> _items = {for (final MapEntry(:key, :value) in widget.preset.items.entries) key: {...value}};
  String? _problem;
  bool _saving = false;
  ProfileChoices? _offered;

  @override
  void initState() {
    super.initState();
    _ask();
  }

  /// Asks what the chosen source's agent offers; what was asked for a source since left is dropped.
  void _ask() {
    final source = _source;
    final ask = widget.choicesFor;
    if (source == null || ask == null) {
      _offered = null;
      return;
    }
    ask(source).then((offered) {
      if (mounted && _source == source) setState(() => _offered = offered);
    }, onError: (Object _) {});
  }

  @override
  void dispose() {
    for (final controller in [_name, _model, _effort, _mode, _context]) {
      controller.dispose();
    }
    super.dispose();
  }

  String? get _nameProblem {
    final name = _name.text;
    if (name.isEmpty) return null;
    if (!isPresetName(name)) return presetNameRule;
    if (widget.taken.any((other) => other.toLowerCase() == name.toLowerCase() && other != widget.original)) {
      return 'Another profile has that name';
    }
    return null;
  }

  static String? _text(TextEditingController controller) {
    final text = controller.text.trim();
    return text.isEmpty ? null : text;
  }

  Preset _made() => Preset(
    source: _source,
    model: _text(_model),
    effort: _text(_effort),
    mode: _text(_mode),
    // Instructions are kept as written, but a field of only blanks is none.
    context: _context.text.trim().isEmpty ? null : _context.text,
    cacheLifetime: _cache,
    loadout: _loadout,
    access: _access,
    items: _items,
  );

  ProfileItemCatalog get _catalog => widget.catalog ?? profileItems;
  ClientCertificateChoices get _certificates => widget.certificates ?? profileCertificates;

  /// What the form of [item] holds now.
  Map<String, Object?> _valuesOf(ProfileItemInfo item) => item.id == 'access'
      ? {'sandbox': ?_access.sandbox, 'network': ?_access.network, 'approvals': ?_access.approvals}
      : _items[item.id] ?? const {};

  /// Takes what the form of [item] holds: the access is the profile's own setting, any other item's values are kept under its id,
  /// and an item with no value left is no longer set.
  void _take(ProfileItemInfo item, Map<String, Object?> values) {
    setState(() {
      if (item.id == 'access') {
        _access = ProfileAccess(
          sandbox: values['sandbox'] as String?,
          network: values['network'] as bool?,
          approvals: values['approvals'] as String?,
        );
      } else if (values.isEmpty) {
        _items = {..._items}..remove(item.id);
      } else {
        _items = {..._items, item.id: values};
      }
    });
  }

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _problem = null;
    });
    final refused = await widget.onSave(_name.text, _made());
    if (mounted) {
      setState(() {
        _saving = false;
        _problem = refused;
      });
    }
  }

  DropdownMenuItem<String?> _none(String words) => DropdownMenuItem<String?>(value: null, child: Text(words));

  @override
  Widget build(BuildContext context) {
    final ready = _name.text.isNotEmpty && _nameProblem == null && !_saving;
    // A value the file holds that the list no longer offers stays shown, so saving does not drop it unseen.
    final sources = {...widget.sources, ?_source}.toList();
    final loadouts = {...widget.loadouts, ?_loadout}.toList();
    return Column(
      key: const Key('profile-editor'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(widget.original == null ? 'New profile' : 'Edit ${widget.original}', style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-name'),
          controller: _name,
          // The default profile keeps its name: sessions that name none start with it.
          enabled: widget.original != presetDefault,
          decoration: InputDecoration(labelText: 'Name', errorText: _nameProblem),
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 8),
        DropdownButtonFormField<String?>(
          key: const Key('profile-edit-source'),
          isExpanded: true,
          initialValue: _source,
          decoration: const InputDecoration(labelText: 'Source'),
          items: [
            _none('The session\'s own'),
            for (final label in sources) DropdownMenuItem<String?>(value: label, child: Text(label)),
          ],
          onChanged: (value) => setState(() {
            _source = value;
            _offered = null;
            _ask();
          }),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: _ChoiceField(
              id: 'model',
              label: 'Model',
              controller: _model,
              choices: [for (final model in _offered?.models ?? const <ModelChoice>[]) (model.id, model.name == model.id ? model.id : '${model.name} (${model.id})')],
              onPicked: () => setState(() {}),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: _ChoiceField(
              id: 'effort',
              label: 'Effort',
              controller: _effort,
              choices: [for (final effort in _offered?.effortsFor(_text(_model)) ?? const <String>[]) (effort, effort)],
              onPicked: () => setState(() {}),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: _ChoiceField(
              id: 'mode',
              label: 'Mode',
              controller: _mode,
              choices: _offered?.modes ?? const [],
              onPicked: () => setState(() {}),
            ),
          ),
        ]),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-context'),
          controller: _context,
          minLines: 3,
          maxLines: 10,
          decoration: const InputDecoration(
            labelText: 'Starting instructions',
            alignLabelWithHint: true,
            helperText: 'Added to the session\'s initial context after every other layer\'s',
          ),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-cache'),
              isExpanded: true,
              initialValue: _cache,
              decoration: const InputDecoration(labelText: 'Prompt cache'),
              items: [
                _none('Not set'),
                for (final lifetime in CacheLifetime.values)
                  DropdownMenuItem<String?>(
                    value: lifetime.id,
                    child: Text('${lifetime.label}: ${lifetime.about}', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _cache = value),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-loadout'),
              isExpanded: true,
              initialValue: _loadout,
              decoration: const InputDecoration(labelText: 'Loadout'),
              items: [
                _none('The default'),
                for (final name in loadouts)
                  DropdownMenuItem<String?>(
                    value: name,
                    child: Text(widget.loadouts.contains(name) ? name : '$name (not in canoa.cfg)', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _loadout = value),
            ),
          ),
        ]),
        ListenableBuilder(
          listenable: Listenable.merge([_catalog, _certificates]),
          builder: (context, _) {
            final listed = [for (final item in _catalog.items) if (!_ownRows.contains(item.id)) item];
            // An item the profile holds values for that no provider offers now (its extension was removed): the values stay, and say so.
            final gone = [for (final id in _items.keys) if (_catalog.answered && _catalog.byId(id) == null) id];
            return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final item in listed)
                _ItemSection(
                  key: ValueKey('item-${item.id}'),
                  item: item,
                  values: _valuesOf(item),
                  choices: {clientCertificateWidget: _certificates.choices},
                  onChanged: (values) => _take(item, values),
                ),
              for (final id in gone)
                ListTile(
                  key: Key('item-gone-$id'),
                  dense: true,
                  leading: Icon(Icons.warning_amber_outlined, color: Theme.of(context).colorScheme.error),
                  title: Text(id),
                  subtitle: const Text('No extension offers this item now: its values are kept and not applied.'),
                  trailing: TextButton(
                    key: Key('item-gone-remove-$id'),
                    onPressed: () => setState(() => _items = {..._items}..remove(id)),
                    child: const Text('Remove its values'),
                  ),
                ),
            ]);
          },
        ),
        if (_problem != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(_problem!, key: const Key('profile-edit-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        const SizedBox(height: 8),
        Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          TextButton(key: const Key('profile-edit-cancel'), onPressed: widget.onCancel, child: const Text('Cancel')),
          const SizedBox(width: 8),
          FilledButton(key: const Key('profile-edit-save'), onPressed: ready ? _save : null, child: const Text('Save')),
        ]),
      ],
    );
  }
}

/// A box that takes any text, with a menu of what the agent is known to offer beside it: picking fills the box, and what is typed that
/// the agent did not list is kept as typed.
class _ChoiceField extends StatelessWidget {
  const _ChoiceField({required this.id, required this.label, required this.controller, required this.choices, required this.onPicked});

  final String id, label;
  final TextEditingController controller;

  /// (value, label) of what the agent offers; empty when it is not known, which leaves only the box.
  final List<(String, String)> choices;
  final VoidCallback onPicked;

  @override
  Widget build(BuildContext context) => TextField(
    key: Key('profile-edit-$id'),
    controller: controller,
    decoration: InputDecoration(
      labelText: label,
      hintText: 'The agent\'s own',
      suffixIcon: choices.isEmpty
          ? null
          : PopupMenuButton<String>(
              key: Key('profile-edit-$id-pick'),
              tooltip: 'What the agent offers',
              icon: const Icon(Icons.arrow_drop_down),
              onSelected: (value) {
                controller.text = value;
                onPicked();
              },
              itemBuilder: (context) => [for (final (value, title) in choices) PopupMenuItem<String>(value: value, child: Text(title))],
            ),
    ),
  );
}

/// The form one item's provider describes, drawn from its schema and form: it holds the values the person gave and nothing else.
class _ItemSection extends StatelessWidget {
  const _ItemSection({super.key, required this.item, required this.values, required this.choices, required this.onChanged});

  final ProfileItemInfo item;
  final Map<String, Object?> values;
  final Map<String, List<SchemaChoice>> choices;
  final ValueChanged<Map<String, Object?>> onChanged;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        key: Key('item-section-${item.id}'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(item.title, style: Theme.of(context).textTheme.titleSmall),
          if (item.help != null) Text(item.help!, style: Theme.of(context).textTheme.bodySmall),
          if (item.harnesses.isNotEmpty) Text('For ${item.harnesses.join(', ')} only', style: Theme.of(context).textTheme.bodySmall),
          SchemaFields(
            schema: item.schema,
            form: item.form,
            values: values,
            idPrefix: 'item-${item.id}',
            unsetEntry: true,
            choices: choices,
            onValues: (held) => onChanged(collectSchemaValues(item.schema, item.form, held)),
          ),
        ],
      ),
    );
  }
}
