import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';

/// The one switch Settings draws for "this is on, or off": a catalog source on the Catalog sources page and a provider on the
/// Providers page are turned off and on with it, and both mark what is off with [OffBadge], so the person meets one vocabulary.
class OffSwitch extends StatelessWidget {
  const OffSwitch({super.key, required this.on, required this.onChanged, required this.label});

  /// Whether the thing is on.
  final bool on;

  /// Turns it off or on; null while it cannot be changed.
  final ValueChanged<bool>? onChanged;

  /// What a screen reader says the switch turns.
  final String label;

  @override
  Widget build(BuildContext context) => Semantics(
    container: true,
    label: label,
    toggled: on,
    child: Switch(value: on, onChanged: onChanged),
  );
}

/// The word "off" as a small mark beside a name, with the reason in a tooltip when there is one.
class OffBadge extends StatelessWidget {
  const OffBadge({super.key, this.reason});

  /// Why it is off, when that is worth saying.
  final String? reason;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final badge = DecoratedBox(
      decoration: BoxDecoration(color: scheme.surfaceContainerHighest, borderRadius: BorderRadius.circular(6)),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        child: Text(AppText.of(context)('switch.off'), style: Theme.of(context).textTheme.labelSmall?.copyWith(color: scheme.onSurfaceVariant)),
      ),
    );
    return reason == null ? badge : Tooltip(message: reason!, child: badge);
  }
}
