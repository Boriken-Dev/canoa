import '../l10n/app_text.dart';
import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import 'overlay_page.dart' show oneLine;
import 'store.dart';
import 'store_icon.dart';
import 'store_selection.dart' show StoreMarkImage;

/// One item a banner can feature: its name and icon, the catalog it is listed in, its tags, and what opens it.
final class BannerItem {
  const BannerItem({required this.name, required this.catalog, required this.open, this.iconPath, this.iconBytes, this.tags = const []});

  final String name, catalog;
  final String? iconPath;
  final List<int>? iconBytes;
  final List<String> tags;
  final VoidCallback open;
}

/// What one banner at the top of a type's page says: a line about whose it is, a title, a sentence, the icons of the items it
/// features and what Explore does.
final class BannerSpec {
  const BannerSpec({required this.kicker, required this.title, required this.about, required this.featured, required this.onExplore,
      this.exploreKey});

  final String kicker, title, about;
  final List<BannerItem> featured;
  final VoidCallback onExplore;
  final Key? exploreKey;
}

/// The banners of a type's page, from the data of the catalogs in [scope]: the type and what it lists, with the items tagged
/// `featured` first, then one for each of the most used tags. [noun] is the type's plural in lower case, [title] its name.
/// With one catalog in scope the first banner is that catalog's own name and description. With nothing featured the first
/// banner is the title and mark alone.
List<BannerSpec> bannersFor({
  required String noun,
  required String title,
  required List<LoadedCatalog> scope,
  required List<BannerItem> items,
  required void Function(String tag) showTag,
}) {
  final only = scope.length == 1 ? scope.single : null;
  final catalogs = scope.isNotEmpty ? scope.length : {for (final item in items) item.catalog}.length;
  final featured = [for (final item in items) if (item.tags.contains('featured')) item];
  final lead = (featured.isEmpty ? items : featured).take(6).toList();
  final specs = <BannerSpec>[
    BannerSpec(
      kicker: only == null ? 'Across $catalogs ${catalogs == 1 ? 'catalog' : 'catalogs'}' : 'From ${oneLine(only.label)}',
      title: oneLine(only?.catalog?.name ?? (only?.label ?? title)),
      about: oneLine(only?.catalog?.description ??
          (only == null ? '${items.length} $noun from the catalogs you have added.' : '${items.length} $noun, as ${only.label} lists them.')),
      featured: lead,
      onExplore: () {
        if (lead.isNotEmpty) lead.first.open();
      },
      exploreKey: const Key('store-banner-explore'),
    ),
  ];
  final uses = <String, List<BannerItem>>{};
  for (final item in items) {
    for (final tag in item.tags.toSet()) {
      if (tag != 'featured') (uses[tag] ??= []).add(item);
    }
  }
  final tags = uses.keys.where((tag) => uses[tag]!.length >= 2).toList()
    ..sort((a, b) => uses[a]!.length != uses[b]!.length ? uses[b]!.length - uses[a]!.length : a.compareTo(b));
  for (final tag in tags.take(2)) {
    specs.add(BannerSpec(
      kicker: only == null ? 'Across the catalogs' : 'From ${oneLine(only.label)}',
      title: oneLine(tag),
      about: '${uses[tag]!.length} $noun carry this tag.',
      featured: uses[tag]!.take(6).toList(),
      onExplore: () => showTag(tag),
      exploreKey: Key('store-banner-explore-$tag'),
    ));
  }
  return specs;
}

/// The wide card at the top of a type's page: a soft gradient of the look's own accent, a small line, a title, a sentence, the
/// icons it features and Explore. Arrows page through several on a wide window, with dots under; a narrow one swipes.
class StoreBanner extends StatefulWidget {
  const StoreBanner({super.key, required this.specs, required this.iconLimits, this.mark = Icons.storefront_outlined, this.markImage});

  final List<BannerSpec> specs;
  final aos.Settings? iconLimits;

  /// The icon shown when nothing is featured.
  final IconData mark;

  /// A picture shown in place of [mark], which an extension's type may ship.
  final StoreMarkImage? markImage;

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
          if (spec.featured.isEmpty && widget.markImage != null)
            StillIcon(key: const Key('store-banner-image'), name: spec.title, path: widget.markImage!.path, bytes: widget.markImage!.bytes, size: 32)
          else if (spec.featured.isEmpty)
            Icon(widget.mark, size: 32)
          else
            for (final item in spec.featured.take(narrow ? 4 : 6))
              Padding(
                padding: const EdgeInsets.only(right: 6),
                child: StillIcon(
                  name: item.name,
                  path: item.iconPath,
                  bytes: item.iconBytes,
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
      required this.onOpen, required this.onInstall, this.summary, this.needs, this.busy = false, this.installKey, this.onRemove,
      this.removeKey, this.onUpdate, this.updateKey, this.verified});

  final Widget leading;
  final String title;
  final String? summary;

  /// The algorithm the item's files are checked with; a check mark beside the title says so. Null shows none.
  final String? verified;

  /// What it needs of this computer or the agent, one line; null when nothing is said.
  final String? needs;
  final String by;
  final bool installed, busy;
  final Key? installKey, removeKey;
  final VoidCallback onOpen;
  final VoidCallback? onInstall;

  /// Takes an installed item away, shown beside the check; null for an item that has no copy of the host's to remove.
  final VoidCallback? onRemove;

  /// Brings an installed item up to what its source holds now; when set, the check becomes an update button.
  final VoidCallback? onUpdate;
  final Key? updateKey;

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
                Row(children: [
                  Flexible(child: Text(oneLine(title), style: text.titleSmall?.copyWith(fontWeight: FontWeight.w700), maxLines: 1, overflow: TextOverflow.ellipsis)),
                  if (verified case final algorithm?)
                    Padding(
                      padding: const EdgeInsets.only(left: 4),
                      child: Tooltip(
                        message: context.t('store.verified_with', {'algorithm': algorithm}),
                        child: Icon(Icons.verified_outlined, key: const Key('verified-mark'), size: 16, color: scheme.primary),
                      ),
                    ),
                ]),
                if (summary != null) Text(oneLine(summary!), style: text.bodyMedium, maxLines: 2, overflow: TextOverflow.ellipsis),
                Text(oneLine(by), key: const Key('provenance'),
                    style: text.bodySmall?.copyWith(color: scheme.onSurfaceVariant), maxLines: 1, overflow: TextOverflow.ellipsis),
                if (installed && onUpdate != null)
                  Text(context.t('overlays.update_available'), key: const Key('update-available'),
                      style: text.bodySmall?.copyWith(color: scheme.primary, fontWeight: FontWeight.w600)),
                if (needs != null)
                  Text(oneLine(needs!), key: const Key('compatibility'),
                      style: text.bodySmall?.copyWith(color: scheme.onSurfaceVariant), maxLines: 1, overflow: TextOverflow.ellipsis),
              ]),
            ),
            const SizedBox(width: 8),
            if (installed && !busy && onRemove != null)
              Padding(
                padding: const EdgeInsets.only(right: 4),
                child: SizedBox.square(
                  dimension: 36,
                  child: IconButton(
                    key: removeKey,
                    tooltip: 'Remove',
                    style: IconButton.styleFrom(shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8))),
                    onPressed: onRemove,
                    icon: const Icon(Icons.delete_outline, size: 20),
                  ),
                ),
              ),
            SizedBox.square(
              dimension: 36,
              child: busy
                  ? const Padding(padding: EdgeInsets.all(8), child: CircularProgressIndicator(strokeWidth: 2))
                  : IconButton.filledTonal(
                      key: installed && onUpdate != null ? updateKey : installKey,
                      tooltip: installed && onUpdate != null ? context.t('overlays.update') : installed ? 'Installed' : 'Install',
                      style: IconButton.styleFrom(
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                        backgroundColor: installed && onUpdate != null ? scheme.primary : null,
                        foregroundColor: installed && onUpdate != null ? scheme.onPrimary : null,
                      ),
                      onPressed: installed ? onUpdate : onInstall,
                      icon: Icon(installed ? (onUpdate != null ? Icons.arrow_circle_up : Icons.check) : Icons.add, size: 20),
                    ),
            ),
          ]),
        ),
      ),
    );
  }
}

/// The publisher a store tile names: its authors, else where its source is.
String publisherOf(ListedEntry listed) => publisherFrom(listed.entry.about, displaySource(listed.entry.source));

/// The same for any entry: its `authors` in [about], else [where] its source is.
String publisherFrom(Map<String, Object?> about, String where) {
  final authors = (about['authors'] as List?)?.map((author) => '$author').toList() ?? const [];
  return authors.isEmpty ? 'from $where' : 'by ${authors.join(', ')}';
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

/// The line that says what an entry needs of this computer, from its manifest; null when it needs nothing.
String? needsOf(EntryFacts? facts) => facts == null || facts.requirements.isEmpty ? null : 'Needs ${facts.requirements.join(', ')}';
