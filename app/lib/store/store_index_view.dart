import 'package:flutter/material.dart';

import '../theme/look.dart';
import 'store_icon.dart';
import 'store_selection.dart';

/// The Store section's index: the store types (Overlays, Skills, Agents, MCP servers, then those extensions add), each with its
/// icon, its name and how much of it there is. Choosing one shows that type's page across every catalog.
class StoreIndex extends StatelessWidget {
  const StoreIndex({super.key, required this.selection});

  final StoreSelection selection;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: selection,
    builder: (context, _) {
      final chosen = selection.selected;
      return Material(
        color: context.look.rail,
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 14, 8, 10),
            child: Text('Store', style: Theme.of(context).textTheme.titleLarge),
          ),
          const Divider(height: 1),
          Expanded(
            child: ListView(children: [
              for (final type in selection.types)
                _Row(
                  rowKey: Key('store-index-${type.id}'),
                  icon: type.icon,
                  image: type.image,
                  imageKey: Key('store-index-${type.id}-image'),
                  title: type.label,
                  detail: '${type.installed} installed${type.available == null ? '' : ' · ${type.available} to discover'}',
                  selected: chosen == type.id,
                  onTap: () => _choose(context, type.id),
                ),
            ]),
          ),
        ]),
      );
    },
  );

  void _choose(BuildContext context, String id) {
    selection.select(id);
    // On a narrow window the index is in the drawer, which gives way to the content once a type is chosen.
    final scaffold = Scaffold.maybeOf(context);
    if (scaffold != null && scaffold.isDrawerOpen) scaffold.closeDrawer();
  }
}

class _Row extends StatelessWidget {
  const _Row({required this.rowKey, required this.icon, required this.title, required this.detail, required this.selected,
      required this.onTap, this.image, this.imageKey});

  final Key rowKey;
  final IconData icon;
  final StoreMarkImage? image;
  final Key? imageKey;
  final String title, detail;
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
              if (image case final image?)
                StillIcon(key: imageKey, name: title, path: image.path, bytes: image.bytes, size: 20)
              else
                Icon(icon, size: 20, color: scheme.onSurfaceVariant),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(title, maxLines: 1, overflow: TextOverflow.ellipsis),
                  Text(detail, maxLines: 1, overflow: TextOverflow.ellipsis, style: Theme.of(context).textTheme.bodySmall),
                ]),
              ),
            ]),
          ),
        ),
      ),
    );
  }
}
