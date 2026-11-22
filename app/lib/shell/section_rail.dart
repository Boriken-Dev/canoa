import 'package:flutter/material.dart';

import '../l10n/app_text.dart';
import '../theme/look.dart';
import 'sections.dart';

/// The vertical selector at the window's far left: an icon for each section with its name in a tooltip and for a screen reader,
/// and at its foot the sections that sit there and the account. The section index is hidden or shown from the layout controls
/// at the window's top right, never from here.
class SectionRail extends StatelessWidget {
  const SectionRail({
    super.key,
    required this.sections,
    required this.selected,
    required this.onSelect,
    this.account,
    this.onSignOut,
  });

  /// The width of the selector.
  static const width = 56.0;

  final List<AppSection> sections;
  final String selected;
  final ValueChanged<String> onSelect;

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
                child: section.symbol == null
                    ? Icon(section.icon, size: 22, color: on ? scheme.primary : scheme.onSurfaceVariant)
                    : SymbolIcon(section.symbol!, size: 22, color: on ? scheme.primary : scheme.onSurfaceVariant),
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
          for (final section in top) _item(context, section),
          const Spacer(),
          for (final section in foot) _item(context, section),
          if (onSignOut != null)
            Tooltip(
              message: account == null ? context.t('shell.sign_out') : context.t('shell.signed_in_as', {'account': account}),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                child: IconButton(
                  key: const ValueKey('sign-out'),
                  icon: const SymbolIcon('sign_out', size: 22),
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
