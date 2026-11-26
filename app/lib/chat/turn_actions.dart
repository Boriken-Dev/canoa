import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../session_store.dart';

/// What the person can do with one of their own messages once the agent has answered it. Both go on from before that message in a
/// new session; the conversation they leave is as it was, and the files on disk are not touched.
final class TurnActions {
  const TurnActions({required this.rewind, required this.resend});

  /// Goes back to before the person's message number [turn] (from 0) of [session], whose text is [text]; whether it did, the
  /// person having been told why not.
  final Future<bool> Function(ChatSession session, int turn, String text)
  rewind;

  /// Goes back as [rewind] does, then sends [edited] in its place; whether it went back.
  final Future<bool> Function(
    ChatSession session,
    int turn,
    String text,
    String edited,
  )
  resend;
}

/// The number, from 0, of the person's message [entry] among the messages of [session], or null when [entry] is none of them.
int? turnOf(ChatSession session, chat.ChatEntryData entry) {
  if (entry.role != MessageRole.user) return null;
  var turn = 0;
  for (final message in session.messages) {
    if (message.role != MessageRole.user) continue;
    if (message.at.microsecondsSinceEpoch == entry.at.microsecondsSinceEpoch &&
        message.text == entry.text)
      return turn;
    turn++;
  }
  return null;
}

/// Draws the person's messages of [session] with the means to go back to before one and to edit and resend it. [actions] is null
/// where the conversation cannot go back: the messages are drawn as the package draws them.
chat.ChatRenderers turnRenderers(ChatSession session, TurnActions? actions) {
  if (actions == null) return chat.ChatRenderers.none;
  return chat.ChatRenderers(
    entry: [
      (context, entry) {
        final turn = turnOf(session, entry);
        if (turn == null) return null;
        return UserTurn(
          // A message and its place in the conversation, so that editing one does not carry over to another.
          key: ValueKey('turn-$turn'),
          entry: entry,
          onRewind: () => actions.rewind(session, turn, entry.text),
          onResend: (edited) =>
              actions.resend(session, turn, entry.text, edited),
        );
      },
    ],
  );
}

/// One message of the person: as the transcript draws it, with its buttons, and a box to rewrite it in.
class UserTurn extends StatefulWidget {
  const UserTurn({
    super.key,
    required this.entry,
    required this.onRewind,
    required this.onResend,
  });

  final chat.ChatEntryData entry;
  final Future<bool> Function() onRewind;
  final Future<bool> Function(String edited) onResend;

  @override
  State<UserTurn> createState() => _UserTurnState();
}

class _UserTurnState extends State<UserTurn> {
  TextEditingController? _edit;
  bool _busy = false;

  @override
  void dispose() {
    _edit?.dispose();
    super.dispose();
  }

  /// Runs [action] with the buttons held, and says whether it went through.
  Future<bool> _run(Future<bool> Function() action) async {
    if (_busy) return false;
    setState(() => _busy = true);
    try {
      return await action();
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  void _startEdit() =>
      setState(() => _edit = TextEditingController(text: widget.entry.text));

  void _stopEdit() => setState(() {
    _edit?.dispose();
    _edit = null;
  });

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final edit = _edit;
    if (edit != null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.end,
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            key: const Key('turn-edit'),
            controller: edit,
            autofocus: true,
            minLines: 1,
            maxLines: 12,
            style: TextStyle(fontSize: look.bodySize, height: 1.45),
            decoration: const InputDecoration(
              isDense: true,
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 6),
          Wrap(
            spacing: 8,
            children: [
              TextButton(
                key: const Key('turn-edit-cancel'),
                onPressed: _busy ? null : _stopEdit,
                child: Text(context.t('common.cancel')),
              ),
              FilledButton(
                key: const Key('turn-edit-send'),
                onPressed: _busy
                    ? null
                    : () {
                        final edited = edit.text.trim();
                        if (edited.isEmpty) return;
                        unawaited(
                          _run(() => widget.onResend(edited)).then((sent) {
                            if (sent && mounted) _stopEdit();
                          }),
                        );
                      },
                child: Text(context.t('turn.resend')),
              ),
            ],
          ),
        ],
      );
    }
    final text = widget.entry.text;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      mainAxisSize: MainAxisSize.min,
      children: [
        chat.MessageBox(
          color: look.user,
          inBubble: look.userPrefix == null,
          child: Text(
            '${look.userPrefix ?? ''}$text',
            style: TextStyle(
              fontSize: look.bodySize,
              height: 1.45,
              color: look.userPrefix == null ? null : look.accent,
            ),
          ),
        ),
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            chat.CopyButton(text: text, tooltip: context.t('turn.copy')),
            IconButton(
              key: const Key('turn-rewind'),
              tooltip: context.t('turn.rewind'),
              visualDensity: VisualDensity.compact,
              iconSize: 16,
              icon: context.symbol('turn.rewind').build(size: 16),
              onPressed: _busy ? null : () => unawaited(_run(widget.onRewind)),
            ),
            IconButton(
              key: const Key('turn-edit-start'),
              tooltip: context.t('turn.edit'),
              visualDensity: VisualDensity.compact,
              iconSize: 16,
              icon: context.symbol('turn.edit').build(size: 16),
              onPressed: _busy ? null : _startEdit,
            ),
          ],
        ),
      ],
    );
  }
}
