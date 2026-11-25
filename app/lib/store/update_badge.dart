import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';

/// "Update available" as a small pill with the update icon, on the card of an installed item whose source holds something newer.
class UpdateBadge extends StatelessWidget {
  const UpdateBadge({super.key});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return DecoratedBox(
      decoration: BoxDecoration(color: scheme.primaryContainer, borderRadius: BorderRadius.circular(10)),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          Icon(Icons.arrow_circle_up, size: 14, color: scheme.onPrimaryContainer),
          const SizedBox(width: 4),
          Flexible(
            child: Text(
              context.t('overlays.update_available'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.labelSmall?.copyWith(color: scheme.onPrimaryContainer),
            ),
          ),
        ]),
      ),
    );
  }
}

/// The first eight characters of a commit.
String shortCommit(String commit) => commit.length < 8 ? commit : commit.substring(0, 8);
