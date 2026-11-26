import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import '../l10n/app_text.dart';
import '../selectable.dart';
import 'store_banner.dart';
import 'store_cards.dart';
import 'store_icon.dart';
import 'store_selection.dart' show StoreMarkImage;
import 'store_update_all.dart';
import 'store_view.dart';

/// One item of a store type an extension adds: what its tile shows, the catalog that lists it and what + does.
final class ExtensionStoreItem {
  const ExtensionStoreItem({required this.catalog, required this.name, required this.by, this.summary, this.tags = const [],
      this.installed = false, this.verified, this.install, this.remove, this.leading, this.details});

  final String catalog, name, by;
  final String? summary;

  /// The algorithm the digests of its files are made with, when every file has one the download is checked against; the tile then
  /// shows a check mark that says so.
  final String? verified;
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

/// What a store type says in place of items when it cannot list any: the host could not read it (a problem, with a retry) or the
/// program it lists is not there (quieter, with the action that fixes it when there is one). It is drawn above the items and is never
/// one of them.
final class ExtensionStoreNotice {
  const ExtensionStoreNotice({required this.id, required this.words, this.problem = false, this.retry, this.actionLabel, this.action});

  /// Names the notice in keys; the id of the type's library.
  final String id;

  /// What it says, from the person's text (the host's or the library's own words go in as they are).
  final String Function(AppText text) words;

  /// Whether it is an error (drawn in the error colour, with Retry) rather than a note.
  final bool problem;

  /// Asks the host again; null leaves the notice without a Retry.
  final Future<void> Function()? retry;

  /// The label of the action that fixes what it says, and what the action does given the context its dialogs are shown in.
  final String Function(AppText text)? actionLabel;
  final Future<void> Function(BuildContext context)? action;
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
    this.notices,
    this.updates,
  });

  /// The items that hold an update the Store can apply, which Update all plans and applies together; null when the type has none.
  final List<UpdateCandidate> Function(AppText text)? updates;

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

  /// What the type says besides its items, asked when the page is drawn: a listing that failed, a program that is not there.
  final List<ExtensionStoreNotice> Function()? notices;
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

/// A page with more items than this has a row of chips that filter it by the tags its items carry.
const _chipsFrom = 8;

/// The tags of [items], each with how many carry it, the most common first.
List<MapEntry<String, int>> _tagCounts(List<ExtensionStoreItem> items) {
  final counts = <String, int>{};
  for (final item in items) {
    for (final tag in item.tags.toSet()) {
      if (tag.isNotEmpty) counts[tag] = (counts[tag] ?? 0) + 1;
    }
  }
  return counts.entries.toList()
    ..sort((a, b) => b.value != a.value ? b.value.compareTo(a.value) : a.key.compareTo(b.key));
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
        if (_tagCounts(inScope) case final counts when counts.length > 1 && inScope.length > _chipsFrom)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Wrap(spacing: 6, runSpacing: 4, children: [
              for (final MapEntry(key: tag, value: count) in counts)
                FilterChip(
                  key: Key('store-tag-${type.id}-$tag'),
                  label: Text('$tag ($count)'),
                  visualDensity: VisualDensity.compact,
                  selected: view.tags.contains(tag),
                  onSelected: (on) {
                    if (on) {
                      view.tags.add(tag);
                    } else {
                      view.tags.remove(tag);
                    }
                    view.changed();
                  },
                ),
            ]),
          ),
        for (final notice in type.notices?.call() ?? const <ExtensionStoreNotice>[]) _NoticeRow(notice: notice),
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
            verified: item.verified,
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

/// One notice of a type: its words, in the error colour when it is a problem, with the Retry and the action it has.
class _NoticeRow extends StatefulWidget {
  const _NoticeRow({required this.notice});

  final ExtensionStoreNotice notice;

  @override
  State<_NoticeRow> createState() => _NoticeRowState();
}

class _NoticeRowState extends State<_NoticeRow> {
  bool _busy = false;

  Future<void> _do(Future<void> Function() what) async {
    setState(() => _busy = true);
    try {
      await what();
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final notice = widget.notice;
    final text = AppText.of(context);
    final scheme = Theme.of(context).colorScheme;
    final color = notice.problem ? scheme.error : scheme.onSurfaceVariant;
    final retry = notice.retry;
    final action = notice.action;
    final label = notice.actionLabel;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(notice.problem ? Icons.error_outline : Icons.info_outline, size: 18, color: color),
          const SizedBox(width: 8),
          Expanded(child: SelectableText(notice.words(text), key: Key('store-type-notice-${notice.id}'), style: TextStyle(color: color))),
          if (retry != null)
            TextButton(key: Key('store-type-notice-retry-${notice.id}'), onPressed: _busy ? null : () => unawaited(_do(retry)), child: Text(text('store.retry'))),
          if (action != null && label != null)
            TextButton(
              key: Key('store-type-notice-action-${notice.id}'),
              onPressed: _busy ? null : () => unawaited(_do(() => action(context))),
              child: Text(label(text)),
            ),
        ],
      ),
    );
  }
}
