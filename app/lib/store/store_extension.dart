import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../selectable.dart';
import 'store_banner.dart';
import 'store_cards.dart';
import 'store_icon.dart';
import 'store_selection.dart' show StoreMarkImage;
import 'store_view.dart';

/// One item of a store type an extension adds: what its tile shows, the catalog that lists it and what + does.
final class ExtensionStoreItem {
  const ExtensionStoreItem({required this.catalog, required this.name, required this.by, this.summary, this.tags = const [],
      this.installed = false, this.install, this.remove, this.leading, this.details});

  final String catalog, name, by;
  final String? summary;
  final List<String> tags;
  final bool installed;

  /// What + does, given the context a confirmation dialog is shown in; null leaves the tile without one that works.
  final Future<void> Function(BuildContext context)? install;

  /// What the tile's remove button does, once the person confirms the host's plan; null when the host holds no copy of its own to
  /// take away.
  final Future<void> Function(BuildContext context)? remove;

  /// The icon on its tile; null draws the first letter of its name on a colour.
  final Widget? leading;

  /// The page the tile opens, given canoa.cfg as the Store has it; null leaves the tile opening nothing.
  final Widget Function(BuildContext context, CanoaConfig? config)? details;
}

/// A store type an extension adds, listed in the Store's index after the built-in ones. Its page is the same as theirs: a banner,
/// "Top …" and a tile for each item with a +, and a filter for the catalogs the items come from.
final class ExtensionStoreType {
  const ExtensionStoreType({
    required this.id,
    required this.label,
    required this.items,
    this.icon = Icons.extension_outlined,
    this.image,
    this.changes,
    this.load,
  });

  /// A word of letters and digits that no built-in type uses; the index keeps its choice by it.
  final String id;

  /// The type's name as the index and the page's title say it, in the plural.
  final String label;
  final IconData icon;

  /// A picture the extension ships, drawn in place of [icon].
  final StoreMarkImage? image;

  /// The items now, asked when the page is drawn.
  final List<ExtensionStoreItem> Function() items;

  /// Tells when [items] changed, so the page and the index's counts are drawn again.
  final Listenable? changes;

  /// Asks where the items come from; run when the page opens.
  final Future<void> Function()? load;
}

/// The page of a store type an extension adds.
class ExtensionBody extends StatefulWidget {
  const ExtensionBody({super.key, required this.type, required this.view, this.config});

  final ExtensionStoreType type;
  final StoreView view;

  /// canoa.cfg as the Store holds it, for the pages items open.
  final CanoaConfig? config;

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
          markImage: type.image,
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
            leading: item.leading ?? StillIcon(name: item.name, path: null, bytes: null, size: 48),
            title: item.name,
            summary: item.summary,
            by: item.by,
            installed: item.installed,
            busy: _installing.contains('${item.catalog}/${item.name}'),
            installKey: Key('store-entry-install-${item.catalog}-${item.name}'),
            onOpen: item.details == null
                ? () {}
                : () => unawaited(Navigator.of(context).push<void>(selectablePage(builder: (page) => item.details!(page, widget.config)))),
            onInstall: item.install == null ? null : () => unawaited(_install(item)),
            removeKey: Key('store-entry-remove-${item.catalog}-${item.name}'),
            onRemove: item.remove == null ? null : () => unawaited(_run(item, item.remove!)),
          ),
      ],
    );
  }

  Future<void> _install(ExtensionStoreItem item) => _run(item, item.install!);

  Future<void> _run(ExtensionStoreItem item, Future<void> Function(BuildContext context) action) async {
    final key = '${item.catalog}/${item.name}';
    setState(() => _installing.add(key));
    try {
      await action(context);
    } finally {
      if (mounted) setState(() => _installing.remove(key));
    }
  }
}
