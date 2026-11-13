import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/experimental.dart' show ChatSessionView;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:flutter/material.dart';

import '../chat/session_views.dart' show appName;
import '../theme/look.dart';

/// A conversation in a window of its own: the transcript, what the agent waits on, and a composer, drawn over a
/// [ChatSessionView] of the conversation the main window holds. It keeps nothing of the conversation itself; what the person does
/// here is asked of the main window, and what comes of it, or of an answer given there, arrives as the next state.
class PopoutChat extends StatefulWidget {
  const PopoutChat({super.key, required this.view, this.onDock, this.renderers = chat.ChatRenderers.none});

  final ChatSessionView view;

  /// Called after the view has detached, to close the window.
  final VoidCallback? onDock;

  final chat.ChatRenderers renderers;

  @override
  State<PopoutChat> createState() => _PopoutChatState();
}

class _PopoutChatState extends State<PopoutChat> {
  final _input = TextEditingController();
  final _focus = FocusNode();
  StreamSubscription<model.ChatEvent>? _events;
  String? _problem;

  model.ChatSession get _session => widget.view.session;

  @override
  void initState() {
    super.initState();
    _session.addListener(_changed);
    _events = _session.events.listen(_event);
    unawaited(widget.view.done.then((_) => _changed()));
  }

  @override
  void dispose() {
    _session.removeListener(_changed);
    unawaited(_events?.cancel());
    _input.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  /// Says who answered a request this window was showing, when that was not this window.
  void _event(model.ChatEvent event) {
    if (event is! model.PermissionSettledEvent || !mounted) return;
    final outcome = event.question.outcome;
    final by = outcome?.by;
    if (outcome == null || outcome.how != model.Settlement.answered || by == null || by == widget.view.clientId) return;
    final name = _session.clients.where((c) => c.id == by).firstOrNull?.name ?? by;
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text('Answered in $name.')));
  }

  Future<void> _send() async {
    final text = _input.text.trim();
    if (text.isEmpty) return;
    _input.clear();
    try {
      await widget.view.send(text);
      if (mounted) setState(() => _problem = null);
    } catch (error) {
      if (!mounted) return;
      _input.text = text;
      setState(() => _problem = 'Not sent: ${error is StateError ? error.message : error}');
    }
  }

  Future<void> _dock() async {
    await widget.view.detach();
    widget.onDock?.call();
  }

  @override
  Widget build(BuildContext context) {
    final session = _session;
    final view = widget.view;
    final working = session.status == model.ChatStatus.working;
    final waiting = session.pendingPermissions.firstOrNull;
    final asking = session.pendingQuestions.firstOrNull;
    final agent = chat.shortLine(session.agent) ?? 'The agent';
    return Scaffold(
      body: Column(children: [
        Material(
          color: context.look.bar,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Row(children: [
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600)),
                  Text([agent, if (session.cwd != null) session.cwd!].join(' · '),
                      maxLines: 1, overflow: TextOverflow.ellipsis, style: Theme.of(context).textTheme.bodySmall),
                ]),
              ),
              IconButton(
                key: const Key('dock-back'),
                tooltip: 'Bring back to the main window',
                icon: const Icon(Icons.close_fullscreen),
                onPressed: view.isOpen ? () => unawaited(_dock()) : null,
              ),
            ]),
          ),
        ),
        if (!view.isOpen)
          MaterialBanner(
            content: const Text('The main window no longer holds this conversation.'),
            actions: [TextButton(onPressed: widget.onDock, child: const Text('Close'))],
          ),
        Expanded(
          child: chat.ChatTranscript(
            entries: [for (final message in session.messages) chat.ModelEntry(message)],
            terminals: session.terminals,
            plan: session.plan,
            working: working,
            renderers: widget.renderers,
          ),
        ),
        if (session.queue.isNotEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Text(
              session.queue.length == 1 ? '1 message waits for this turn to end.' : '${session.queue.length} messages wait for this turn to end.',
              key: const Key('popout-queue'),
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        if (waiting != null)
          chat.PermissionCard(
            key: ValueKey('permission-${waiting.id}'),
            agent: session.agent,
            title: waiting.title,
            detail: waiting.detail,
            options: waiting.options,
            bypassPassed: waiting.bypassPassed,
            forPerson: waiting.forPerson,
            more: session.pendingPermissions.length - 1,
            host: appName,
            onChoose: (option) => view.choosePermission(waiting, option),
            onCancel: () => view.choosePermission(waiting, null),
          ),
        if (asking != null)
          chat.QuestionCard(
            key: ValueKey('question-${asking.id}'),
            request: asking.request,
            agent: session.agent,
            more: session.pendingQuestions.length - 1,
            host: appName,
            onAnswer: (answer) => view.answerQuestion(asking, answer),
          ),
        if (_problem != null)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Text(_problem!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        Container(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
          child: Center(
            heightFactor: 1,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.chatLook.messageWidth + 100),
              child: chat.Composer(
                controller: _input,
                focusNode: _focus,
                onSend: () => unawaited(_send()),
                enabled: view.isOpen && session.status != model.ChatStatus.disconnected,
                working: working,
                onStop: view.isOpen ? view.cancel : null,
                commands: session.commands,
              ),
            ),
          ),
        ),
      ]),
    );
  }
}
