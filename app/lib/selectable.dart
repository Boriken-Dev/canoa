// Selectable text: every screen, dialog and sheet of the app is a selection area of its own, so any text in it can be
// dragged over, copied with Ctrl+C (Cmd+C on a Mac) and the context menu, and selected whole with Ctrl+A.
import 'dart:async';

import 'package:flutter/material.dart';

/// [child] as one selection area. Areas nested inside it (the conversation, the plan, the changes pane) stay apart from it:
/// a drag never runs from one into another, and each one's Select all is its own.
///
/// A dialog, sheet or page is its own area, never part of the screen under it, so Select all inside a dialog does not
/// reach the text behind it. Menus and tooltips are left out: a menu item is a command, and a drag there would not select.
class Selectable extends StatelessWidget {
  const Selectable({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => SelectionArea(child: child);
}

/// Like [showDialog], with the dialog's text selectable.
Future<T?> showSelectableDialog<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  bool barrierDismissible = true,
}) => showDialog<T>(
  context: context,
  barrierDismissible: barrierDismissible,
  builder: (context) => _DialogArea(dismissible: barrierDismissible, child: builder(context)),
);

/// A dialog's selection area. A dialog fills its route, so the area also lies over the barrier and wins its taps; a tap
/// that starts off the dialog's panel and ends near where it began closes the route here, as the barrier would have.
class _DialogArea extends StatefulWidget {
  const _DialogArea({required this.dismissible, required this.child});

  final bool dismissible;
  final Widget child;

  @override
  State<_DialogArea> createState() => _DialogAreaState();
}

class _DialogAreaState extends State<_DialogArea> {
  /// How far, in logical pixels, a press may move and still be a tap.
  static const double _slop = 18;

  /// Set by the panel's own listener, which a pointer reaches before the one around it.
  bool _onPanel = false;
  Offset? _outsideAt;

  @override
  Widget build(BuildContext context) => Listener(
    behavior: HitTestBehavior.translucent,
    onPointerDown: (event) {
      _outsideAt = _onPanel ? null : event.position;
      _onPanel = false;
    },
    onPointerUp: (event) {
      final from = _outsideAt;
      _outsideAt = null;
      if (from == null || !widget.dismissible || (event.position - from).distance > _slop) return;
      // The barrier may close it too: only the route still on top is closed.
      if (ModalRoute.of(context)?.isCurrent ?? false) unawaited(Navigator.of(context).maybePop());
    },
    child: Selectable(
      child: Listener(
        onPointerDown: (_) => _onPanel = true,
        child: widget.child,
      ),
    ),
  );
}

/// A page over the current one, its text selectable.
Route<T> selectablePage<T>({required WidgetBuilder builder}) =>
    MaterialPageRoute<T>(builder: (context) => Selectable(child: builder(context)));
