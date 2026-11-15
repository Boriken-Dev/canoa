import 'package:flutter/material.dart';

import '../theme/look.dart';
import 'sections.dart';

/// The vertical selector at the window's far left: an icon for each section with its name in a tooltip and for a screen reader,
/// and at its foot the sections that sit there and the account. [toggle], when given, is the button that hides or shows the
/// section's index.
class SectionRail extends StatelessWidget {
  const SectionRail({
    super.key,
    required this.sections,
    required this.selected,
    required this.onSelect,
    this.toggle,
    this.account,
    this.onSignOut,
  });

  /// The width of the selector.
  static const width = 56.0;

  final List<AppSection> sections;
  final String selected;
  final ValueChanged<String> onSelect;

  /// The index toggle, drawn at the top; none when the section has no index.
  final Widget? toggle;
  final String? account;
  final Future<void> Function()? onSignOut;

  Widget _item(BuildContext context, AppSection section) {
    final scheme = Theme.of(context).colorScheme;
    final on = section.id == selected;
    return Semantics(
      button: true,
      selected: on,
      label: section.label,
      excludeSemantics: true,
      child: Tooltip(
        message: section.tooltip ?? section.label,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
          child: Material(
            color: on ? context.look.selected : Colors.transparent,
            borderRadius: BorderRadius.circular(10),
            child: InkWell(
              key: ValueKey('section ${section.id}'),
              borderRadius: BorderRadius.circular(10),
              onTap: () => onSelect(section.id),
              child: SizedBox(
                height: 44,
                child: Icon(section.icon, size: 22, color: on ? scheme.primary : scheme.onSurfaceVariant),
              ),
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final top = [for (final section in sections) if (!section.atFoot) section];
    final foot = [for (final section in sections) if (section.atFoot) section];
    return Material(
      color: context.look.rail,
      child: SizedBox(
        width: width,
        child: Column(children: [
          const SizedBox(height: 10),
          ?toggle,
          for (final section in top) _item(context, section),
          const Spacer(),
          for (final section in foot) _item(context, section),
          if (onSignOut != null)
            Tooltip(
              message: account == null ? 'Sign out' : 'Signed in as $account. Sign out',
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                child: IconButton(
                  key: const ValueKey('sign-out'),
                  icon: const Icon(Icons.logout_rounded, size: 22),
                  onPressed: onSignOut,
                ),
              ),
            ),
          const SizedBox(height: 8),
        ]),
      ),
    );
  }
}
