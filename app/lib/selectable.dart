// Selectable text: every screen, dialog and sheet of the app is a selection area of its own, so any text in it can be
// dragged over, copied with Ctrl+C (Cmd+C on a Mac) and the context menu, and selected whole with Ctrl+A.
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
  builder: (context) => Selectable(child: builder(context)),
);

/// A page over the current one, its text selectable.
Route<T> selectablePage<T>({required WidgetBuilder builder}) =>
    MaterialPageRoute<T>(builder: (context) => Selectable(child: builder(context)));
