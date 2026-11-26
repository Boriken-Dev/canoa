import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import '../shell/confirm_remove.dart';
import 'host_sections.dart';
import 'settings_row.dart';

/// A kind of row a collection may hold (`kinds` of the list's answer): its id, title and the schema and form of what adding one asks.
final class CollectionKind {
  const CollectionKind({required this.id, required this.title, this.description, this.schema, this.form});

  final String id;
  final String title;
  final String? description;
  final Map<String, Object?>? schema;
  final Map<String, Object?>? form;

  static CollectionKind? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'], title = json['title'];
    if (id is! String || id.isEmpty || title is! String || title.isEmpty) return null;
    return CollectionKind(
      id: id,
      title: title,
      description: json['description'] is String ? json['description']! as String : null,
      schema: json['schema'] is Map ? (json['schema']! as Map).cast<String, Object?>() : null,
      form: json['form'] is Map ? (json['form']! as Map).cast<String, Object?>() : null,
    );
  }
}

/// One row of a collection, as the list's answer words it. It never carries a secret.
final class CollectionRow {
  const CollectionRow({required this.name, required this.kind, required this.state, this.kindTitle, this.details = const []});

  final String name;
  final String kind;
  final String? kindTitle;

  /// What the host says of the row's sign-in or condition.
  final String state;
  final List<String> details;

  static CollectionRow? parse(Object? json, String kindMember) {
    if (json is! Map) return null;
    final name = json['name'], state = json['state'];
    if (name is! String || state is! String) return null;
    final kind = json[kindMember];
    final title = json['${kindMember}_title'];
    final raw = json['details'];
    return CollectionRow(
      name: name,
      kind: kind is String ? kind : '',
      state: state,
      kindTitle: title is String ? title : null,
      details: [
        if (raw is List)
          for (final item in raw)
            if (item is String && item.isNotEmpty) item,
      ],
    );
  }

  /// What the row's second line says: its kind's title, what it reports of itself and its state.
  String describe(List<CollectionKind> kinds) =>
      [kindTitle ?? kinds.where((each) => each.id == kind).firstOrNull?.title ?? kind, ...details, state].join(' · ');
}

/// What a check of one row answered.
final class CollectionTest {
  const CollectionTest({required this.ok, this.message, this.problem});

  final bool ok;
  final String? message, problem;

  static CollectionTest parse(Object? json) {
    if (json is! Map) return const CollectionTest(ok: false);
    return CollectionTest(
      ok: json['ok'] == true,
      message: json['message'] is String ? json['message']! as String : null,
      problem: json['problem'] is String ? json['problem']! as String : null,
    );
  }
}

/// The rows and kinds a section's collection lists, and the result of the last check of each row.
class CollectionController extends ChangeNotifier {
  CollectionController({required this.collection, required this.sections});

  final SectionCollection collection;
  final HostSections sections;

  List<CollectionRow> _rows = const [];
  List<CollectionKind> _kinds = const [];
  final _tests = <String, CollectionTest>{};
  final _testing = <String>{};
  ControlFailed? _denied;
  Object? _failure;
  bool _loading = false, _disposed = false;

  List<CollectionRow> get rows => _rows;
  List<CollectionKind> get kinds => _kinds;
  bool get loading => _loading;

  /// Why the host would not list the rows, as it answered: null when it did.
  ControlFailed? get denied => _denied;

  /// What went wrong reaching the host, when it did not answer.
  Object? get failure => _failure;

  bool get problem => _denied != null || _failure != null;

  CollectionRow? named(String name) => _rows.where((each) => each.name == name).firstOrNull;

  CollectionTest? testOf(String name) => _tests[name];

  bool isTesting(String name) => _testing.contains(name);

  Future<void> refresh() async {
    _loading = true;
    _notify();
    try {
      final answer = await sections.service(collection.list, const {});
      final items = answer is Map ? answer[collection.items] : null;
      final kinds = answer is Map ? answer[collection.kinds] : null;
      _rows = [if (items is List) for (final each in items) ?CollectionRow.parse(each, collection.kindMember)];
      _kinds = [if (kinds is List) for (final each in kinds) ?CollectionKind.parse(each)];
      _denied = null;
      _failure = null;
    } on ControlFailed catch (error) {
      _denied = error;
      _failure = null;
    } on Object catch (error) {
      _denied = null;
      _failure = error;
    } finally {
      _loading = false;
      _notify();
    }
  }

  /// Adds a row; the host's refusal is thrown as it words it, and the list is read again either way.
  Future<void> add(String name, String kind, Map<String, Object?> values) async {
    try {
      await sections.service(collection.add, {'name': name, collection.kindMember: kind, ...values});
    } finally {
      await refresh();
    }
  }

  /// Checks [name] and keeps what was found for [testOf].
  Future<void> test(String name) async {
    final method = collection.test;
    if (method == null) return;
    _testing.add(name);
    _tests.remove(name);
    _notify();
    CollectionTest result;
    try {
      result = CollectionTest.parse(await sections.service(method, {'name': name}, slow: true));
    } on ControlFailed catch (error) {
      result = CollectionTest(ok: false, problem: error.message);
    } on Object catch (error) {
      result = CollectionTest(ok: false, problem: '$error');
    }
    _testing.remove(name);
    _tests[name] = result;
    _notify();
  }

  Future<void> remove(String name) async {
    try {
      await sections.service(collection.remove, {'name': name});
    } finally {
      _tests.remove(name);
      await refresh();
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// The rows of a section's collection, each with its check and Remove, and the button that adds one.
class SectionCollectionList extends StatefulWidget {
  const SectionCollectionList({super.key, required this.id, required this.collection, required this.sections});

  final String id;
  final SectionCollection collection;
  final HostSections sections;

  @override
  State<SectionCollectionList> createState() => _SectionCollectionListState();
}

class _SectionCollectionListState extends State<SectionCollectionList> {
  late CollectionController _rows = CollectionController(collection: widget.collection, sections: widget.sections);

  String get _id => widget.id;

  @override
  void initState() {
    super.initState();
    unawaited(_rows.refresh());
  }

  @override
  void didUpdateWidget(SectionCollectionList oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.sections != widget.sections || oldWidget.collection != widget.collection) {
      _rows.dispose();
      _rows = CollectionController(collection: widget.collection, sections: widget.sections);
      unawaited(_rows.refresh());
    }
  }

  @override
  void dispose() {
    _rows.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    await showDialog<void>(
      context: context,
      builder: (_) => AddCollectionRowDialog(id: _id, collection: widget.collection, controller: _rows),
    );
  }

  Future<void> _remove(CollectionRow row) async {
    final t = context.t;
    if (!await confirmRemoval(
      context,
      title: t('collection.remove_title', {'name': row.name}),
      body: widget.collection.removeNote ?? t('collection.remove_body'),
      confirmLabel: t('collection.remove'),
      cancelLabel: t('collection.keep'),
      dialogKey: Key('section-$_id-remove-dialog'),
      cancelKey: null,
      confirmKey: Key('section-$_id-remove-confirm'),
    )) {
      return;
    }
    try {
      await _rows.remove(row.name);
    } on Object catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(t('collection.remove_failed', {'name': row.name, 'why': error}))));
    }
  }

  String _problem(BuildContext context) {
    final denied = _rows.denied;
    if (denied != null) return denied.code == -32001 ? context.t('collection.denied') : denied.message;
    return context.t('collection.list_failed', {'why': _rows.failure});
  }

  @override
  Widget build(BuildContext context) {
    final collection = widget.collection;
    return ListenableBuilder(
      listenable: _rows,
      builder: (context, _) {
        final scheme = Theme.of(context).colorScheme;
        final canAdd = _rows.kinds.isNotEmpty || !_rows.problem;
        return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          if (_rows.problem)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(_problem(context), key: Key('section-$_id-collection-problem'), style: TextStyle(color: scheme.error)),
            ),
          Align(
            alignment: Alignment.centerRight,
            child: TextButton.icon(
              key: Key('section-$_id-add'),
              onPressed: _rows.loading || !canAdd ? null : () => unawaited(_add()),
              icon: const Icon(Icons.add, size: 18),
              label: Text(collection.addLabel ?? context.t('collection.add')),
            ),
          ),
          SettingsGroup(
            key: Key('section-$_id-rows'),
            children: [
              if (_rows.rows.isEmpty)
                SettingsRow(
                  title: _rows.loading ? context.t('collection.reading') : (collection.empty ?? context.t('collection.empty')),
                  description: collection.emptyHelp,
                ),
              for (final row in _rows.rows) _row(context, row),
            ],
          ),
        ]);
      },
    );
  }

  Widget _row(BuildContext context, CollectionRow row) {
    final scheme = Theme.of(context).colorScheme;
    final result = _rows.testOf(row.name);
    final testing = _rows.isTesting(row.name);
    final collection = widget.collection;
    return Column(key: Key('section-$_id-row ${row.name}'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        title: row.name,
        description: row.describe(_rows.kinds),
        below: testing
            ? LinearProgressIndicator(key: Key('section-$_id-testing'))
            : result == null
                ? null
                : Text(
                    result.ok ? (result.message ?? context.t('collection.test_ok')) : (result.problem ?? context.t('collection.test_failed')),
                    key: Key('section-$_id-test-result ${row.name}'),
                    style: TextStyle(color: result.ok ? scheme.primary : scheme.error),
                  ),
        control: Row(mainAxisSize: MainAxisSize.min, children: [
          if (collection.test != null)
            OutlinedButton(
              key: Key('section-$_id-test ${row.name}'),
              onPressed: testing ? null : () => unawaited(_rows.test(row.name)),
              child: Text(collection.testLabel ?? context.t('collection.test')),
            ),
          IconButton(
            key: Key('section-$_id-remove ${row.name}'),
            tooltip: context.t('collection.remove'),
            icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
            onPressed: testing ? null : () => unawaited(_remove(row)),
          ),
        ]),
      ),
    ]);
  }
}

/// Adds one row from the kind the person picks, asking for what that kind's schema and form describe.
class AddCollectionRowDialog extends StatefulWidget {
  const AddCollectionRowDialog({super.key, required this.id, required this.collection, required this.controller});

  final String id;
  final SectionCollection collection;
  final CollectionController controller;

  @override
  State<AddCollectionRowDialog> createState() => _AddCollectionRowDialogState();
}

class _AddCollectionRowDialogState extends State<AddCollectionRowDialog> {
  final _name = TextEditingController();
  final _values = <String, Map<String, Object?>>{};
  String? _kindId;
  int _generation = 0;
  String? _problem;
  bool _saving = false;

  String get _id => widget.id;

  @override
  void initState() {
    super.initState();
    _kindId = widget.controller.kinds.firstOrNull?.id;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  CollectionKind? get _kind {
    final kinds = widget.controller.kinds;
    return kinds.where((each) => each.id == _kindId).firstOrNull ?? kinds.firstOrNull;
  }

  Future<void> _submit() async {
    final name = _name.text.trim();
    final t = context.t;
    if (name.isEmpty) return setState(() => _problem = t('collection.name_needed'));
    if (widget.controller.named(name) != null) return setState(() => _problem = t('collection.name_taken'));
    final kind = _kind;
    if (kind == null) return setState(() => _problem = t('collection.no_kinds'));

    final schema = kind.schema ?? const {};
    final form = kind.form ?? const {};
    final typed = _values[kind.id] ?? const {};
    final localProblem = schema.isEmpty ? null : schemaProblem(schema, form, typed);
    if (localProblem != null) return setState(() => _problem = localProblem);

    final values = schema.isEmpty ? typed : collectSchemaValues(schema, form, typed);
    setState(() {
      _problem = null;
      _saving = true;
    });
    try {
      await widget.controller.add(name, kind.id, values);
      if (mounted) Navigator.of(context).pop();
    } on ControlFailed catch (error) {
      if (mounted) setState(() => _problem = error.message);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = '$error');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.t;
    final kinds = widget.controller.kinds;
    final kind = _kind;
    final schema = kind?.schema ?? const {};
    final form = kind?.form ?? const {};
    return AlertDialog(
      key: Key('section-$_id-add-dialog'),
      title: Text(widget.collection.addLabel ?? t('collection.add')),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            TextField(
              key: Key('section-$_id-name'),
              controller: _name,
              decoration: InputDecoration(labelText: t('collection.name')),
              textInputAction: TextInputAction.next,
            ),
            if (kinds.isNotEmpty) ...[
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                key: Key('section-$_id-kind-picker'),
                isExpanded: true,
                initialValue: kind?.id,
                decoration: InputDecoration(labelText: widget.collection.kindLabel ?? t('collection.kind')),
                items: [for (final each in kinds) DropdownMenuItem<String>(value: each.id, child: Text(each.title))],
                onChanged: _saving
                    ? null
                    : (id) => setState(() {
                          _kindId = id;
                          _generation++;
                          _problem = null;
                        }),
              ),
              if (kind?.description case final description?)
                Padding(padding: const EdgeInsets.only(top: 6), child: Text(description, style: Theme.of(context).textTheme.bodySmall)),
            ] else
              Padding(padding: const EdgeInsets.only(top: 12), child: Text(t('collection.no_kinds'))),
            if (kind != null && schema.isNotEmpty) ...[
              const SizedBox(height: 8),
              SchemaFields(
                key: ValueKey('$_id-${kind.id}-$_generation'),
                schema: schema,
                form: form,
                values: _values[kind.id] ?? const {},
                idPrefix: 'section-$_id',
                unsetEntry: true,
                onValues: (values) => _values[kind.id] = values,
                onSubmitted: () => unawaited(_submit()),
              ),
            ],
            if (_problem != null) ...[
              const SizedBox(height: 10),
              Text(
                _problem!,
                key: Key('section-$_id-add-problem'),
                style: Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error),
              ),
            ],
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: _saving ? null : () => Navigator.of(context).pop(), child: Text(t('common.cancel'))),
        FilledButton(
          key: Key('section-$_id-add-submit'),
          onPressed: _saving || kind == null ? null : _submit,
          child: Text(_saving ? t('collection.adding') : t('collection.add_confirm')),
        ),
      ],
    );
  }
}
