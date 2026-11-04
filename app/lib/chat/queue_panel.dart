import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/look.dart';

/// The messages written while the agent worked, in the order they go, each editable and removable. While the queue is held,
/// after a turn that was stopped or failed, it says so and waits for the person to send the next or clear them.
class QueuePanel extends StatelessWidget {
  const QueuePanel({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  Future<void> _edit(BuildContext context, QueuedMessage message) async {
    final text = await showDialog<String>(context: context, builder: (_) => _EditQueued(text: message.text));
    if (text != null) store.editQueued(session, message, text);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final small = TextStyle(fontSize: 12, color: look.muted);
    final held = session.queueHeld;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 6, 16, 0),
      child: Center(
        heightFactor: 1,
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
            Row(children: [
              Icon(held == null ? Icons.schedule : Icons.pause_circle_outline, size: 16, color: look.muted),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  held == null
                      ? '${session.queue.length} queued: sent one at a time, as each turn ends'
                      : 'Held, since $held: nothing more is sent until you say',
                  style: small,
                ),
              ),
              if (held != null && session.status != SessionStatus.working)
                TextButton(onPressed: () => store.sendQueued(session), child: const Text('Send next')),
              TextButton(onPressed: () => store.removeQueued(session), child: const Text('Clear queue')),
            ]),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 180),
              child: ListView(shrinkWrap: true, children: [
                for (final message in session.queue)
                  Container(
                    key: ObjectKey(message),
                    margin: const EdgeInsets.only(bottom: 4),
                    padding: const EdgeInsets.only(left: 12),
                    decoration: BoxDecoration(
                      border: Border.all(color: look.muted.withValues(alpha: 0.6)),
                      borderRadius: BorderRadius.circular(look.radius / 2),
                    ),
                    child: Row(children: [
                      Expanded(
                        child: Text(
                          [
                            message.text.trim(),
                            if (message.attachments.isNotEmpty) '(with ${message.attachments.map((item) => item.name).join(', ')})',
                          ].where((part) => part.isNotEmpty).join(' '),
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      IconButton(tooltip: 'Edit queued message', icon: const Icon(Icons.edit_outlined, size: 18),
                          onPressed: () => _edit(context, message)),
                      IconButton(tooltip: 'Remove queued message', icon: const Icon(Icons.close, size: 18),
                          onPressed: () => store.removeQueued(session, message)),
                    ]),
                  ),
              ]),
            ),
          ]),
        ),
      ),
    );
  }
}

class _EditQueued extends StatefulWidget {
  const _EditQueued({required this.text});

  final String text;

  @override
  State<_EditQueued> createState() => _EditQueuedState();
}

class _EditQueuedState extends State<_EditQueued> {
  late final _text = TextEditingController(text: widget.text);

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Queued message'),
    content: SizedBox(
      width: 480,
      child: TextField(key: const Key('queued-text'), controller: _text, autofocus: true, minLines: 2, maxLines: 8),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, _text.text), child: const Text('Save')),
    ],
  );
}
