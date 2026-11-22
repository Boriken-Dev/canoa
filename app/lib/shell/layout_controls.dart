import 'package:flutter/material.dart';

import '../l10n/app_text.dart';

import 'layout_state.dart';
import 'outline_icon.dart';
import 'shell.dart' show shortcutLabel;

/// Three window outlines with a region filled, for the left index, the bottom panel and the right panel, and a Layout menu. Each
/// toggle shows only where its region has something to show, is pressed while the region is, and acts on the same layout state as
/// its shortcut and its menu command.
class LayoutControls extends StatelessWidget {
  const LayoutControls({
    super.key,
    required this.layout,
    required this.section,
    required this.hasIndex,
    required this.hasRight,
    required this.hasBottom,
    this.menu = true,
  });

  final ShellLayout layout;

  /// The section whose index the left toggle hides.
  final String section;

  /// Whether each region has a surface or an index for this view.
  final bool hasIndex, hasRight, hasBottom;

  /// Whether the Layout menu follows the toggles.
  final bool menu;

  Widget _toggle(BuildContext context, {
    required Key key,
    required WindowRegion region,
    required bool shown,
    required String tooltip,
    required VoidCallback onPressed,
  }) => Tooltip(
    message: tooltip,
    child: Semantics(
      button: true,
      toggled: shown,
      label: tooltip,
      excludeSemantics: true,
      child: IconButton(
        key: key,
        isSelected: shown,
        visualDensity: VisualDensity.compact,
        icon: WindowOutline(region: region, filled: false),
        selectedIcon: WindowOutline(region: region, filled: true),
        onPressed: onPressed,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: layout,
    builder: (context, _) {
      final focused = layout.focus != PanelFocus.none;
      final indexShown = !layout.collapsed(section) && !focused;
      final rightShown = layout.rightOpen && !focused;
      final bottomShown = layout.bottomOpen && !focused;
      return Row(mainAxisSize: MainAxisSize.min, children: [
        if (focused)
          TextButton(key: const ValueKey('layout-restore'), onPressed: layout.restore, child: Text(context.t('layout.restore'))),
        if (hasIndex)
          _toggle(
            context,
            key: const ValueKey('layout-left'),
            region: WindowRegion.left,
            shown: indexShown,
            tooltip: context.t('layout.toggle_index', {'keys': shortcutLabel('B')}),
            onPressed: () => layout.toggleIndex(section),
          ),
        if (hasBottom)
          _toggle(
            context,
            key: const ValueKey('layout-bottom'),
            region: WindowRegion.bottom,
            shown: bottomShown,
            tooltip: context.t('layout.toggle_bottom', {'keys': shortcutLabel('`')}),
            onPressed: layout.toggleBottom,
          ),
        if (hasRight)
          _toggle(
            context,
            key: const ValueKey('layout-right'),
            region: WindowRegion.right,
            shown: rightShown,
            tooltip: context.t('layout.toggle_right', {'keys': shortcutLabel('Alt+B')}),
            onPressed: layout.toggleRight,
          ),
        if (menu)
        PopupMenuButton<String>(
          key: const ValueKey('layout-menu'),
          tooltip: context.t('layout.menu'),
          icon: const SymbolIcon('layout', size: 20),
          padding: const EdgeInsets.all(6),
          onSelected: (value) {
            switch (value) {
              case 'focus':
                layout.focusOn(PanelFocus.main);
              case 'restore':
                layout.restore();
              case 'reset':
                layout.reset();
            }
          },
          itemBuilder: (menu) => [
            if (focused)
              PopupMenuItem(value: 'restore', child: Text(menu.t('layout.restore')))
            else
              PopupMenuItem(value: 'focus', child: Text(menu.t('layout.focus'))),
            PopupMenuItem(value: 'reset', child: Text(menu.t('layout.reset'))),
          ],
        ),
      ]);
    },
  );
}
