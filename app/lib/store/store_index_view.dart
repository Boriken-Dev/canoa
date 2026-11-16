import 'package:flutter/material.dart';

import '../theme/look.dart';
import 'overlay_page.dart' show oneLine;
import 'store_selection.dart';

/// The Store section's index: the stores (catalogs) the Store reads, each with its icon, its name and how much it offers, and
/// "All stores" first. Choosing one scopes the Store's content to it.
class StoreIndex extends StatelessWidget {
  const StoreIndex({super.key, required this.selection});

  final StoreSelection selection;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: selection,
    builder: (context, _) {
      final sources = selection.sources;
      final chosen = selection.selected;
      final text = Theme.of(context).textTheme;
      return Material(
        color: context.look.rail,
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 14, 8, 10),
            child: Text('Stores', style: text.titleLarge),
          ),
          const Divider(height: 1),
          Expanded(
            child: sources.isEmpty
                ? Padding(
                    padding: const EdgeInsets.all(18),
                    child: Text(
                      selection.loading
                          ? 'Reading the stores…'
                          : 'No store yet. Add a catalog in Store settings to list its overlays and skills.',
                      key: const Key('store-index-empty'),
                    ),
                  )
                : ListView(children: [
                    _Row(
                      rowKey: const Key('store-index-all'),
                      icon: Icons.apps_outlined,
                      title: 'All stores',
                      detail: '${sources.fold<int>(0, (sum, source) => sum + source.overlays)} overlays · '
                          '${sources.fold<int>(0, (sum, source) => sum + source.skills)} skills',
                      selected: chosen == null,
                      onTap: () => _choose(context, null),
                    ),
                    for (final source in sources)
                      _Row(
                        rowKey: Key('store-index-${source.label}'),
                        icon: source.failed ? Icons.cloud_off_outlined : Icons.storefront_outlined,
                        title: source.label,
                        detail: source.failed
                            ? 'Could not be read'
                            : '${source.overlays} overlays · ${source.skills} skills${source.stale ? ' · older copy' : ''}',
                        warning: source.problem,
                        selected: chosen == source.label,
                        onTap: () => _choose(context, source.label),
                      ),
                  ]),
          ),
        ]),
      );
    },
  );

  void _choose(BuildContext context, String? label) {
    selection.select(label);
    // On a narrow window the index is in the drawer, which gives way to the content once a store is chosen.
    final scaffold = Scaffold.maybeOf(context);
    if (scaffold != null && scaffold.isDrawerOpen) scaffold.closeDrawer();
  }
}

class _Row extends StatelessWidget {
  const _Row({required this.rowKey, required this.icon, required this.title, required this.detail, required this.selected,
      required this.onTap, this.warning});

  final Key rowKey;
  final IconData icon;
  final String title, detail;
  final String? warning;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Semantics(
      selected: selected,
      child: Material(
        color: selected ? context.look.selected : Colors.transparent,
        child: InkWell(
          key: rowKey,
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
            child: Row(children: [
              Icon(icon, size: 20, color: warning != null ? scheme.error : scheme.onSurfaceVariant),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(oneLine(title), maxLines: 1, overflow: TextOverflow.ellipsis),
                  Text(detail, maxLines: 1, overflow: TextOverflow.ellipsis, style: Theme.of(context).textTheme.bodySmall),
                ]),
              ),
              if (warning != null)
                Tooltip(message: oneLine(warning!), child: Icon(Icons.error_outline, size: 16, color: scheme.error)),
            ]),
          ),
        ),
      ),
    );
  }
}
