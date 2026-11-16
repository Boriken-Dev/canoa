import 'dart:async';

import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../loadout/loadout_controller.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart';

/// Store settings: the catalogs the Store reads, added, listed with what each said and removed. Where the limits on a catalog
/// are not the defaults, they are named here too.
class StoreSettingsPage extends StatelessWidget {
  const StoreSettingsPage({super.key, required this.controller});

  final LoadoutController controller;

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Store settings')),
    body: ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final c = controller;
        final text = Theme.of(context).textTheme;
        return ListView(key: const Key('store-settings-page'), padding: const EdgeInsets.all(16), children: [
          Text('Catalogs ${c.config.catalogs.length}', style: text.titleMedium),
          const SizedBox(height: 4),
          const Text('A catalog lists overlays, and skills, by name and source. What it says of an overlay is its author\'s '
              'claim; an overlay\'s page shows what the overlay itself does once fetched.'),
          if (c.problem != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(oneLine(c.problem!), key: const Key('store-settings-problem'),
                  style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          for (final catalog in c.loaded) CatalogTile(controller: c, catalog: catalog),
          const SizedBox(height: 8),
          CatalogAddForm(controller: c),
          const SizedBox(height: 16),
          for (final nonDefault in c.config.store.nonDefaults) Text('[store] $nonDefault', style: text.bodySmall),
        ]);
      },
    ),
  );
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
