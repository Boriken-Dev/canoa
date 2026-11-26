import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../shell/index_pane.dart';
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
      final scheme = Theme.of(context).colorScheme;
      return IndexPane(
        title: 'Store',
        headerBottomPadding: 10,
        aboveDivider: [
          if (selection.updates > 0 && selection.onUpdateAll != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 0, 14, 10),
              child: Align(
                alignment: Alignment.centerLeft,
                child: FilledButton.tonalIcon(
                  key: const Key('store-index-update-all'),
                  onPressed: () => _updateAll(context),
                  icon: const Icon(Icons.arrow_circle_up, size: 18),
                  label: Text(context.t('store.update_all_count', {'count': selection.updates})),
                ),
              ),
            ),
        ],
        body: ListView(children: [
          for (final type in selection.types)
            IndexRow(
              rowKey: Key('store-index-${type.id}'),
              leading: type.image == null
                  ? Icon(type.icon, size: 20, color: scheme.onSurfaceVariant)
                  : StillIcon(
                      key: Key('store-index-${type.id}-image'),
                      name: type.label,
                      path: type.image!.path,
                      bytes: type.image!.bytes,
                      size: 20,
                    ),
              title: type.label,
              subtitle: type.available == null
                  ? context.t('store.index_installed', {'installed': type.installed})
                  : context.t('store.index_counts', {'installed': type.installed, 'available': type.available}),
              selected: chosen == type.id,
              trailing: [
                if (type.updates > 0)
                  Tooltip(
                    message: context.t('overlays.updates_badge', {'count': type.updates}),
                    child: Badge(key: Key('store-index-${type.id}-updates'), label: Text('${type.updates}')),
                  ),
              ],
              onTap: () => _choose(context, type.id),
            ),
        ]),
      );
    },
  );

  void _choose(BuildContext context, String id) {
    selection.select(id);
    _closeDrawer(context);
  }

  void _updateAll(BuildContext context) {
    _closeDrawer(context);
    selection.onUpdateAll?.call();
  }

  /// On a narrow window the index is in the drawer, which gives way to the content once something is chosen in it.
  void _closeDrawer(BuildContext context) {
    final scaffold = Scaffold.maybeOf(context);
    if (scaffold != null && scaffold.isDrawerOpen) scaffold.closeDrawer();
  }
}
