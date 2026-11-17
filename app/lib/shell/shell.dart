import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../chat/palette.dart' show chord;

import '../theme/look.dart';
import 'layout_controls.dart';
import 'layout_state.dart';
import 'section_rail.dart';
import 'sections.dart';
import 'split.dart';

/// A shortcut as the person reads it on this platform: "Ctrl+B", or "⌘B" on a Mac.
String shortcutLabel(String key) => defaultTargetPlatform == TargetPlatform.macOS ? '⌘$key' : 'Ctrl+$key';

/// What a section's main view can ask of the shell it sits in.
class ShellScope extends InheritedWidget {
  const ShellScope({super.key, required this.narrow, required this.openNavigation, required this.layout, required super.child});

  /// Whether the selector and the index are in a drawer.
  final bool narrow;

  /// Opens the drawer; only meaningful when [narrow].
  final VoidCallback openNavigation;
  final ShellLayout layout;

  static ShellScope? maybeOf(BuildContext context) => context.dependOnInheritedWidgetOfExactType<ShellScope>();

  /// The button that opens the drawer on a narrow window, for a page's own app bar; null on a wide one.
  static Widget? menuButton(BuildContext context) {
    final scope = maybeOf(context);
    if (scope == null || !scope.narrow) return null;
    return IconButton(key: const ValueKey('open-navigation'), tooltip: 'Sections', icon: const Icon(Icons.menu), onPressed: scope.openNavigation);
  }

  @override
  bool updateShouldNotify(ShellScope oldWidget) => narrow != oldWidget.narrow || layout != oldWidget.layout;
}

/// The three areas: the section selector, the selected section's index, and its main view. A narrow window puts the selector and
/// the index in a drawer. A section's main view is built when first shown and kept while another is, so its state stays.
class AppShell extends StatefulWidget {
  const AppShell({
    super.key,
    required this.layout,
    required this.sections,
    required this.narrow,
    this.account,
    this.onSignOut,
    this.indexWidth = 260,
    this.banner,
    this.selector = true,
  });

  /// Whether the section selector shows. The chat-only app has one section, so it has none.
  final bool selector;

  final ShellLayout layout;
  final List<AppSection> sections;
  final bool narrow;
  final String? account;
  final Future<void> Function()? onSignOut;

  /// The index's width until the person changes it.
  final double indexWidth;

  /// A notice across the top of the window, above the selector, the index and the main view.
  final Widget? banner;

  /// Whether a window of this width is narrow.
  static bool isNarrow(double width) => width < 730;

  @override
  State<AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<AppShell> {
  final _scaffold = GlobalKey<ScaffoldState>();
  final Set<String> _shown = {};

  /// Keeps the sections' views alive when the index appears and goes, which moves them in the tree.
  final _mainKey = GlobalKey();

  AppSection get _current => widget.sections.firstWhere(
    (section) => section.id == widget.layout.section,
    orElse: () => widget.sections.first,
  );

  void _select(String id) {
    widget.layout.select(id);
    if (widget.narrow && widget.sections.firstWhere((section) => section.id == id).index == null) {
      _scaffold.currentState?.closeDrawer();
    }
  }

  Widget _selector(AppSection current) => SectionRail(
    sections: widget.sections,
    selected: current.id,
    onSelect: _select,
    account: widget.account,
    onSignOut: widget.onSignOut,
  );

  Widget _stack(AppSection current) {
    _shown.add(current.id);
    return Stack(fit: StackFit.expand, children: [
      for (final section in widget.sections)
        if (_shown.contains(section.id))
          Offstage(
            key: ValueKey('main ${section.id}'),
            offstage: section.id != current.id,
            child: TickerMode(enabled: section.id == current.id, child: section.main),
          ),
    ]);
  }

  /// Hides or shows the index of the section shown; on a narrow window it opens the drawer, where the index is.
  void _toggleIndex() {
    final current = _current;
    if (widget.narrow) {
      _scaffold.currentState?.openDrawer();
    } else if (current.index != null) {
      widget.layout.toggleIndex(current.id);
    }
  }

  @override
  Widget build(BuildContext context) => CallbackShortcuts(
    bindings: {chord(LogicalKeyboardKey.keyB): _toggleIndex},
    child: Focus(autofocus: true, skipTraversal: true, child: _build(context)),
  );

  Widget _under(Widget child) => widget.banner == null
      ? child
      : Column(children: [widget.banner!, Expanded(child: child)]);

  Widget _build(BuildContext context) => ListenableBuilder(
    listenable: widget.layout,
    builder: (context, _) {
      final current = _current;
      final layout = widget.layout;
      final main = ShellScope(
        key: _mainKey,
        narrow: widget.narrow,
        layout: layout,
        openNavigation: () => _scaffold.currentState?.openDrawer(),
        child: Stack(fit: StackFit.expand, children: [
          _stack(current),
          if (!widget.narrow && current.index != null && !current.ownsLayoutControls)
            Positioned(
              top: 6,
              right: 8,
              child: LayoutControls(layout: layout, section: current.id, hasIndex: true, hasRight: false, hasBottom: false, menu: false),
            )
          else
            const SizedBox.shrink(),
        ]),
      );
      if (widget.narrow) {
        return Scaffold(
          key: _scaffold,
          drawer: Drawer(
            width: (widget.selector ? SectionRail.width : 0) + 304,
            child: SafeArea(
              child: Row(children: [
                if (widget.selector) _selector(current),
                Expanded(child: current.index ?? ColoredBox(color: context.look.rail, child: const SizedBox.expand())),
              ]),
            ),
          ),
          body: SafeArea(child: _under(main)),
        );
      }
      final hidden = layout.collapsed(current.id);
      return Scaffold(
        key: _scaffold,
        body: SafeArea(
          child: _under(Row(children: [
            if (widget.selector) ...[_selector(current), const VerticalDivider(width: 1)],
            Expanded(
              child: current.index == null || hidden
                  ? main
                  : IndexSplit(layout: layout, section: current.id, fallbackWidth: widget.indexWidth, index: current.index!, main: main),
            ),
          ])),
        ),
      );
    },
  );
}
