import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/catalog_specs.dart';
import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../selectable.dart';
import '../l10n/app_text.dart';
import '../settings/off_switch.dart';
import '../settings/settings_row.dart';
import '../shell/confirm_remove.dart';
import 'catalog_types.dart';
import 'overlay_page.dart' show oneLine;
import 'source_uri.dart';

/// What the app asks the host with: `store/catalogs` and `store/catalog_set`.
typedef CatalogCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// The catalogs of every Store type, as the host merges them: what each type's extension registers, then canoa.cfg's entries by label.
/// A default can be turned off and on, replaced by an entry of the same label, or left; an entry can be added or taken out. Every
/// change is one call to the host, which writes canoa.cfg and answers with the rows it now has.
class CatalogSources extends ChangeNotifier {
  CatalogSources(this._call);

  final CatalogCall _call;

  bool _disposed = false;
  List<CatalogType> _types = const [];
  String? _problem;
  bool _loading = false, _loaded = false;
  final _busy = <String>{};

  List<CatalogType> get types => _types;
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;

  /// Whether a change of the catalog [label] of the type [type] is being made.
  bool busy(String type, String label) => _busy.contains('$type/$label');

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads the types and what the host last read of each catalog; nothing is fetched to show them.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      _types = catalogTypesFromAnswer(await _call('store/catalogs', const {}));
      _problem = null;
      _loaded = true;
    } on ControlFailed catch (error) {
      _problem = error.message;
    } on Object catch (error) {
      _problem = '$error';
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  /// Makes the catalog [label] of [type] what [entry] says: `false` turns it off, a string or a table replaces or adds it, and null
  /// takes the entry out of canoa.cfg. Returns what is wrong in the host's words, or null once canoa.cfg holds it.
  Future<String?> set(String type, String label, Object? entry) async {
    final key = '$type/$label';
    if (!_busy.add(key)) return null;
    notifyListeners();
    try {
      await _call('store/catalog_set', {'type': type, 'label': label, 'entry': entry});
      await load();
      return null;
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return '$error';
    } finally {
      _busy.remove(key);
      notifyListeners();
    }
  }
}

/// The word and the mark of a catalog whose answer is fixed to a revision.
class TrustBadge extends StatelessWidget {
  const TrustBadge({super.key, required this.trust});

  /// `pinned`.
  final String trust;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme.tertiary;
    return Tooltip(
      message: context.t('catalogs.trust.$trust.help'),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        Icon(Icons.push_pin_outlined, size: 14, color: color),
        const SizedBox(width: 2),
        Text(context.t('catalogs.trust.$trust'), style: Theme.of(context).textTheme.labelSmall?.copyWith(color: color)),
      ]),
    );
  }
}

/// Settings, Catalog sources: each Store type's catalogs with where each comes from, whether it is pinned and when it was read.
class CatalogSourcesSection extends StatefulWidget {
  const CatalogSourcesSection({super.key, required this.sources});

  final CatalogSources sources;

  @override
  State<CatalogSourcesSection> createState() => _CatalogSourcesSectionState();
}

class _CatalogSourcesSectionState extends State<CatalogSourcesSection> {
  @override
  void initState() {
    super.initState();
    if (!widget.sources.loaded && !widget.sources.loading) unawaited(widget.sources.load());
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.sources,
    builder: (context, _) {
      final s = widget.sources;
      final scheme = Theme.of(context).colorScheme;
      return Column(key: const Key('catalog-sources-section'), crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('catalogs.intro'), style: Theme.of(context).textTheme.bodyMedium?.copyWith(color: scheme.onSurfaceVariant)),
        const SizedBox(height: 12),
        if (s.problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text(oneLine(problem), key: const Key('catalog-sources-problem'), style: TextStyle(color: scheme.error)),
          ),
        if (s.loading && !s.loaded) const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()),
        for (final type in s.types)
          SettingsGroup(
            key: Key('catalog-type-${type.id}'),
            title: '${type.title} · ${type.words.plural}',
            trailing: TextButton.icon(
              key: Key('catalog-add-${type.id}'),
              onPressed: () => unawaited(_edit(context, type, null)),
              icon: const Icon(Icons.add, size: 18),
              label: Text(context.t('catalogs.add', {'noun': type.words.noun})),
            ),
            children: [
              if (type.rows.isEmpty) Padding(padding: const EdgeInsets.all(12), child: Text(context.t('catalogs.none', {'plural': type.words.plural}))),
              for (final row in type.rows) _CatalogSourceTile(key: Key('catalog-source-${type.id}-${row.label}'), sources: s, type: type, row: row, onEdit: () => unawaited(_edit(context, type, row))),
            ],
          ),
      ]);
    },
  );

  Future<void> _edit(BuildContext context, CatalogType type, CatalogRow? row) => showSelectableDialog<void>(
    context: context,
    builder: (dialog) => CatalogEntryDialog(sources: widget.sources, type: type, replacing: row),
  );
}

/// One catalog: its label, where it comes from and whether it is pinned on the first line; where it reads from, what pins it and when
/// it was read below; and what can be done with it at the right.
class _CatalogSourceTile extends StatelessWidget {
  const _CatalogSourceTile({super.key, required this.sources, required this.type, required this.row, required this.onEdit});

  final CatalogSources sources;
  final CatalogType type;
  final CatalogRow row;
  final VoidCallback onEdit;

  Future<void> _turn(BuildContext context, bool on) async {
    final problem = await sources.set(type.id, row.label, on ? null : false);
    if (problem != null && context.mounted) ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(problem)));
  }

  Future<void> _remove(BuildContext context) async {
    if (!await confirmRemoval(
      context,
      title: context.t('catalogs.remove_title', {'label': row.label}),
      body: context.t(row.origin == 'replaced' ? 'catalogs.remove_replaced' : 'catalogs.remove_added'),
      confirmLabel: context.t(row.origin == 'replaced' ? 'catalogs.restore' : 'catalogs.remove'),
      cancelLabel: context.t('common.cancel'),
      dialogKey: Key('catalog-remove-dialog-${row.label}'),
      cancelKey: null,
      confirmKey: Key('catalog-remove-confirm-${row.label}'),
    ) || !context.mounted) {
      return;
    }
    final problem = await sources.set(type.id, row.label, null);
    if (problem != null && context.mounted) ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(problem)));
  }

  String _read(BuildContext context) {
    if (row.from == 'snapshot') return context.t('catalogs.read.snapshot');
    final at = row.fetched;
    if (row.from == null || at == null) return '';
    final time = DateTime.fromMillisecondsSinceEpoch(at);
    String two(int n) => n.toString().padLeft(2, '0');
    return context.t('catalogs.read.at', {'when': '${time.year}-${two(time.month)}-${two(time.day)} ${two(time.hour)}:${two(time.minute)}'});
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final busy = sources.busy(type.id, row.label);
    final lines = [
      if (row.problem != null) row.problem!,
      if (row.problem == null) row.shown,
      if (row.pin != null) context.t('catalogs.pinned_at', {'pin': row.pin}),
      if (row.ttl != null) context.t('catalogs.ttl', {'seconds': row.ttl}),
      _read(context),
    ].where((line) => line.isNotEmpty).map(oneLine).join('\n');
    return SettingsRow(
      title: row.label,
      description: lines,
      control: Row(mainAxisSize: MainAxisSize.min, children: [
        _Chip(key: Key('catalog-origin-${type.id}-${row.label}'), text: context.t('catalogs.origin.${row.origin}')),
        if (row.trust != null && row.enabled) ...[const SizedBox(width: 8), TrustBadge(key: Key('catalog-trust-${type.id}-${row.label}'), trust: row.trust!)],
        if (!row.enabled) ...[const SizedBox(width: 8), OffBadge(reason: context.t('catalogs.off_reason', {'label': row.label}))],
        const SizedBox(width: 4),
        if (!row.fixed && row.origin != 'added')
          IconButton(
            key: Key('catalog-replace-${type.id}-${row.label}'),
            tooltip: context.t('catalogs.replace'),
            icon: const Icon(Icons.edit_outlined),
            onPressed: busy ? null : onEdit,
          ),
        if (row.origin == 'default' && !row.fixed)
          OffSwitch(
            key: Key('catalog-switch-${type.id}-${row.label}'),
            on: row.enabled,
            label: context.t('switch.turn', {'name': row.label}),
            onChanged: busy ? null : (on) => unawaited(_turn(context, on)),
          )
        else if (!row.fixed)
          IconButton(
            key: Key('catalog-remove-${type.id}-${row.label}'),
            tooltip: context.t(row.origin == 'replaced' ? 'catalogs.restore' : 'catalogs.remove'),
            icon: Icon(row.origin == 'replaced' ? Icons.restore : Icons.delete_outline, color: scheme.onSurfaceVariant),
            onPressed: busy ? null : () => unawaited(_remove(context)),
          ),
      ]),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return DecoratedBox(
      decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(6)),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        child: Text(text, style: Theme.of(context).textTheme.labelSmall?.copyWith(color: scheme.onSurfaceVariant)),
      ),
    );
  }
}

/// The dialog that replaces a default (the label is its own) or adds a catalog: the label, the source that reads it among those that
/// serve the type and that source's own form, and what pins it.
class CatalogEntryDialog extends StatefulWidget {
  const CatalogEntryDialog({super.key, required this.sources, required this.type, this.replacing});

  final CatalogSources sources;
  final CatalogType type;

  /// The catalog this one replaces; null to add one.
  final CatalogRow? replacing;

  @override
  State<CatalogEntryDialog> createState() => _CatalogEntryDialogState();
}

class _CatalogEntryDialogState extends State<CatalogEntryDialog> {
  late final _label = TextEditingController(text: widget.replacing?.label ?? '');
  final _pin = TextEditingController();
  /// What each source's form holds, by the source's id; a catalog being replaced starts with what it was read with.
  late final _held = <String, Map<String, Object?>>{
    if (widget.replacing?.spec case final spec?) _idOfSpec(spec): {...spec.fields},
  };
  late String? _chosen = widget.replacing?.spec == null ? null : _idOfSpec(widget.replacing!.spec!);

  /// What tells a generator from a provider of the same id.
  static String _idOf(CatalogSourceForm form) => form.generator ? 'generator:${form.id}' : form.id;
  static String _idOfSpec(CatalogSpec spec) => spec.generator != null ? 'generator:${spec.generator}' : spec.provider;
  String? _problem;
  var _saving = false;
  final _generation = 0;

  @override
  void dispose() {
    _label.dispose();
    _pin.dispose();
    super.dispose();
  }

  static const _read = CatalogSourceForm(
    id: readProvider,
    title: readProvider,
    schema: {
      'type': 'object',
      'required': ['source'],
      'properties': {
        'source': {'type': 'string', 'title': 'Source'},
      },
    },
  );

  List<CatalogSourceForm> get _offered => widget.type.forms.isEmpty ? const [_read] : widget.type.forms;
  CatalogSourceForm get _form => _offered.where((each) => _idOf(each) == _chosen).firstOrNull ?? _offered.first;

  Future<void> _save() async {
    final form = _form;
    final label = _label.text.trim();
    if (label.isEmpty) {
      setState(() => _problem = context.t('catalogs.label_needed'));
      return;
    }
    final values = _held[_idOf(form)] ?? const <String, Object?>{};
    final invalid = schemaProblem(form.schema, form.form, values);
    if (invalid != null) {
      setState(() => _problem = invalid);
      return;
    }
    final kept = collectSchemaValues(form.schema, form.form, values);
    // What was typed is written as the full URI.
    if (kept['source'] case final String typed) kept['source'] = sourceUri(typed, uris: widget.type.readsUris);
    final pin = _pin.text.trim();
    final document = form.id == readProvider && !form.generator;
    final bare = document && pin.isEmpty && kept.length == 1 && kept['source'] is String;
    final entry = bare
        ? kept['source']
        : <String, Object?>{if (form.generator) 'generator': form.id else 'provider': form.id, ...kept, if (pin.isNotEmpty) 'pin': pin};
    setState(() {
      _saving = true;
      _problem = null;
    });
    final why = await widget.sources.set(widget.type.id, label, entry);
    if (!mounted) return;
    if (why == null) {
      Navigator.pop(context);
    } else {
      setState(() {
        _saving = false;
        _problem = why;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final form = _form;
    final replacing = widget.replacing;
    return AlertDialog(
      key: const Key('catalog-entry-dialog'),
      title: Text(replacing == null ? context.t('catalogs.add_title', {'type': widget.type.title}) : context.t('catalogs.replace_title', {'label': replacing.label})),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            TextField(
              key: const Key('catalog-entry-label'),
              controller: _label,
              readOnly: replacing != null,
              decoration: InputDecoration(labelText: context.t('catalogs.label')),
            ),
            if (_offered.length > 1)
              DropdownButton<String>(
                key: const Key('catalog-entry-provider'),
                isExpanded: true,
                value: _idOf(form),
                items: [for (final each in _offered) DropdownMenuItem(value: _idOf(each), child: Text(each.extension == null ? each.title : '${each.title} (${each.extension})'))],
                onChanged: (id) => setState(() {
                  _chosen = id;
                  _problem = null;
                }),
              ),
            if (form.help case final help?) Padding(padding: const EdgeInsets.only(top: 4), child: Text(help)),
            SchemaFields(
              key: ValueKey('${_idOf(form)}/$_generation'),
              schema: form.schema,
              form: form.form,
              values: _held[_idOf(form)] ?? const {},
              idPrefix: 'catalog-entry-${form.id}',
              keys: form.id == readProvider || form.generator ? const {'source': 'catalog-entry-source'} : const {},
              unsetEntry: true,
              onValues: (values) => _held[_idOf(form)] = values,
              onSubmitted: () => unawaited(_save()),
            ),
            TextField(
              key: const Key('catalog-entry-pin'),
              controller: _pin,
              decoration: InputDecoration(labelText: context.t('catalogs.pin'), helperText: context.t('catalogs.pin_help')),
            ),
            if (_problem case final problem?)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(oneLine(problem), key: const Key('catalog-entry-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: _saving ? null : () => Navigator.pop(context), child: Text(context.t('common.cancel'))),
        FilledButton(key: const Key('catalog-entry-save'), onPressed: _saving ? null : () => unawaited(_save()), child: Text(context.t('common.save'))),
      ],
    );
  }
}
