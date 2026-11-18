import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../config/catalog_specs.dart';
import '../loadout/loadout_controller.dart';
import 'catalog_types.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart';

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
          for (final nonDefault in c.config.store.nonDefaults) Text('[store] $nonDefault', style: text.bodySmall),
        ]),
      );
    },
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
      subtitle: Text([
        if (row.shown.isNotEmpty) row.shown,
        if (provider != null && provider != readProvider) 'Read by ${titled ?? provider}',
        ?row.problem,
      ].map(oneLine).join('\n')),
      trailing: row.fixed
          ? null
          : TextButton(
              key: Key('store-remove-catalog-${row.label}'),
              onPressed: c.busy != null ? null : () => unawaited(c.removeCatalogOf(type, row.label)),
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
  fields: [SourceField(name: 'source', label: 'Source', shape: 'text', required: true)],
);

class _AddCatalogFormState extends State<AddCatalogForm> {
  final label = TextEditingController();
  final fields = <String, TextEditingController>{};
  String? chosen;
  String? problem;

  @override
  void dispose() {
    label.dispose();
    for (final controller in fields.values) {
      controller.dispose();
    }
    super.dispose();
  }

  List<CatalogSourceForm> get offered {
    final given = widget.controller.typeOf(widget.type)?.sources ?? const [];
    return given.isEmpty ? const [_readForm] : given;
  }

  CatalogSourceForm get form => offered.where((each) => each.id == chosen).firstOrNull ?? offered.first;

  TextEditingController _controllerOf(CatalogSourceForm form, String name) => fields.putIfAbsent('${form.id}/$name', TextEditingController.new);

  Future<void> _add() async {
    final form = this.form;
    final values = <String, Object?>{};
    for (final field in form.fields) {
      final text = _controllerOf(form, field.name).text.trim();
      if (text.isEmpty) {
        if (field.required) {
          setState(() => problem = '${field.label} is needed');
          return;
        }
        continue;
      }
      switch (field.shape) {
        case 'number':
          final number = int.tryParse(text);
          if (number == null || number < 0) {
            setState(() => problem = '${field.label} is a whole number');
            return;
          }
          values[field.name] = number;
        case 'list':
          values[field.name] = [for (final line in text.split('\n')) if (line.trim().isNotEmpty) line.trim()];
        default:
          values[field.name] = text;
      }
    }
    final why = await widget.controller.addCatalogOf(widget.type, label.text, form.id, values);
    if (!mounted) return;
    setState(() => problem = why);
    if (why == null) {
      label.clear();
      for (final each in fields.values) {
        each.clear();
      }
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
      for (final field in form.fields)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: SizedBox(
            width: 520,
            child: TextField(
              key: Key(form.id == readProvider && field.name == 'source' ? 'store-catalog-source' : 'store-field-${form.id}-${field.name}'),
              controller: _controllerOf(form, field.name),
              minLines: 1,
              maxLines: field.shape == 'list' ? 4 : 1,
              decoration: InputDecoration(
                labelText: field.required ? field.label : '${field.label}, optional',
                hintText: field.defaultText,
                helperText: field.shape == 'list' ? 'One word a line' : field.help,
                helperMaxLines: 3,
              ),
              onSubmitted: field.shape == 'list' ? null : (_) => unawaited(_add()),
            ),
          ),
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
        onPressed: c.busy != null ? null : () => unawaited(c.removeCatalog(catalog.label)),
        child: const Text('Remove'),
      ),
    );
  }
}

/// The label and source fields and the button that adds a catalog.
class CatalogAddForm extends StatefulWidget {
  const CatalogAddForm({super.key, required this.controller});

  final LoadoutController controller;

  @override
  State<CatalogAddForm> createState() => _CatalogAddFormState();
}

class _CatalogAddFormState extends State<CatalogAddForm> {
  final label = TextEditingController();
  final spec = TextEditingController();

  @override
  void dispose() {
    label.dispose();
    spec.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    await widget.controller.addCatalog(label.text, spec.text);
    label.clear();
    spec.clear();
  }

  @override
  Widget build(BuildContext context) => Wrap(spacing: 8, runSpacing: 8, crossAxisAlignment: WrapCrossAlignment.end, children: [
    SizedBox(
      width: 160,
      child: TextField(key: const Key('store-catalog-label'), controller: label,
          decoration: const InputDecoration(labelText: 'Label, optional')),
    ),
    SizedBox(
      width: 420,
      child: TextField(key: const Key('store-catalog-source'), controller: spec,
          decoration: const InputDecoration(labelText: 'Source', hintText: 'https://github.com/ana/overlays.git@main#catalog.toml')),
    ),
    FilledButton(
      key: const Key('store-add-catalog'),
      onPressed: widget.controller.busy != null ? null : () => unawaited(_add()),
      child: const Text('Add catalog'),
    ),
  ]);
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
      CatalogAddForm(controller: c),
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
          '${address == null || address == CanoaConfig.defaultSkillsSearch ? '' : ' Asking $address, as canoa.cfg says.'}',
        ),
        value: c.skillsSearch != null,
        onChanged: c.busy != null ? null : (on) => unawaited(c.setSkillsSearch(on)),
      );
    },
  );
}
