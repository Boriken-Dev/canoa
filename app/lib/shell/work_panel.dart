import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/look.dart';
import 'bottom_panel.dart';
import 'layout_state.dart';
import 'split.dart';
import 'surface.dart';

/// A panel of surfaces: tabs for the ones open in it, a menu that opens another, and the chosen surface. Empty, it offers a
/// launcher. Every open surface stays built while another is chosen, so none starts over.
class WorkPanel extends StatelessWidget {
  const WorkPanel({
    super.key,
    required this.slot,
    required this.session,
    required this.surfaces,
    required this.offered,
    required this.layout,
    this.narrow = false,
  });

  final SurfaceSlot slot;
  final ChatSession session;
  final SessionSurfaces surfaces;

  /// What the panel could open for this session.
  final List<WorkSurface> offered;
  final ShellLayout layout;

  /// Whether the panel fills the window, with a way back to the conversation.
  final bool narrow;

  bool get _right => slot == SurfaceSlot.right;

  PanelFocus get _mine => _right ? PanelFocus.right : PanelFocus.bottom;

  void _hide() => _right ? layout.setRightOpen(false) : layout.setBottomOpen(false);

  Widget _header(BuildContext context, List<OpenSurface> here, OpenSurface? active) {
    final scheme = Theme.of(context).colorScheme;
    final focused = layout.focus == _mine;
    final other = _right ? SurfaceSlot.bottom : SurfaceSlot.right;
    final canMove = active != null && active.surface.placements.contains(other);
    // Read when the menu opens, so what depends on a selection is as it is then.
    List<SurfaceAction> actionsNow() => active == null ? const <SurfaceAction>[] : active.surface.actions(context, session, active.controller);
    return Material(
      color: context.look.bar,
      child: SizedBox(
        height: 44,
        child: Row(children: [
          if (narrow)
            IconButton(
              key: const ValueKey('panel-back'),
              tooltip: 'Back to the conversation',
              icon: const Icon(Icons.arrow_back),
              onPressed: _hide,
            ),
          Expanded(
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 6),
              children: [
                for (final each in here)
                  if (each.surface.tabs(session, each.controller) case final inside when inside.isNotEmpty)
                    for (final tab in inside)
                      Padding(
                        padding: const EdgeInsets.only(right: 4),
                        child: _Tab(
                          key: ValueKey('tab ${each.surface.id}:${tab.id}'),
                          title: tab.title,
                          icon: each.surface.icon,
                          closeKey: ValueKey('close ${each.surface.id}:${tab.id}'),
                          selected: identical(each, active) && tab.selected,
                          onSelect: () {
                            tab.select();
                            surfaces.activate(each.surface.id);
                          },
                          onClose: tab.close ?? () => surfaces.close(each.surface.id),
                        ),
                      )
                  else
                    Padding(
                      padding: const EdgeInsets.only(right: 4),
                      child: _Tab(
                        key: ValueKey('tab ${each.surface.id}'),
                        title: each.title,
                        icon: each.surface.icon,
                        closeKey: ValueKey('close ${each.surface.id}'),
                        selected: identical(each, active),
                        onSelect: () => surfaces.activate(each.surface.id),
                        onClose: () => surfaces.close(each.surface.id),
                      ),
                    ),
              ],
            ),
          ),
          if (here.isNotEmpty)
            Builder(
              builder: (button) => IconButton(
                key: const ValueKey('panel-add'),
                tooltip: 'New tab',
                icon: const Icon(Icons.add, size: 20),
                padding: EdgeInsets.zero,
                onPressed: () => unawaited(_chooseNew(button)),
              ),
            ),
          PopupMenuButton<String>(
            key: const ValueKey('panel-more'),
            tooltip: 'Panel actions',
            icon: const Icon(Icons.more_horiz, size: 20),
            padding: EdgeInsets.zero,
            onSelected: (value) {
              if (value.startsWith('action:')) {
                for (final each in actionsNow()) {
                  if (value == 'action:${each.id}') each.run(context);
                }
                return;
              }
              switch (value) {
                case 'move':
                  surfaces.move(active!.surface.id, other);
                  // The panel it went to is the one to see it in.
                  _right ? layout.showBottom() : layout.showRight();
                case 'focus':
                  layout.focusOn(_mine);
                case 'restore':
                  layout.restore();
              }
            },
            itemBuilder: (_) => [
              for (final each in actionsNow())
                PopupMenuItem(key: ValueKey('action ${active!.surface.id} ${each.id}'), value: 'action:${each.id}', enabled: each.enabled, child: Text(each.label)),
              if (actionsNow().isNotEmpty) const PopupMenuDivider(),
              if (canMove) PopupMenuItem(value: 'move', child: Text(_right ? 'Move to the bottom panel' : 'Move to the right panel')),
              if (!focused) const PopupMenuItem(value: 'focus', child: Text('Maximise this panel')),
              if (focused) const PopupMenuItem(value: 'restore', child: Text('Restore the panel to its size')),
            ],
          ),
          if (focused)
            IconButton(
              key: const ValueKey('panel-restore'),
              tooltip: 'Restore the panel to its size',
              visualDensity: VisualDensity.compact,
              icon: Icon(Icons.close_fullscreen, size: 18, color: scheme.onSurfaceVariant),
              onPressed: layout.restore,
            )
          else if (!narrow)
            IconButton(
              key: const ValueKey('panel-focus'),
              tooltip: 'Maximise the panel to the whole conversation area',
              visualDensity: VisualDensity.compact,
              icon: Icon(Icons.open_in_full, size: 18, color: scheme.onSurfaceVariant),
              onPressed: () => layout.focusOn(_mine),
            ),
          if (!narrow)
            IconButton(key: const ValueKey('panel-hide'), tooltip: 'Hide the panel', visualDensity: VisualDensity.compact, icon: const Icon(Icons.close, size: 20), onPressed: _hide),
        ]),
      ),
    );
  }

  /// The kinds of tab the + menu offers (each surface says its own), asked when it is opened so that what a host lists since shows.
  Future<void> _chooseNew(BuildContext button) async {
    final box = button.findRenderObject()! as RenderBox;
    final overlay = Overlay.of(button).context.findRenderObject()! as RenderBox;
    final corner = RelativeRect.fromRect(
      Rect.fromPoints(box.localToGlobal(Offset.zero, ancestor: overlay), box.localToGlobal(box.size.bottomRight(Offset.zero), ancestor: overlay)),
      Offset.zero & overlay.size,
    );
    final entries = [for (final each in await Future.wait([for (final surface in offered) surface.newTabs(session, surfaces)])) ...each];
    if (entries.isEmpty || !button.mounted) return;
    final chosen = await showMenu<NewTab>(
      context: button,
      position: corner,
      items: [
        for (final each in entries)
          PopupMenuItem(
            key: ValueKey(each.key),
            value: each,
            enabled: each.enabled,
            child: Text.rich(TextSpan(children: [WidgetSpan(alignment: PlaceholderAlignment.middle, child: Icon(each.icon, size: 18)), TextSpan(text: '  ${each.label}')])),
          ),
      ],
    );
    chosen?.open(slot);
  }

  Widget _launcher(BuildContext context) {
    final look = context.chatLook;
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24),
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Text('Open a surface', textAlign: TextAlign.center, style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 4),
          Text('It shows beside the conversation and needs no message to the agent.',
              textAlign: TextAlign.center, style: TextStyle(color: look.muted, fontSize: 12)),
          const SizedBox(height: 12),
          if (offered.isEmpty) Text('Nothing here has anything to show for this session yet.', textAlign: TextAlign.center, style: TextStyle(color: look.muted)),
          for (final each in offered)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 2),
              child: OutlinedButton.icon(
                key: ValueKey('launch ${each.id}'),
                onPressed: () => surfaces.openSurface(each, slot),
                icon: Icon(each.icon, size: 18),
                label: Text(each.title),
              ),
            ),
        ]),
      ),
    );
  }

  Widget _body(BuildContext context, List<OpenSurface> here, OpenSurface? active) {
    if (here.isEmpty) return _launcher(context);
    return Stack(fit: StackFit.expand, children: [
      for (final each in here)
        Offstage(
          key: ValueKey('surface ${each.surface.id}'),
          offstage: !identical(each, active),
          child: KeyedSubtree(key: each.key, child: _built(context, each)),
        ),
    ]);
  }

  /// A surface that fails to build says so in its place: the conversation and the other surfaces go on.
  Widget _built(BuildContext context, OpenSurface each) {
    try {
      return each.surface.build(context, session, each.controller);
    } on Object catch (error) {
      return Center(child: Padding(padding: const EdgeInsets.all(16), child: Text('${each.title} could not be shown: $error', textAlign: TextAlign.center)));
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([surfaces, layout, for (final each in surfaces.inSlot(slot)) ?each.surface.tabsChanged(session, each.controller)]),
    builder: (context, _) {
      final here = surfaces.inSlot(slot);
      final active = surfaces.activeIn(slot);
      return Material(
        color: context.look.rail,
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          _header(context, here, active),
          const Divider(height: 1),
          Expanded(child: _body(context, here, active)),
        ]),
      );
    },
  );
}

class _Tab extends StatelessWidget {
  const _Tab({super.key, required this.title, required this.icon, required this.closeKey, required this.selected, required this.onSelect, required this.onClose});

  final String title;
  final IconData icon;
  final Key closeKey;
  final bool selected;
  final VoidCallback onSelect, onClose;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Semantics(
      button: true,
      selected: selected,
      label: title,
      child: Material(
        color: selected ? context.look.selected : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: onSelect,
          child: Padding(
            padding: const EdgeInsets.only(left: 10),
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              Icon(icon, size: 16, color: selected ? scheme.primary : scheme.onSurfaceVariant),
              const SizedBox(width: 6),
              Text(title, style: TextStyle(fontSize: 13, fontWeight: selected ? FontWeight.w600 : FontWeight.w400)),
              if (selected)
                IconButton(
                  key: closeKey,
                  tooltip: 'Close $title',
                  visualDensity: VisualDensity.compact,
                  iconSize: 14,
                  icon: const Icon(Icons.close),
                  onPressed: onClose,
                )
              else
                const SizedBox(width: 10),
            ]),
          ),
        ),
      ),
    );
  }
}

/// [value] held between [min] and [max], where [min] wins in a window too small for both.
double _within(double value, double min, double max) => value.clamp(min, max < min ? min : max);

/// The conversation with the right and bottom panels around it, sized from [layout]: the right panel the full height beside it, the
/// bottom one under it. A panel hidden keeps its surfaces built, so showing it again finds them as they were.
class PanelFrame extends StatelessWidget {
  const PanelFrame({
    super.key,
    required this.layout,
    required this.session,
    required this.surfaces,
    required this.registry,
    required this.child,
    required this.wrap,
    this.narrow = false,
  });

  final ShellLayout layout;
  final ChatSession session;
  final SessionSurfaces surfaces;
  final WorkSurfaces registry;

  /// Whether the window is narrow: a panel then takes all of it, with a way back to the conversation.
  final bool narrow;

  /// The conversation.
  final Widget child;

  /// Puts what the panels draw under whatever they need from the conversation (the surfaces' environment).
  final Widget Function(Widget panels) wrap;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([layout, surfaces, registry]),
    builder: (context, _) => LayoutBuilder(builder: (context, constraints) {
      final focus = layout.focus;
      final rightOffered = registry.availableFor(session, SurfaceSlot.right);
      final bottomOffered = registry.availableFor(session, SurfaceSlot.bottom);
      final hasRight = rightOffered.isNotEmpty || surfaces.inSlot(SurfaceSlot.right).isNotEmpty;
      final hasBottom = bottomOffered.isNotEmpty || surfaces.inSlot(SurfaceSlot.bottom).isNotEmpty;
      final showRight = hasRight && (focus == PanelFocus.right || (layout.rightOpen && (focus == PanelFocus.none || narrow)));
      final showBottom = hasBottom && (focus == PanelFocus.bottom || (layout.bottomOpen && focus == PanelFocus.none));
      final maxRight = constraints.maxWidth / 2;
      final rightWidth = _within(layout.rightWidth, ShellLayout.rightMin, maxRight);
      final maxBottom = constraints.maxHeight * 0.6;
      final bottomHeight = _within(layout.bottomHeight, ShellLayout.bottomMin, maxBottom);

      Widget panel(SurfaceSlot slot, List<WorkSurface> offered) => WorkPanel(
        slot: slot, session: session, surfaces: surfaces, offered: offered, layout: layout, narrow: narrow && slot == SurfaceSlot.right,
      );

      // Each panel is built once it has anything open, and kept while hidden.
      final rightBuilt = surfaces.inSlot(SurfaceSlot.right).isNotEmpty || showRight;
      final bottomBuilt = surfaces.inSlot(SurfaceSlot.bottom).isNotEmpty || showBottom;
      final bottomAlone = focus == PanelFocus.bottom && showBottom;
      final rightAlone = showRight && (focus == PanelFocus.right || narrow);

      final main = Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Flexible(flex: bottomAlone ? 0 : 1, child: Offstage(offstage: bottomAlone, child: child)),
        if (bottomAlone)
          Expanded(child: panel(SurfaceSlot.bottom, bottomOffered))
        else if (bottomBuilt)
          Offstage(
            offstage: !showBottom,
            child: BottomPanel(layout: layout, height: bottomHeight, maxHeight: maxBottom, child: panel(SurfaceSlot.bottom, bottomOffered)),
          ),
      ]);

      final panels = Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Expanded(child: Offstage(offstage: rightAlone, child: main)),
        if (rightBuilt)
          Offstage(
            offstage: !showRight,
            child: rightAlone
                ? SizedBox(width: constraints.maxWidth, child: panel(SurfaceSlot.right, rightOffered))
                : Row(children: [
                    ResizeDivider(
                      label: 'Resize the right panel',
                      value: '${rightWidth.round()} pixels',
                      onDelta: (delta) => layout.setRightWidth(_within(layout.rightWidth, ShellLayout.rightMin, maxRight) - delta),
                      onEnd: () => layout.setRightWidth(_within(layout.rightWidth, ShellLayout.rightMin, maxRight), keep: true),
                      onReset: layout.resetRightWidth,
                    ),
                    SizedBox(width: rightWidth, child: panel(SurfaceSlot.right, rightOffered)),
                  ]),
          ),
      ]);
      return wrap(panels);
    }),
  );
}
