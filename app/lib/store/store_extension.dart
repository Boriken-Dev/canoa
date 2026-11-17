import 'dart:async';

import 'package:flutter/material.dart';

import 'store_banner.dart';
import 'store_cards.dart';
import 'store_icon.dart';
import 'store_view.dart';

/// One item of a store type an extension adds: what its tile shows, the catalog that lists it and what + does.
final class ExtensionStoreItem {
  const ExtensionStoreItem({required this.catalog, required this.name, required this.by, this.summary, this.tags = const [],
      this.installed = false, this.install});

  final String catalog, name, by;
  final String? summary;
  final List<String> tags;
  final bool installed;

  /// What + does, given the context a confirmation dialog is shown in; null leaves the tile without one that works.
  final Future<void> Function(BuildContext context)? install;
}

/// A store type an extension adds, listed in the Store's index after the built-in ones. Its page is the same as theirs: a banner,
/// "Top …" and a tile for each item with a +, and a filter for the catalogs the items come from.
final class ExtensionStoreType {
  const ExtensionStoreType({
    required this.id,
    required this.label,
    required this.items,
    this.icon = Icons.extension_outlined,
    this.changes,
    this.load,
  });

  /// A word of letters and digits that no built-in type uses; the index keeps its choice by it.
  final String id;

  /// The type's name as the index and the page's title say it, in the plural.
  final String label;
  final IconData icon;

  /// The items now, asked when the page is drawn.
  final List<ExtensionStoreItem> Function() items;

  /// Tells when [items] changed, so the page and the index's counts are drawn again.
  final Listenable? changes;

  /// Asks where the items come from; run when the page opens.
  final Future<void> Function()? load;
}

/// The page of a store type an extension adds.
class ExtensionBody extends StatefulWidget {
  const ExtensionBody({super.key, required this.type, required this.view});

  final ExtensionStoreType type;
  final StoreView view;

  @override
  State<ExtensionBody> createState() => _ExtensionBodyState();
}

class _ExtensionBodyState extends State<ExtensionBody> with AutomaticKeepAliveClientMixin {
  final _installing = <String>{};

  @override
  bool get wantKeepAlive => true;

  @override
  void initState() {
    super.initState();
    final load = widget.type.load;
    if (load != null) unawaited(load());
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final changes = widget.type.changes;
    return changes == null ? _page(context) : ListenableBuilder(listenable: changes, builder: (context, _) => _page(context));
  }

  Widget _page(BuildContext context) {
    final view = widget.view;
    final type = widget.type;
    final all = type.items();
    final inScope = [for (final item in all) if (view.catalogs.isEmpty || view.catalogs.contains(item.catalog)) item];
    final shown = [
      for (final item in inScope)
        if (view.matches('${item.name}\n${item.summary ?? ''}\n${item.by}') && view.tags.every(item.tags.contains)) item,
    ];
    final noun = type.label.toLowerCase();
    final text = Theme.of(context).textTheme;
    final filtered = view.tags.isNotEmpty || view.query.text.isNotEmpty;
    return CardList(
      key: Key('store-type-${type.id}'),
      header: [
        StoreBanner(
          specs: bannersFor(
            noun: noun,
            title: type.label,
            scope: const [],
            items: [
              for (final item in inScope)
                BannerItem(name: item.name, catalog: item.catalog, tags: item.tags, open: () {}),
            ],
            showTag: (tag) {
              view.tags
                ..clear()
                ..add(tag);
              view.changed();
            },
          ),
          iconLimits: null,
          mark: type.icon,
        ),
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Row(children: [
            Text(filtered ? 'Matching $noun' : 'Top $noun', style: text.titleMedium),
            const SizedBox(width: 8),
            Chip(key: const Key('store-count'), label: Text('${shown.length}'), visualDensity: VisualDensity.compact),
            const Spacer(),
            if (filtered)
              TextButton(
                key: const Key('store-show-all'),
                onPressed: () {
                  view.tags.clear();
                  view.query.clear();
                  view.changed();
                },
                child: const Text('Show all →'),
              ),
          ]),
        ),
        if (shown.isEmpty) Text(filtered ? 'Nothing listed matches.' : 'No catalog lists any $noun.', key: const Key('store-type-nothing')),
      ],
      cards: [
        for (final item in shown)
          StoreTile(
            key: Key('store-entry-${item.catalog}-${item.name}'),
            leading: StillIcon(name: item.name, path: null, bytes: null, size: 48),
            title: item.name,
            summary: item.summary,
            by: item.by,
            installed: item.installed,
            busy: _installing.contains('${item.catalog}/${item.name}'),
            installKey: Key('store-entry-install-${item.catalog}-${item.name}'),
            onOpen: () {},
            onInstall: item.install == null ? null : () => unawaited(_install(item)),
          ),
      ],
    );
  }

  Future<void> _install(ExtensionStoreItem item) async {
    final key = '${item.catalog}/${item.name}';
    setState(() => _installing.add(key));
    try {
      await item.install!(context);
    } finally {
      if (mounted) setState(() => _installing.remove(key));
    }
  }
}
