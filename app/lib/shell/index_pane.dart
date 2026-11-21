import 'package:flutter/material.dart';

import '../theme/look.dart';

/// A rail-coloured section index pane with a title bar, optional header action, optional controls above the divider, a scrollable
/// or empty body, and an optional footer.
class IndexPane extends StatelessWidget {
  const IndexPane({
    super.key,
    required this.title,
    required this.body,
    this.trailing,
    this.aboveDivider = const [],
    this.footer,
    this.headerBottomPadding = 6,
  });

  final String title;
  final Widget? trailing;
  final List<Widget> aboveDivider;
  final Widget body;
  final Widget? footer;
  final double headerBottomPadding;

  @override
  Widget build(BuildContext context) => Material(
    color: context.look.rail,
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(18, 14, 8, headerBottomPadding),
          child: Row(
            children: [
              Expanded(child: Text(title, style: Theme.of(context).textTheme.titleLarge)),
              ?trailing,
            ],
          ),
        ),
        ...aboveDivider,
        const Divider(height: 1),
        Expanded(child: body),
        ?footer,
      ],
    ),
  );
}

/// One selectable row in an [IndexPane], with a 20 px leading icon or graphic, a primary label (or custom [titleWidget]), an
/// optional secondary line, and optional trailing widgets.
class IndexRow extends StatelessWidget {
  const IndexRow({
    super.key,
    required this.rowKey,
    required this.leading,
    required this.subtitle,
    required this.selected,
    required this.onTap,
    this.title,
    this.titleWidget,
    this.trailing = const [],
  }) : assert(title != null || titleWidget != null);

  final Key rowKey;
  final Widget leading;
  final String? title;
  final Widget? titleWidget;
  final String subtitle;
  final bool selected;
  final VoidCallback onTap;
  final List<Widget> trailing;

  @override
  Widget build(BuildContext context) {
    final label = titleWidget ?? Text(title!, maxLines: 1, overflow: TextOverflow.ellipsis);
    return Semantics(
      selected: selected,
      child: Material(
        color: selected ? context.look.selected : Colors.transparent,
        child: InkWell(
          key: rowKey,
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
            child: Row(
              children: [
                leading,
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      label,
                      Text(subtitle, maxLines: 1, overflow: TextOverflow.ellipsis, style: Theme.of(context).textTheme.bodySmall),
                    ],
                  ),
                ),
                ...trailing,
              ],
            ),
          ),
        ),
      ),
    );
  }
}
