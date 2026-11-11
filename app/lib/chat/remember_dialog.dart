import 'package:flutter/material.dart';

import '../permissions/permission_rules.dart';
import '../session_store.dart';
import '../theme/look.dart';

/// The last part of [folder], which is how a person names it.
String folderName(String folder) {
  final parts = folder.replaceAll('\\', '/').split('/').where((part) => part.isNotEmpty);
  return parts.isEmpty ? folder : parts.last;
}

/// Asks for the scope of "Always allow here" before it is kept: what exactly Canoa will allow, which the person reads and edits.
/// Nothing is remembered until they confirm, and a rule has to cover the very request that is waiting.
class RememberDialog extends StatefulWidget {
  const RememberDialog({super.key, required this.store, required this.session, required this.question});

  final SessionStore store;
  final ChatSession session;
  final PermissionQuestion question;

  @override
  State<RememberDialog> createState() => _RememberDialogState();
}

class _RememberDialogState extends State<RememberDialog> {
  late final RuleSubject subject = widget.question.subject!;
  late final RuleType type = switch (subject) {
    CommandSubject() => RuleType.command,
    PathSubject() => RuleType.path,
    McpSubject() => RuleType.mcp,
  };
  late final TextEditingController text = TextEditingController(
    text: switch (subject) {
      CommandSubject(:final command) => command,
      PathSubject(:final path) => path,
      McpSubject(:final name) => name,
    },
  );
  String? problem;

  @override
  void dispose() {
    text.dispose();
    super.dispose();
  }

  String get value => switch ((type, subject)) {
    (RuleType.command, CommandSubject(:final command)) => command,
    (RuleType.mcp, McpSubject(:final name)) => name,
    _ => text.text,
  };

  void confirm() {
    final failed = widget.store.rememberHere(widget.session, widget.question, type, value);
    if (failed == null) {
      Navigator.pop(context, true);
    } else {
      setState(() => problem = failed);
    }
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.session;
    final folder = session.cwd ?? '';
    final source = session.agent;
    final label = Theme.of(context).textTheme.labelMedium;
    final mono = const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12);
    final fields = <Widget>[
      switch (subject) {
        CommandSubject(:final command) => Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('Only this command, exactly:', style: label),
          const SizedBox(height: 4),
          SelectableText(command, key: const Key('rule-exact'), style: mono),
        ]),
        PathSubject() => TextField(
          key: const Key('rule-value'),
          controller: text,
          style: mono,
          onChanged: (_) => setState(() => problem = null),
          decoration: InputDecoration(
            labelText: 'This file, or this folder and what is under it',
            helperText: 'An absolute path inside $folder. Never a hidden file or folder, build scripts, agent instructions or Canoa\'s own files.',
          ),
        ),
        McpSubject(:final server, :final tool) => Text('Any use of the tool $tool of $server, with any input.'),
      },
      if (problem != null)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(problem!, key: const Key('rule-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ),
    ];
    return AlertDialog(
      title: Text('Always allow here: ${folderName(folder)}'),
      content: SizedBox(
        width: 440,
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            Text('For $source, in $folder.', style: label),
            const SizedBox(height: 8),
            ...fields,
            const SizedBox(height: 12),
            if (subject is CommandSubject)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: Text(
                  'A command runs whatever the files in this folder make it do: a test or build command runs the scripts and tests '
                  'there, as they are when it runs. Only one of commands or file changes and tools can be allowed for this source in '
                  'this folder, or one next to it, at a time.',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(fontWeight: FontWeight.w600),
                ),
              ),
            Text(
              'Limits: a hard link is not detected; the list of files never covered cannot be complete; and what the agent has '
              'itself been set to allow (its own permissions, or rules it remembered) is outside what Canoa can see, and can make '
              'Manual silent.',
              key: const Key('rule-limits'),
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 8),
            Text(
              'Canoa then answers a matching request with allow once, itself, without asking you, until you revoke the rule in '
              'Settings, Permissions. It never answers an "always", a request from another source or folder, or one the agent '
              'offers no allow once for.',
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(key: const Key('rule-confirm'), onPressed: confirm, child: const Text('Always allow here')),
      ],
    );
  }
}
