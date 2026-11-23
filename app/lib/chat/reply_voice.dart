import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatMessage, MessageRole;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../highlight/highlighted.dart';
import '../voice/reply_reader.dart';
import '../voice/reply_speaker.dart';
import 'dictation.dart';

/// The key of a reply, which does not change while it streams: when its first words arrived.
String replyKey(DateTime at) => '${at.microsecondsSinceEpoch}';

/// The registry entry that draws an agent's reply with Play beside Copy. Anything else is left to the chat's own drawing.
ChatRenderers replyRenderers(ReplySpeaker reader) => ChatRenderers(
  entry: [
    (context, entry) => entry.role == MessageRole.agent && reader.available ? ReplyMessage(entry: entry, reader: reader) : null,
  ],
);

/// An agent's reply as the chat draws it, with a button that reads it aloud beside the one that copies it.
class ReplyMessage extends StatelessWidget {
  const ReplyMessage({super.key, required this.entry, required this.reader});

  final ChatEntryData entry;
  final ReplySpeaker reader;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Column(crossAxisAlignment: CrossAxisAlignment.end, mainAxisSize: MainAxisSize.min, children: [
      MessageBox(color: look.agent, inBubble: look.agentInBubble, child: AgentMarkdown(text: entry.text, builders: highlightedMarkdown)),
      Row(mainAxisSize: MainAxisSize.min, children: [
        PlayReplyButton(reader: reader, replyKey: replyKey(entry.at), text: entry.text),
        CopyButton(text: entry.text, tooltip: 'Copy message'),
      ]),
    ]);
  }
}

/// Reads the reply aloud, and while it is being read stops it. A failure is said in a message under the page.
class PlayReplyButton extends StatefulWidget {
  const PlayReplyButton({super.key, required this.reader, required this.replyKey, required this.text});

  final ReplySpeaker reader;
  final String replyKey;
  final String text;

  @override
  State<PlayReplyButton> createState() => _PlayReplyButtonState();
}

class _PlayReplyButtonState extends State<PlayReplyButton> {
  @override
  void initState() {
    super.initState();
    widget.reader.addListener(_failed);
  }

  @override
  void didUpdateWidget(PlayReplyButton old) {
    super.didUpdateWidget(old);
    if (!identical(old.reader, widget.reader)) {
      old.reader.removeListener(_failed);
      widget.reader.addListener(_failed);
    }
  }

  @override
  void dispose() {
    widget.reader.removeListener(_failed);
    super.dispose();
  }

  void _failed() {
    final reader = widget.reader;
    final why = reader.error;
    if (why == null || reader.errorKey != widget.replyKey || !mounted) return;
    reader.clearError();
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(why), duration: const Duration(seconds: 6)));
  }

  @override
  Widget build(BuildContext context) => SelectionContainer.disabled(
    child: ListenableBuilder(
      listenable: widget.reader,
      builder: (context, _) {
        final reading = widget.reader.isReading(widget.replyKey);
        return IconButton(
          key: Key(reading ? 'reply-stop' : 'reply-play'),
          tooltip: reading ? 'Stop reading' : 'Read aloud',
          iconSize: 15,
          visualDensity: VisualDensity.compact,
          color: context.chatLook.muted,
          onPressed: () => unawaited(widget.reader.toggle(widget.replyKey, widget.text)),
          icon: Icon(reading ? Icons.stop_circle_outlined : Icons.volume_up_outlined),
        );
      },
    ),
  );
}

/// Tells the reader what a conversation's replies are, for voice mode: each by its key, whole once its turn is over or something
/// came after it.
void followReplies(ReplyReader reader, String session, List<ChatMessage> messages, {required bool working}) {
  if (!reader.voiceMode) return;
  final replies = <ReplyText>[];
  for (var i = 0; i < messages.length; i++) {
    final message = messages[i];
    if (message.role != MessageRole.agent) continue;
    replies.add((key: replyKey(message.at), text: message.text, complete: !working || i != messages.length - 1));
  }
  reader.observe(session, replies);
}

/// Voice mode's entry in the `+` menu: while it is on, each reply is read aloud as it arrives, a sentence at a time.
class VoiceModeMenuItem extends StatelessWidget {
  const VoiceModeMenuItem({super.key, required this.reader, required this.onChanged});

  final ReplySpeaker reader;

  /// Told when the person turns it on or off, to keep the choice.
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: reader,
    builder: (context, _) {
      final on = reader.voiceMode;
      return CheckboxMenuButton(
        value: on,
        onChanged: (_) => _turn(reader, !on, onChanged),
        child: const Text('Read replies aloud', key: Key('voice-mode')),
      );
    },
  );
}

/// Turns voice mode [on] or off. A browser lets a page make a sound only after a press, so the output is made ready here.
void _turn(ReplySpeaker reader, bool on, ValueChanged<bool> onChanged) {
  reader.unlock();
  reader.setVoiceMode(on);
  onChanged(on);
}

/// Voice mode in the line under the composer: nothing while it is off and "Voice mode" while it is on, which turns it off. Typing,
/// and starting to dictate, stop what is being read, and a failure to read is said in a message.
class VoiceModeStatus extends StatefulWidget {
  const VoiceModeStatus({super.key, required this.reader, required this.controller, this.dictation, required this.onChanged});

  final ReplySpeaker reader;

  /// The message being written; what is typed in it stops the reading.
  final TextEditingController controller;
  final Dictation? dictation;

  /// Told when the person turns it off, to keep the choice.
  final ValueChanged<bool> onChanged;

  @override
  State<VoiceModeStatus> createState() => _VoiceModeStatusState();
}

class _VoiceModeStatusState extends State<VoiceModeStatus> {
  String _text = '';

  @override
  void initState() {
    super.initState();
    _text = widget.controller.text;
    widget.controller.addListener(_typed);
    widget.dictation?.addListener(_dictated);
    widget.reader.addListener(_failed);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_typed);
    widget.dictation?.removeListener(_dictated);
    widget.reader.removeListener(_failed);
    super.dispose();
  }

  void _typed() {
    final now = widget.controller.text;
    final grew = now.isNotEmpty && now != _text;
    _text = now;
    // Sending empties the message, which is not typing.
    if (grew && widget.reader.busy) widget.reader.stop();
  }

  void _dictated() {
    final dictation = widget.dictation;
    if (dictation != null && dictation.state != DictationState.idle && widget.reader.busy) widget.reader.stop();
  }

  void _failed() {
    final reader = widget.reader;
    final why = reader.error;
    if (why == null || reader.errorKey != null || !mounted) return;
    reader.clearError();
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(why), duration: const Duration(seconds: 6)));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.reader,
    builder: (context, _) {
      if (!widget.reader.voiceMode) return const SizedBox.shrink();
      return InkWell(
        key: const Key('voice-mode-on'),
        borderRadius: BorderRadius.circular(8),
        onTap: () => _turn(widget.reader, false, widget.onChanged),
        child: const Tooltip(
          message: 'Voice mode is on: replies are read aloud as they arrive. Typing or dictating stops the reading. Click to turn it off',
          child: ComposerPill(label: 'Voice mode', icon: Icons.record_voice_over, status: true, enabled: false),
        ),
      );
    },
  );
}
