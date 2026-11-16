import 'dart:async';

import 'package:flutter/material.dart';

import '../selectable.dart';
import 'open_in.dart';

/// Runs [action] on [target]; a failure is a snackbar and changes nothing else, so a draft being written stays as it is.
Future<void> runWorkspaceAction(BuildContext context, WorkspaceAction action, WorkspaceTarget target) async {
  final messenger = ScaffoldMessenger.maybeOf(context);
  try {
    await action.run(target);
  } on OpenInFailure catch (failure) {
    messenger?.showSnackBar(SnackBar(content: Text('${action.label}: ${failure.message}')));
  }
}

/// What can be done with a session's folder, or why nothing can: its own dialog, so the header carries one button for it.
Future<void> showWorkspaceActions(
  BuildContext context, {
  required WorkspaceOpener opener,
  required WorkspaceTarget? target,
  required String? reason,
}) {
  final actions = target == null ? const <WorkspaceAction>[] : opener.actionsFor(WorkspaceContext.header, target);
  return showSelectableDialog<void>(
    context: context,
    builder: (dialog) => SimpleDialog(
      title: const Text('Open the workspace'),
      children: [
        if (target != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
            child: Text(target.path, style: Theme.of(dialog).textTheme.bodySmall),
          ),
        if (reason != null) Padding(padding: const EdgeInsets.fromLTRB(24, 0, 24, 16), child: Text(reason)),
        for (final action in actions)
          SimpleDialogOption(
            onPressed: () {
              Navigator.pop(dialog);
              unawaited(runWorkspaceAction(context, action, target!));
            },
            child: Row(children: [Icon(action.icon), const SizedBox(width: 12), Text(action.label)]),
          ),
      ],
    ),
  );
}
