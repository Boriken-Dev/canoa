import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../theme/look.dart';
import 'provider_setup.dart';
import 'secrets_controller.dart';

/// The Secrets section's index: All vaults, then one row per vault with how many secrets it holds, and Set up a vault at the foot.
/// What it selects is what the page shows.
class SecretsIndex extends StatelessWidget {
  const SecretsIndex({super.key, required this.controller});

  final SecretsController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) => Material(
      color: context.look.rail,
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 14, 8, 10),
          child: Text('Secrets', style: Theme.of(context).textTheme.titleLarge),
        ),
        const Divider(height: 1),
        Expanded(
          child: ListView(children: [
            _Row(
              rowKey: const Key('secrets-index all'),
              icon: Icons.key_outlined,
              title: 'All vaults',
              detail: _count(controller.rows.length),
              selected: controller.vault == null,
              onTap: () => _choose(context, null),
            ),
            for (final entry in controller.entries)
              _Row(
                rowKey: Key('secrets-index vault ${entry.name}'),
                icon: entry.setUp ? Icons.lock_outline : Icons.shield_outlined,
                title: entry.name,
                detail: '${entry.title} · ${_count(controller.countIn(entry.name))}',
                selected: controller.vault == entry.name,
                onTap: () => _choose(context, entry.name),
              ),
          ]),
        ),
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.all(8),
          child: TextButton.icon(
            key: const Key('secret-add-vault'),
            onPressed: () => unawaited(_add(context)),
            icon: const Icon(Icons.add_moderator_outlined),
            label: const Text('Set up a vault'),
          ),
        ),
      ]),
    ),
  );

  static String _count(int n) => n == 1 ? '1 secret' : '$n secrets';

  void _choose(BuildContext context, String? name) {
    controller.select(name);
    // On a narrow window the index is in the drawer, which gives way to the page once a vault is chosen.
    final scaffold = Scaffold.maybeOf(context);
    if (scaffold != null && scaffold.isDrawerOpen) scaffold.closeDrawer();
  }

  Future<void> _add(BuildContext context) async {
    await showSecretProviderSetup(context, controller);
    unawaited(controller.refresh());
  }
}

class _Row extends StatelessWidget {
  const _Row({required this.rowKey, required this.icon, required this.title, required this.detail, required this.selected, required this.onTap});

  final Key rowKey;
  final IconData icon;
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
