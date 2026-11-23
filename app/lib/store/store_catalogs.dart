import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart' show FilteringTextInputFormatter;

import '../config/catalog_specs.dart';
import '../forms/schema_fields.dart';
import '../loadout/loadout_controller.dart';
import '../settings/setting_layers.dart' show SettingNote;
import '../settings/settings_row.dart';
import '../shell/confirm_remove.dart';
import '../theme/look.dart';
import 'catalog_types.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart';
import 'store_settings.dart';

export '../shell/confirm_remove.dart' show confirmRemoval;

/// Asks before a catalog is taken out of canoa.cfg.
Future<bool> confirmCatalogRemoval(BuildContext context, String label) => confirmRemoval(
  context,
  title: 'Stop reading $label?',
  body: 'It is taken out of canoa.cfg. What was installed from it stays installed, and adding it again reads it once more.',
);

/// Store settings of one type: the catalogs that type reads, added, listed with what each said and removed. What the type calls them
/// and the forms that add one are the host's to say (`store/catalogs`), so a type an extension adds has them too. Where the limits
/// on a catalog are not the defaults, they are named here too.
class StoreSettingsPage extends StatelessWidget {
  const StoreSettingsPage({
    super.key,
    required this.controller,
    required this.type,
    this.title,
    this.loadedDetail = false,
    this.extras = const [],
  });

  final LoadoutController controller;

  /// The id of the Store type whose catalogs these are.
  final String type;

  /// What the type is called until the host says.
  final String? title;

  /// Whether a catalog the controller has read is shown with what it said (its name, its entries, what it refused).
  final bool loadedDetail;

  /// What only this type has beside its catalogs.
  final List<Widget> extras;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final c = controller;
      final text = Theme.of(context).textTheme;
      final known = c.typeOf(type);
      final words = known?.words ?? CatalogWords.generic;
      final rows = known?.rows ?? [for (final MapEntry(:key, :value) in c.config.catalogSpecs(type).entries) CatalogRow(label: key, spec: value)];
      final shownTitle = known?.title ?? title ?? type;
      return Scaffold(
        appBar: AppBar(title: Text('$shownTitle settings')),
        body: ListView(key: const Key('store-settings-page'), padding: const EdgeInsets.all(16), children: [
          Text('${_capital(words.plural)} ${rows.length}', key: const Key('store-settings-heading'), style: text.titleMedium),
          const SizedBox(height: 4),
          Text('Where $shownTitle finds ${words.lists}. Each ${words.noun} below is read by the source it names; what it says of an entry is '
              'its author\'s claim.'),
          if (c.problem != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(oneLine(c.problem!), key: const Key('store-settings-problem'),
                  style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          for (final row in rows)
            if (loadedDetail ? c.loaded.where((each) => each.label == row.label).firstOrNull : null case final catalog?)
              CatalogTile(controller: c, catalog: catalog)
            else
              CatalogRowTile(controller: c, type: type, row: row),
          const SizedBox(height: 8),
          AddCatalogForm(controller: c, type: type, words: words),
          if (extras.isNotEmpty) ...[const SizedBox(height: 24), ...extras],
          const SizedBox(height: 16),
          StoreLimits(controller: c),
        ]),
      );
    },
  );
}

/// How much of a catalog and of an icon the Store reads, which every type's catalogs share: whole numbers written to `[store]`, with the
/// default beside each, and one set back to its default is not written.
class StoreLimits extends StatelessWidget {
  const StoreLimits({super.key, required this.controller});

  final LoadoutController controller;

  static const _limits = [
    ('catalog_max_bytes', 'Largest catalog', 'Bytes read from one catalog file'),
    ('catalog_max_depth', 'Deepest nesting', 'Levels of nesting in a catalog'),
    ('catalog_max_entries', 'Most entries', 'Entries one catalog may list'),
    ('catalog_max_icons', 'Most icons', 'Distinct icon files read for one catalog'),
    ('icon_max_bytes', 'Largest icon', 'Bytes of one icon file'),
    ('icon_max_width', 'Widest icon', 'Pixels'),
    ('icon_max_height', 'Tallest icon', 'Pixels'),
    ('contents_ttl_seconds', 'Contents kept for', 'Seconds what was read of every entry stays current; 0 reads again at every refresh'),
  ];

  static int _value(StoreSettings s, String key) => switch (key) {
    'catalog_max_bytes' => s.catalogMaxBytes,
    'catalog_max_depth' => s.catalogMaxDepth,
    'catalog_max_entries' => s.catalogMaxEntries,
    'catalog_max_icons' => s.catalogMaxIcons,
    'icon_max_bytes' => s.iconMaxBytes,
    'icon_max_width' => s.iconMaxWidth,
    'contents_ttl_seconds' => s.contentsTtlSeconds,
    _ => s.iconMaxHeight,
  };

  @override
  Widget build(BuildContext context) => SettingsGroup(title: 'Limits', children: [
    for (final (key, title, description) in _limits)
      SettingsRow(
        title: title,
        description: '$description. Default ${_value(const StoreSettings(), key)}',
        configKey: ['store', key],
        layerConfig: controller.config,
        onLayerReset: () => unawaited(controller.apply(controller.config.resetKey(['store', key]))),
        control: _LimitField(
          key: Key('limit-$key'),
          value: _value(controller.config.store, key),
          onChanged: (n) => unawaited(controller.apply(controller.config.withStore(controller.config.store.withLimit(key, n)))),
        ),
      ),
  ]);
}

/// A whole number, 0 or more, applied as it is typed once it reads as one.
class _LimitField extends StatefulWidget {
  const _LimitField({super.key, required this.value, required this.onChanged});

  final int value;
  final ValueChanged<int> onChanged;

  @override
  State<_LimitField> createState() => _LimitFieldState();
}

class _LimitFieldState extends State<_LimitField> {
  late final _text = TextEditingController(text: '${widget.value}');

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SizedBox(
    width: 140,
    child: TextField(
      controller: _text,
      keyboardType: TextInputType.number,
      inputFormatters: [FilteringTextInputFormatter.digitsOnly],
      textAlign: TextAlign.end,
      style: const TextStyle(fontFamily: CanoaLook.monospaceFont),
      decoration: const InputDecoration(isDense: true, border: OutlineInputBorder()),
      onChanged: (text) {
        final n = int.tryParse(text);
        if (n != null) widget.onChanged(n);
      },
    ),
  );
}

String _capital(String word) => word.isEmpty ? word : '${word[0].toUpperCase()}${word.substring(1)}';

/// One catalog of a type as canoa.cfg names it, with the source that reads it and the button that removes it.
class CatalogRowTile extends StatelessWidget {
  const CatalogRowTile({super.key, required this.controller, required this.type, required this.row});

  final LoadoutController controller;
  final String type;
  final CatalogRow row;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final provider = row.spec?.provider;
    final titled = provider == null ? null : c.typeOf(type)?.sources.where((source) => source.id == provider).firstOrNull?.title;
    return ListTile(
      key: Key('store-catalog-${row.label}'),
      contentPadding: EdgeInsets.zero,
      title: Text(row.label),
      subtitle: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text([
          if (row.shown.isNotEmpty) row.shown,
          if (provider != null && provider != readProvider) 'Read by ${titled ?? provider}',
          ?row.problem,
        ].map(oneLine).join('\n')),
        SettingNote(
          configKey: ['store', type, 'catalogs', row.label],
          config: c.config,
          onReset: () => unawaited(c.apply(c.config.resetKey(['store', type, 'catalogs', row.label]))),
        ),
      ]),
      trailing: row.fixed
          ? null
          : TextButton(
              key: Key('store-remove-catalog-${row.label}'),
              onPressed: c.busy != null
                  ? null
                  : () async {
                      if (await confirmCatalogRemoval(context, row.label)) await c.removeCatalogOf(type, row.label);
                    },
              child: const Text('Remove'),
            ),
    );
  }
}

/// The form that adds a catalog to a type: the label, the source to read it with among those that serve the type, and that
/// source's own fields as it described them.
class AddCatalogForm extends StatefulWidget {
  const AddCatalogForm({super.key, required this.controller, required this.type, required this.words});

  final LoadoutController controller;
  final String type;
  final CatalogWords words;

  @override
  State<AddCatalogForm> createState() => _AddCatalogFormState();
}

/// The `read` provider's form, for a host that has not said what it offers: an address or a file.
const _readForm = CatalogSourceForm(
  id: readProvider,
  title: 'Read an address or a file',
  schema: {
    'type': 'object',
    'required': ['source'],
    'properties': {
      'source': {'type': 'string', 'title': 'Source'},
    },
  },
);

class _AddCatalogFormState extends State<AddCatalogForm> {
  final label = TextEditingController();
  /// What each source's form holds, by the source's id, and which generation of its fields is shown: a new one clears them.
  final held = <String, Map<String, Object?>>{};
  int generation = 0;
  String? chosen;
  String? problem;

  @override
  void dispose() {
    label.dispose();
    super.dispose();
  }

  List<CatalogSourceForm> get offered {
    final given = widget.controller.typeOf(widget.type)?.sources ?? const [];
    return given.isEmpty ? const [_readForm] : given;
  }

  CatalogSourceForm get form => offered.where((each) => each.id == chosen).firstOrNull ?? offered.first;

  Future<void> _add() async {
    final form = this.form;
    final values = held[form.id] ?? const <String, Object?>{};
    final invalid = schemaProblem(form.schema, form.form, values);
    if (invalid != null) {
      setState(() => problem = invalid);
      return;
    }
    final kept = collectSchemaValues(form.schema, form.form, values);
    final why = await widget.controller.addCatalogOf(widget.type, label.text, form.id, kept);
    if (!mounted) return;
    setState(() => problem = why);
    if (why == null) {
      label.clear();
      setState(() {
        held.clear();
        generation++;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final form = this.form;
    final c = widget.controller;
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Add a ${widget.words.noun}', style: Theme.of(context).textTheme.titleSmall),
      const SizedBox(height: 8),
      Wrap(spacing: 8, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.end, children: [
        SizedBox(
          width: 160,
          child: TextField(key: const Key('store-catalog-label'), controller: label, decoration: const InputDecoration(labelText: 'Label, optional')),
        ),
        if (offered.length > 1)
          DropdownButton<String>(
            key: const Key('store-source-provider'),
            value: form.id,
            items: [
              for (final each in offered)
                DropdownMenuItem(value: each.id, child: Text(each.extension == null ? each.title : '${each.title} (${each.extension})')),
            ],
            onChanged: (id) => setState(() {
              chosen = id;
              problem = null;
            }),
          ),
      ]),
      if (form.help case final help?) Padding(padding: const EdgeInsets.only(top: 4), child: Text(help)),
      SchemaFields(
        key: ValueKey('${form.id}/$generation'),
        schema: form.schema,
        form: form.form,
        values: held[form.id] ?? const {},
        idPrefix: 'store-${form.id}',
        keys: form.id == readProvider ? const {'source': 'store-catalog-source'} : const {},
        unsetEntry: true,
        onValues: (values) => held[form.id] = values,
        onSubmitted: () => unawaited(_add()),
      ),
      if (problem != null)
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text(oneLine(problem!), key: const Key('store-add-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
      const SizedBox(height: 8),
      FilledButton(
        key: const Key('store-add-catalog'),
        onPressed: c.busy != null ? null : () => unawaited(_add()),
        child: Text('Add ${widget.words.noun}'),
      ),
    ]);
  }
}

/// One catalog as it was read, and the button that removes it.
class CatalogTile extends StatelessWidget {
  const CatalogTile({super.key, required this.controller, required this.catalog});

  final LoadoutController controller;
  final LoadedCatalog catalog;

  @override
  Widget build(BuildContext context) {
    final read = catalog.catalog;
    final c = controller;
    final lines = [
      displaySource(aos.Source('git', catalog.source.repo, catalog.source.reference, catalog.source.path)),
      if (catalog.commit case final commit?) 'At commit $commit',
      ?catalog.problem,
      if (read != null) ...[
        if (read.name case final name?) 'It calls itself $name',
        '${read.entries.length} overlays and ${read.skills.length} skills listed${read.refused.isEmpty ? '' : ', ${read.refused.length} refused'}',
        if (catalog.leftOut > 0) '${catalog.leftOut} more left out, past [store] catalog_max_entries = ${c.config.store.catalogMaxEntries}',
        for (final (name, why) in read.refused) '  refused $name: $why',
        for (final pointer in read.ignored) '  left out $pointer',
        for (final MapEntry(key: name, value: icon) in catalog.icons.entries)
          if (icon.refused case final why?) '  icon of $name refused: $why',
      ],
    ];
    return ListTile(
      key: Key('store-catalog-${catalog.label}'),
      contentPadding: EdgeInsets.zero,
      title: Text(catalog.label),
      subtitle: Text(lines.map(oneLine).join('\n')),
      trailing: TextButton(
        key: Key('store-remove-catalog-${catalog.label}'),
        onPressed: c.busy != null
            ? null
            : () async {
                if (await confirmCatalogRemoval(context, catalog.label)) await c.removeCatalog(catalog.label);
              },
        child: const Text('Remove'),
      ),
    );
  }
}

/// What a person with no catalog sees: where to find some, and how to add one, never a bare repository field.
class NoCatalogs extends StatelessWidget {
  const NoCatalogs({super.key, required this.controller});

  final LoadoutController controller;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final c = controller;
    return Column(key: const Key('catalogs-empty'), crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Find overlays and skills', style: text.titleMedium),
      const SizedBox(height: 4),
      const Text('A catalog is a list of overlays and skills kept in a git repository. Add one to browse what it lists, '
          'read what each would do, and install it.'),
      const SizedBox(height: 12),
      Text('Catalogs to start from', style: text.titleSmall),
      for (final known in knownCatalogs)
        Card(
          key: Key('known-catalog-${known.label}'),
          child: ListTile(
            title: Text(known.label),
            subtitle: Text('${known.about}\n${known.spec.split('@').first.replaceAll('.git', '')}'),
            isThreeLine: true,
            trailing: FilledButton.tonal(
              key: Key('known-catalog-add-${known.label}'),
              onPressed: c.busy != null ? null : () => unawaited(c.addCatalog(known.label, known.spec)),
              child: const Text('Add'),
            ),
          ),
        ),
      const SizedBox(height: 12),
      Text('Or paste a catalog', style: text.titleSmall),
      const SizedBox(height: 4),
      const Text('A git repository, a branch or tag, and the catalog file in it: https://github.com/ana/overlays.git@main#catalog.toml'),
      const SizedBox(height: 8),
      AddCatalogForm(controller: c, type: 'overlays', words: c.typeOf('overlays')?.words ?? CatalogWords.generic),
    ]);
  }
}

/// The Skills type's own setting beside its sources: whether the Skills page may ask skills.sh.
class SkillsSearchSwitch extends StatelessWidget {
  const SkillsSearchSwitch({super.key, required this.controller});

  final LoadoutController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final c = controller;
      final address = c.skillsSearch;
      return SwitchListTile(
        key: const Key('store-skills-search'),
        contentPadding: EdgeInsets.zero,
        title: const Text('Search skills.sh'),
        subtitle: Text(
          "Off until you turn it on. skills.sh is a third party's directory of skills, and its search is not documented. While this is on, "
          'Skills, Discover can send it the words you search for, and nothing else. A result only says which repository holds a skill: '
          'installing it reads that repository with git like any source, and asks first.'
          '${address == null || address == c.defaultSkillsSearchAddress ? '' : ' Asking $address, as canoa.cfg says.'}',
        ),
        value: c.skillsSearch != null,
        onChanged: c.busy != null ? null : (on) => unawaited(c.setSkillsSearch(on)),
      );
    },
  );
}
