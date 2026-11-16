import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart';
import 'store_icon.dart';

/// What one banner at the top of an opened store says: a line about whose it is, a title, a sentence, the icons of the items it
/// features and what Explore does.
final class BannerSpec {
  const BannerSpec({required this.kicker, required this.title, required this.about, required this.featured, required this.onExplore,
      this.exploreKey});

  final String kicker, title, about;
  final List<ListedEntry> featured;
  final VoidCallback onExplore;
  final Key? exploreKey;
}

/// The banners of an opened store, from its own catalog's data: its name, its description and what it features, then one for each
/// of its most used tags. With nothing featured the first banner is the store's name and mark alone.
List<BannerSpec> bannersFor({
  required LoadedCatalog catalog,
  required List<ListedEntry> entries,
  required void Function(ListedEntry) open,
  required void Function(String tag) showTag,
}) {
  final named = catalog.catalog?.name;
  final description = catalog.catalog?.description;
  final featured = [
    for (final listed in entries)
      if ((listed.entry.about['tags'] as List?)?.contains('featured') ?? false) listed,
  ];
  final lead = (featured.isEmpty ? entries : featured).take(6).toList();
  final specs = <BannerSpec>[
    BannerSpec(
      kicker: 'A store of Canoa',
      title: oneLine(named ?? catalog.label),
      about: oneLine(description ?? '${entries.length} overlays, as ${catalog.label} lists them.'),
      featured: lead,
      onExplore: () {
        if (lead.isNotEmpty) open(lead.first);
      },
      exploreKey: const Key('store-banner-explore'),
    ),
  ];
  final uses = <String, List<ListedEntry>>{};
  for (final listed in entries) {
    for (final tag in {for (final tag in (listed.entry.about['tags'] as List?) ?? const []) '$tag'}) {
      if (tag != 'featured') (uses[tag] ??= []).add(listed);
    }
  }
  final tags = uses.keys.where((tag) => uses[tag]!.length >= 2).toList()
    ..sort((a, b) => uses[a]!.length != uses[b]!.length ? uses[b]!.length - uses[a]!.length : a.compareTo(b));
  for (final tag in tags.take(2)) {
    specs.add(BannerSpec(
      kicker: 'From ${oneLine(catalog.label)}',
      title: oneLine(tag),
      about: '${uses[tag]!.length} overlays carry this tag.',
      featured: uses[tag]!.take(6).toList(),
      onExplore: () => showTag(tag),
      exploreKey: Key('store-banner-explore-$tag'),
    ));
  }
  return specs;
}

/// The wide card at the top of an opened store: a soft gradient of the look's own accent, a small line, a title, a sentence, the
/// icons it features and Explore. Arrows page through several on a wide window, with dots under; a narrow one swipes.
class StoreBanner extends StatefulWidget {
  const StoreBanner({super.key, required this.specs, required this.catalog, required this.iconLimits});

  final List<BannerSpec> specs;
  final LoadedCatalog catalog;
  final aos.Settings? iconLimits;

  @override
  State<StoreBanner> createState() => _StoreBannerState();
}

class _StoreBannerState extends State<StoreBanner> {
  final _pages = PageController();
  int _page = 0;

  @override
  void dispose() {
    _pages.dispose();
    super.dispose();
  }

  void _go(int to) => _pages.animateToPage(to, duration: const Duration(milliseconds: 250), curve: Curves.easeOut);

  @override
  Widget build(BuildContext context) => LayoutBuilder(builder: (context, box) => _build(context, box.maxWidth < 560));

  Widget _build(BuildContext context, bool narrow) {
    final scheme = Theme.of(context).colorScheme;
    final specs = widget.specs;
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Column(children: [
        Container(
          key: const Key('store-banner'),
          height: 176,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(16),
            gradient: LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              colors: [
                Color.alphaBlend(scheme.primary.withValues(alpha: 0.22), scheme.surfaceContainerHighest),
                Color.alphaBlend(scheme.primary.withValues(alpha: 0.05), scheme.surfaceContainerHighest),
              ],
            ),
          ),
          child: Row(children: [
            if (!narrow)
              IconButton(
                key: const Key('store-banner-previous'),
                tooltip: 'Previous',
                onPressed: _page > 0 ? () => _go(_page - 1) : null,
                icon: const Icon(Icons.chevron_left),
              ),
            Expanded(
              child: PageView(
                controller: _pages,
                onPageChanged: (page) => setState(() => _page = page),
                children: [for (final spec in specs) _slide(context, spec, narrow)],
              ),
            ),
            if (!narrow)
              IconButton(
                key: const Key('store-banner-next'),
                tooltip: 'Next',
                onPressed: _page < specs.length - 1 ? () => _go(_page + 1) : null,
                icon: const Icon(Icons.chevron_right),
              ),
          ]),
        ),
        if (specs.length > 1)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Row(mainAxisAlignment: MainAxisAlignment.center, children: [
              for (var i = 0; i < specs.length; i++)
                Container(
                  key: Key('store-banner-dot-$i'),
                  width: 8,
                  height: 8,
                  margin: const EdgeInsets.symmetric(horizontal: 3),
                  decoration: BoxDecoration(shape: BoxShape.circle, color: i == _page ? scheme.primary : scheme.outlineVariant),
                ),
            ]),
          ),
      ]),
    );
  }

  Widget _slide(BuildContext context, BannerSpec spec, bool narrow) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: narrow ? 16 : 8, vertical: 14),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisAlignment: MainAxisAlignment.center, children: [
        Text(spec.kicker.toUpperCase(), style: text.labelSmall?.copyWith(letterSpacing: 1.2, color: scheme.onSurfaceVariant)),
        const SizedBox(height: 4),
        Text(spec.title, style: text.headlineSmall, maxLines: 1, overflow: TextOverflow.ellipsis),
        const SizedBox(height: 2),
        Text(spec.about, style: text.bodyMedium, maxLines: 2, overflow: TextOverflow.ellipsis),
        const Spacer(),
        Row(children: [
          if (spec.featured.isEmpty)
            const Icon(Icons.storefront_outlined, size: 32)
          else
            for (final listed in spec.featured.take(narrow ? 4 : 6))
              Padding(
                padding: const EdgeInsets.only(right: 6),
                child: StillIcon(
                  name: listed.entry.name,
                  path: listed.entry.about['icon'] as String?,
                  bytes: widget.catalog.icons[listed.entry.name]?.bytes,
                  size: 32,
                  limits: widget.iconLimits,
                ),
              ),
          const Spacer(),
          FilledButton.tonal(key: spec.exploreKey, onPressed: spec.onExplore, child: const Text('Explore')),
        ]),
      ]),
    );
  }
}

/// An item of an opened store: a rounded icon tile, the name, a line or two about it, who it is by, and a square + at the top
/// right that installs it, which is a check once it is here. The card opens the item's page.
class StoreTile extends StatelessWidget {
  const StoreTile({super.key, required this.leading, required this.title, required this.by, required this.installed,
      required this.onOpen, required this.onInstall, this.summary, this.busy = false, this.installKey});

  final Widget leading;
  final String title;
  final String? summary;
  final String by;
  final bool installed, busy;
  final Key? installKey;
  final VoidCallback onOpen;
  final VoidCallback? onInstall;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        onTap: onOpen,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            ClipRRect(borderRadius: BorderRadius.circular(10), child: leading),
            const SizedBox(width: 12),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(oneLine(title), style: text.titleSmall?.copyWith(fontWeight: FontWeight.w700), maxLines: 1, overflow: TextOverflow.ellipsis),
                if (summary != null) Text(oneLine(summary!), style: text.bodyMedium, maxLines: 2, overflow: TextOverflow.ellipsis),
                Text(oneLine(by), key: const Key('provenance'),
                    style: text.bodySmall?.copyWith(color: scheme.onSurfaceVariant), maxLines: 1, overflow: TextOverflow.ellipsis),
              ]),
            ),
            const SizedBox(width: 8),
            SizedBox.square(
              dimension: 36,
              child: busy
                  ? const Padding(padding: EdgeInsets.all(8), child: CircularProgressIndicator(strokeWidth: 2))
                  : IconButton.filledTonal(
                      key: installKey,
                      tooltip: installed ? 'Installed' : 'Install',
                      style: IconButton.styleFrom(shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8))),
                      onPressed: installed ? null : onInstall,
                      icon: Icon(installed ? Icons.check : Icons.add, size: 20),
                    ),
            ),
          ]),
        ),
      ),
    );
  }
}

/// The publisher a store tile names: its authors, else where its source is.
String publisherOf(ListedEntry listed) {
  final authors = (listed.entry.about['authors'] as List?)?.map((author) => '$author').toList() ?? const [];
  return authors.isEmpty ? 'from ${displaySource(listed.entry.source)}' : 'by ${authors.join(', ')}';
}

/// Whether [config] names an overlay of this entry's identity from its very source, as the store installs it.
bool isInstalled(CanoaConfig config, CatalogEntry entry) {
  try {
    return namedAs(config, installName(entry)) != null &&
        entryState(config, installName(entry), installSource(entry)) != EntryState.absent &&
        entryState(config, installName(entry), installSource(entry)) != EntryState.taken;
  } on StoreRefusal {
    return false;
  }
}
