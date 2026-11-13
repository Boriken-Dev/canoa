import 'dart:async';

import 'package:coqui_acp/coqui_acp.dart' show AcpSessionInfo, AcpSessionPage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../selectable.dart';
import '../session_store.dart';
import '../theme/look.dart';
import 'remember_dialog.dart';

/// What Canoa calls itself in the sentences of the package's cards.
const String appName = 'Canoa';

/// A session's entries as the transcript reads them.
extension SessionEntries on ChatSession {
  List<ChatEntryData> get entries => messages;
}

/// What an agent is waiting on in [session], with the answers it offers and, where a folder is known, Canoa's own "Always allow
/// here" beside them. The package draws the card; the rules are Canoa's.
class SessionPermissionCard extends StatelessWidget {
  const SessionPermissionCard({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final question = session.pendingPermissions.first;
    final small = Theme.of(context).textTheme.labelSmall;
    final agent = shortLine(session.agent) ?? 'The agent';
    final folder = session.cwd ?? '';
    final canRemember = question.subject != null && folder.isNotEmpty;
    return PermissionCard(
      agent: session.agent,
      title: question.title,
      detail: question.detail,
      options: question.options,
      bypassPassed: question.bypassPassed,
      forPerson: question.forPerson,
      more: session.pendingPermissions.length - 1,
      host: appName,
      onChoose: (option) => store.choosePermission(session, option),
      onCancel: () => store.cancelPermission(session),
      notes: [
        if (canRemember)
          Text(
            'Always allow here: Canoa itself answers allow once to what you confirm, for $agent in $folder, until you revoke '
            'it in Settings, Permissions.',
            style: small,
          ),
      ],
      extraAnswers: [
        if (canRemember)
          FilledButton.tonal(
            key: const Key('always-allow-here'),
            onPressed: () => showDialog<bool>(
              context: context,
              builder: (_) => RememberDialog(store: store, session: session, question: question),
            ),
            child: Text('Always allow here: ${folderName(folder)}', maxLines: 2, overflow: TextOverflow.ellipsis),
          ),
      ],
    );
  }
}

/// The oldest question [session]'s agent is waiting on, as a form.
class SessionQuestionCard extends StatelessWidget {
  const SessionQuestionCard({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) => QuestionCard(
    request: session.pendingQuestions.first.request,
    agent: session.agent,
    more: session.pendingQuestions.length - 1,
    host: appName,
    onAnswer: (answer) => store.answerQuestion(session, answer),
  );
}

/// The oldest secret prompt the host has for [session]: a form whose secret fields are sealed to the host's key, or values it
/// shows. What is typed or shown stays in the card and the answer goes to the host's connection, never into the conversation.
class SessionSecretCard extends StatelessWidget {
  const SessionSecretCard({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final prompt = session.pendingSecrets.first;
    final more = session.pendingSecrets.length - 1;
    final environment = store.secretEnvironment(prompt);
    return switch (prompt) {
      SecretRequestPrompt() => SecretRequestCard(
        key: ValueKey('secret-${prompt.request.context.sessionId}-${prompt.request.context.requestId}'),
        request: prompt.request,
        environment: environment,
        agent: session.agent,
        more: more,
        onReply: (reply) => store.answerSecret(session, prompt, reply),
      ),
      SecretShowPrompt() => SecretShowCard(
        key: ValueKey('secret-${prompt.open.context.sessionId}-${prompt.open.context.requestId}'),
        open: prompt.open,
        environment: environment,
        deliver: prompt.deliver,
        agent: session.agent,
        more: more,
        onReady: (reply) => store.readySecretShow(session, prompt, reply),
        onDone: (shown) => store.finishSecretShow(session, prompt, shown),
      ),
    };
  }
}

/// The messages [session] holds for when its turn ends.
class SessionQueuePanel extends StatelessWidget {
  const SessionQueuePanel({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) => QueuePanel(
    lines: [for (final message in session.queue) QueuedLine(message, message.text, attachments: [for (final item in message.attachments) item.name])],
    held: session.queueHeld,
    working: session.status == SessionStatus.working,
    onEdit: (index, text) => store.editQueued(session, session.queue[index], text),
    onRemove: (index) => store.removeQueued(session, session.queue[index]),
    onClear: () => store.removeQueued(session),
    onSendNext: () => store.sendQueued(session),
    dialog: (context, builder) => showSelectableDialog<String>(context: context, builder: builder),
  );
}

/// What [session]'s tools changed, as the agent reported it.
List<FileChange> sessionChanges(ChatSession session) => changesOf(session.entries);

/// The changes of [session], by file, with the person's marks of viewed kept in the session.
class SessionChangesPane extends StatelessWidget {
  const SessionChangesPane({super.key, required this.store, required this.session, required this.onComment, this.onClose});

  final SessionStore store;
  final ChatSession session;

  /// Adds text to the message being written.
  final ValueChanged<String> onComment;
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) => ChangesPane(
    files: sessionChanges(session),
    isViewed: (file) => session.viewedChanges[file.path] == file.version,
    onViewed: (file, on) => store.markViewed(session, file.path, on ? file.version : null),
    onComment: onComment,
    onClose: onClose,
    color: context.look.rail,
    dialog: (context, builder) => showSelectableDialog<String>(context: context, builder: builder),
  );
}

/// What an agent says it has from before, for a person to pick one. Titles are the agent's, and the list asks for more only when told.
class EarlierConversations extends StatefulWidget {
  const EarlierConversations({super.key, required this.list});

  final Future<AcpSessionPage> Function(String? cursor) list;

  @override
  State<EarlierConversations> createState() => _EarlierConversationsState();
}

class _EarlierConversationsState extends State<EarlierConversations> {
  final List<AcpSessionInfo> _found = [];
  String? _cursor;
  bool _loading = true;
  String? _problem;

  @override
  void initState() {
    super.initState();
    unawaited(_more());
  }

  Future<void> _more() async {
    setState(() {
      _loading = true;
      _problem = null;
    });
    try {
      final page = await widget.list(_cursor);
      if (!mounted) return;
      setState(() {
        _found.addAll(page.sessions);
        _cursor = page.nextCursor;
        _loading = false;
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _problem = 'This connection does not offer that.';
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Earlier conversations'),
      content: SizedBox(
        width: 520,
        height: 420,
        child: Column(children: [
          if (_problem != null)
            Padding(padding: const EdgeInsets.all(8), child: Text(_problem!))
          else if (!_loading && _found.isEmpty)
            const Padding(padding: EdgeInsets.all(8), child: Text('The agent has none here.')),
          Expanded(
            child: ListView.builder(
              itemCount: _found.length,
              itemBuilder: (context, index) {
                final earlier = _found[index];
                final about = [shortLine(earlier.updatedAt), shortLine(earlier.cwd)].whereType<String>().join(' · ');
                return ListTile(
                  title: Text(shortLine(earlier.title) ?? 'Untitled', maxLines: 2, overflow: TextOverflow.ellipsis),
                  subtitle: about.isEmpty ? null : Text(about, maxLines: 1, overflow: TextOverflow.ellipsis),
                  onTap: () => Navigator.of(context).pop(earlier),
                );
              },
            ),
          ),
          if (_loading) const Padding(padding: EdgeInsets.all(8), child: CircularProgressIndicator()),
          if (!_loading && _cursor != null)
            TextButton(onPressed: () => unawaited(_more()), child: const Text('More')),
        ]),
      ),
      actions: [TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Close'))],
    );
  }
}
