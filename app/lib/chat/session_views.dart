import 'dart:async';

import 'package:coqui_acp/coqui_acp.dart' show AcpPermissionOption, AcpSessionInfo, AcpSessionPage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/material.dart';

import '../highlight/highlighted.dart';
import '../selectable.dart';
import '../session_store.dart';
import '../theme/look.dart';
import 'remember_dialog.dart';

/// What Canoa calls itself in the sentences of the package's cards.
const String appName = 'Canoa';

/// A session's entries as the transcript reads them.
extension SessionEntries on ChatSession {
  List<ChatEntryData> get entries => [for (final message in messages) ModelEntry(message)];
}

/// The keys that answer a card from anywhere in the window (`chord` in main.dart binds them), as this platform writes them.
Map<String, String> get answerKeys {
  final mac = defaultTargetPlatform == TargetPlatform.macOS || defaultTargetPlatform == TargetPlatform.iOS;
  return mac ? const {'reject_once': '\u2318\u21E7R', 'allow_once': '\u2318\u21E7A'} : const {'reject_once': 'Ctrl+Shift+R', 'allow_once': 'Ctrl+Shift+A'};
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
    final task = question.task;
    final isPlan = question.forPerson && question.diff == null && (question.detail?.trim().isNotEmpty ?? false);
    if (isPlan) {
      return _PlanApprovalCard(
        agent: task == null ? session.agent : (shortLine(task.source) ?? session.agent),
        title: question.title,
        reason: question.reason,
        planMarkdown: question.detail!,
        options: question.options,
        more: session.pendingPermissions.length - 1,
        answerHints: answerKeys,
        onChoose: (option) => store.choosePermission(session, option),
        onCancel: () => store.cancelPermission(session),
        notes: [
          if (task != null)
            Text(
              'Asked by a subagent: ${task.title} (${task.source}).',
              key: const Key('task-asks'),
              style: small,
            ),
        ],
      );
    }
    return PermissionCard(
      agent: task == null ? session.agent : (shortLine(task.source) ?? session.agent),
      title: question.title,
      reason: question.reason,
      detail: question.detail,
      diff: question.diff,
      options: question.options,
      bypassPassed: question.bypassPassed,
      forPerson: question.forPerson,
      more: session.pendingPermissions.length - 1,
      host: appName,
      answerHints: answerKeys,
      onChoose: (option) => store.choosePermission(session, option),
      onCancel: () => store.cancelPermission(session),
      notes: [
        if (task != null)
          Text(
            'Asked by a subagent: ${task.title} (${task.source}). Allow for the session covers this subagent only.',
            key: const Key('task-asks'),
            style: small,
          ),
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

/// Renders a plan awaiting person approval with Markdown formatting, a generous scroll height cap (420 px), and plan-worded actions.
class _PlanApprovalCard extends StatelessWidget {
  const _PlanApprovalCard({
    required this.agent,
    required this.title,
    required this.planMarkdown,
    required this.options,
    required this.onChoose,
    this.reason,
    this.more = 0,
    this.answerHints = const {},
    this.onCancel,
    this.notes = const [],
  });

  static const double maxPlanHeight = 420;

  final String agent;
  final String title;
  final String? reason;
  final String planMarkdown;
  final List<AcpPermissionOption> options;
  final ValueChanged<AcpPermissionOption> onChoose;
  final int more;
  final Map<String, String> answerHints;
  final VoidCallback? onCancel;
  final List<Widget> notes;

  static String _labelFor(AcpPermissionOption option) {
    final lower = option.name.toLowerCase();
    if (option.kind.startsWith('allow') && (lower == 'allow' || lower == 'allow once' || lower == 'yes' || lower == 'approve')) {
      return 'Approve plan';
    }
    if (option.rejects && (lower == 'deny' || lower == 'no' || lower == 'reject' || lower == 'reject once')) {
      return 'Keep editing';
    }
    return shortLine(option.name) ?? option.kind;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final look = context.chatLook;
    final allow = options.where((option) => option.kind.startsWith('allow')).firstOrNull;
    final deny = options.where((option) => option.rejects).firstOrNull;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 4, 12, 4),
      child: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Card(
            key: const Key('permission-card'),
            child: Padding(
              padding: const EdgeInsets.all(14),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Row(children: [
                    const Chip(key: Key('needs-input'), label: Text('Plan approval'), visualDensity: VisualDensity.compact),
                    const SizedBox(width: 8),
                    Expanded(child: Text(title, style: theme.textTheme.titleSmall)),
                    if (more > 0) Text('+$more more', style: theme.textTheme.labelSmall),
                  ]),
                  if (reason != null && reason!.trim().isNotEmpty)
                    Padding(padding: const EdgeInsets.only(top: 4), child: Text(reason!, style: theme.textTheme.bodySmall)),
                  const SizedBox(height: 8),
                  Container(
                    key: const Key('permission-detail'),
                    constraints: const BoxConstraints(maxHeight: maxPlanHeight),
                    decoration: BoxDecoration(
                      color: look.code,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: look.border),
                    ),
                    padding: const EdgeInsets.all(12),
                    child: SingleChildScrollView(child: AgentMarkdown(text: planMarkdown, builders: highlightedMarkdown)),
                  ),
                  if (notes.isNotEmpty) ...[const SizedBox(height: 8), ...notes],
                  const SizedBox(height: 10),
                  Wrap(spacing: 8, runSpacing: 8, children: [
                    if (allow != null)
                      FilledButton(
                        key: Key('answer-${allow.kind}'),
                        onPressed: () => onChoose(allow),
                        child: Text([
                          _labelFor(allow),
                          if (answerHints[allow.kind] case final hint?) '($hint)',
                        ].join(' ')),
                      ),
                    if (deny != null)
                      OutlinedButton(
                        key: Key('answer-${deny.kind}'),
                        onPressed: () => onChoose(deny),
                        child: Text([
                          _labelFor(deny),
                          if (answerHints[deny.kind] case final hint?) '($hint)',
                        ].join(' ')),
                      ),
                    for (final extra in options)
                      if (extra != allow && extra != deny)
                        TextButton(
                          key: Key('answer-${extra.kind}'),
                          onPressed: () => onChoose(extra),
                          child: Text(_labelFor(extra)),
                        ),
                    if (options.isEmpty && onCancel != null)
                      OutlinedButton(onPressed: onCancel, child: const Text('Dismiss')),
                  ]),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// What the person is offered when the account [session] runs on has run out: a line saying so and a button for each account that
/// could take the conversation. Nothing moves until one is pressed.
class SessionAccountOffer extends StatelessWidget {
  const SessionAccountOffer({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final offer = session.accountOffer;
    if (offer == null) return const SizedBox.shrink();
    final look = context.chatLook;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 4, 12, 4),
      child: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Container(
            key: const Key('account-offer'),
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              border: Border.all(color: look.muted.withValues(alpha: 0.35)),
              borderRadius: BorderRadius.circular(look.radius),
            ),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(
                'Account ${offer.account.isEmpty ? "this machine's login" : offer.account} has run out${offer.left == null ? '' : ' (${offer.left})'}. Move this conversation to another account?',
                key: const Key('account-offer-text'),
              ),
              const SizedBox(height: 8),
              Wrap(spacing: 8, runSpacing: 8, children: [
                for (final other in offer.alternatives)
                  FilledButton.tonal(
                    key: Key('account-offer-move $other'),
                    onPressed: () => unawaited(store.takeAccountOffer(session, other)),
                    child: Text('Move to $other'),
                  ),
                TextButton(
                  key: const Key('account-offer-dismiss'),
                  onPressed: () => store.dismissAccountOffer(session),
                  child: const Text('Not now'),
                ),
              ]),
            ]),
          ),
        ),
      ),
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
  const SessionQueuePanel({super.key, required this.store, required this.session, required this.toComposer});

  final SessionStore store;
  final ChatSession session;

  /// Puts the text of a message taken out of the queue in the composer, after what is written there.
  final void Function(String text) toComposer;

  void _say(BuildContext context, String? problem) {
    if (problem != null) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(problem)));
  }

  /// Runs [action], which sends something, saying why in a snackbar if the agent did not take it.
  void _try(BuildContext context, Future<void> Function() action, String what) {
    final messenger = ScaffoldMessenger.of(context);
    unawaited(action().catchError((Object error) {
      messenger.showSnackBar(SnackBar(content: Text('$what: ${reasonOf(error)}')));
    }));
  }

  @override
  Widget build(BuildContext context) => QueuePanel(
    lines: [for (final message in session.queue) QueuedLine(message, message.text, attachments: [for (final item in message.attachments) item.name])],
    held: session.queueHeld,
    working: session.status == SessionStatus.working,
    merged: store.queueDelivery == QueueDelivery.merge,
    onEdit: (index, text) => store.editQueued(session, session.queue[index], text),
    onRemove: (index) => store.removeQueued(session, session.queue[index]),
    onClear: () => store.removeQueued(session),
    onSendNext: () => _try(context, () => store.sendQueued(session), 'Not sent'),
    onMergeSend: () => _try(context, () => store.sendMerged(session), 'Not sent'),
    onMergeAll: () => _say(context, store.mergeAllQueued(session)),
    onSteer: (index) => _try(context, () => store.steerQueued(session, session.queue[index]), 'Could not steer'),
    onMerge: (index) => _say(context, store.mergeQueued(session, session.queue[index])),
    onToComposer: (index) => _say(context, store.takeQueued(session, session.queue[index], into: toComposer)),
    onMove: (index, by) => store.moveQueued(session, session.queue[index], by),
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
