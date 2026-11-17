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
        if (target != null && opener.note != null)
          Padding(padding: const EdgeInsets.fromLTRB(24, 0, 24, 8), child: Text(opener.note!, style: Theme.of(dialog).textTheme.bodySmall)),
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

/// Asks for an address and opens it in the system's browser; a refusal is a snackbar, as for the other actions.
Future<void> askToOpenInBrowser(BuildContext context, WorkspaceOpener opener) async {
  final address = await showSelectableDialog<String>(
    context: context,
    builder: (dialog) => _AddressDialog(onOpen: (text) => Navigator.pop(dialog, text)),
  );
  if (address == null || address.trim().isEmpty || !context.mounted) return;
  final action = opener.actions.firstWhere((each) => each.id == 'browser');
  await runWorkspaceAction(context, action, WorkspaceTarget(address.trim()));
}

class _AddressDialog extends StatefulWidget {
  const _AddressDialog({required this.onOpen});

  final void Function(String text) onOpen;

  @override
  State<_AddressDialog> createState() => _AddressDialogState();
}

class _AddressDialogState extends State<_AddressDialog> {
  final _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Open in browser'),
    content: TextField(
      key: const ValueKey('browser-address'),
      controller: _text,
      autofocus: true,
      keyboardType: TextInputType.url,
      decoration: const InputDecoration(labelText: 'Web address', hintText: 'https://example.com'),
      onSubmitted: widget.onOpen,
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(key: const ValueKey('browser-open'), onPressed: () => widget.onOpen(_text.text), child: const Text('Open')),
    ],
  );
}
