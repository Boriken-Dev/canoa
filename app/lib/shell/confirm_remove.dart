import 'package:flutter/material.dart';

import '../selectable.dart';

/// Asks before something the person set up is taken away: `title` says what, `body` (or `content`) what is taken out and what
/// stays as it is. True when the person confirms, false on cancel or on dismissing the dialog.
Future<bool> confirmRemoval(
  BuildContext context, {
  required String title,
  String? body,
  Widget? content,
  String confirmLabel = 'Remove',
  String cancelLabel = 'Keep it',
  Key? dialogKey = const Key('confirm-remove'),
  Key? cancelKey = const Key('confirm-remove-no'),
  Key? confirmKey = const Key('confirm-remove-yes'),
  bool selectable = false,
}) async {
  assert(body != null || content != null);
  Widget dialog(BuildContext context) => AlertDialog(
    key: dialogKey,
    title: Text(title),
    content: content ?? Text(body!),
    actions: [
      TextButton(key: cancelKey, onPressed: () => Navigator.pop(context, false), child: Text(cancelLabel)),
      FilledButton(key: confirmKey, onPressed: () => Navigator.pop(context, true), child: Text(confirmLabel)),
    ],
  );
  final confirmed = selectable
      ? await showSelectableDialog<bool>(context: context, builder: dialog)
      : await showDialog<bool>(context: context, builder: dialog);
  return confirmed ?? false;
}
