import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../session_store.dart';
import 'session_views.dart';

/// The words that open a side chat from the message box: `/btw` and the question after it.
final RegExp sideCommand = RegExp(r'^/btw(?:\s+([\s\S]*))?$', caseSensitive: false);

/// The question [text] puts as a side chat (empty when it names none), or null when [text] is no such command.
String? sideQuestionOf(String text) {
  final match = sideCommand.firstMatch(text.trim());
  return match == null ? null : (match.group(1) ?? '').trim();
}

/// Opens the side chat on [parent], with [question] already asked when it is not empty.
Future<void> showSideChat(
  BuildContext context, {
  required SessionStore store,
  required ChatSession parent,
  String question = '',
  ChatRenderers renderers = ChatRenderers.none,
}) => showDialog<void>(
  context: context,
  builder: (context) => SideChatDialog(store: store, parent: parent, question: question, renderers: renderers),
);

/// A small conversation with the agent of [parent] about [parent], on the right of the window. It is a session of its own, in no
/// list and not in the ledger, that ends with the dialog; [parent] never hears of it. What the agent asks of the person it asks
/// here, by the rules the person set.
class SideChatDialog extends StatefulWidget {
  const SideChatDialog({super.key, required this.store, required this.parent, this.question = '', this.renderers = ChatRenderers.none});

  final SessionStore store;
  final ChatSession parent;
  final String question;
  final ChatRenderers renderers;

  @override
  State<SideChatDialog> createState() => _SideChatDialogState();
}

class _SideChatDialogState extends State<SideChatDialog> {
  final _input = TextEditingController();
  final _focus = FocusNode();
  ChatSession? _side;
  String? _problem;

  @override
  void initState() {
    super.initState();
    if (widget.question.trim().isNotEmpty) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _ask(widget.question));
    }
  }

  @override
  void dispose() {
    final side = _side;
    // Closing tells the store's listeners, which cannot happen while the tree is being taken down.
    if (side != null) unawaited(Future.microtask(() => widget.store.close(side)));
    _input.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _ask(String question) {
    final text = question.trim();
    if (text.isEmpty) return;
    final side = _side ??= widget.store.startSide(widget.parent, context.t('side.session', {'title': widget.parent.title}));
    _input.clear();
    setState(() => _problem = null);
    unawaited(
      widget.store.askSide(widget.parent, side, text).catchError((Object error) {
        if (mounted) setState(() => _problem = reasonOf(error));
      }),
    );
    _focus.requestFocus();
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final side = _side;
    return Align(
      alignment: Alignment.centerRight,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480),
        child: Dialog(
          key: const Key('side-chat'),
          insetPadding: const EdgeInsets.all(12),
          child: ListenableBuilder(
            listenable: widget.store,
            builder: (context, _) => Column(children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 12, 8, 4),
                child: Row(children: [
                  Expanded(
                    child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                      Text(context.t('side.title'), style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600)),
                      Text(
                        context.t('side.about', {'title': widget.parent.title}),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: look.muted, fontSize: 12),
                      ),
                    ]),
                  ),
                  IconButton(key: const Key('side-close'), tooltip: context.t('side.close'), icon: const Icon(Icons.close), onPressed: () => Navigator.pop(context)),
                ]),
              ),
              Expanded(
                child: side == null
                    ? Center(
                        child: Padding(
                          padding: const EdgeInsets.all(24),
                          child: Text(context.t('side.empty'), textAlign: TextAlign.center, style: TextStyle(color: look.muted)),
                        ),
                      )
                    : SelectionContainer.disabled(
                        child: ChatTranscript(
                          entries: [for (final message in side.messages) ModelEntry(message)],
                          terminals: side.terminals,
                          plan: side.plan,
                          working: side.status == SessionStatus.working || widget.store.isStarting(side),
                          agentName: side.agent,
                          renderers: widget.renderers,
                          narrow: true,
                        ),
                      ),
              ),
              if (side != null && side.pendingPermissions.isNotEmpty) SessionPermissionCard(store: widget.store, session: side),
              if (_problem != null)
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
                  child: Text(context.t('side.failed', {'why': _problem}), key: const Key('side-problem'), style: TextStyle(color: look.danger, fontSize: 12)),
                ),
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 4, 12, 12),
                child: Row(children: [
                  Expanded(
                    child: TextField(
                      key: const Key('side-input'),
                      controller: _input,
                      focusNode: _focus,
                      autofocus: true,
                      minLines: 1,
                      maxLines: 5,
                      textInputAction: TextInputAction.send,
                      decoration: InputDecoration(hintText: context.t('side.hint'), border: const OutlineInputBorder(), isDense: true),
                      onSubmitted: _ask,
                    ),
                  ),
                  const SizedBox(width: 8),
                  IconButton.filled(key: const Key('side-send'), tooltip: context.t('side.send'), icon: const Icon(Icons.send), onPressed: () => _ask(_input.text)),
                ]),
              ),
            ]),
          ),
        ),
      ),
    );
  }
}
