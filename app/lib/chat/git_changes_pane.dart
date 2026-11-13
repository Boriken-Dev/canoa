import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../selectable.dart';
import '../theme/look.dart';
import 'git_changes.dart';

/// The files git says differ, by file, each with its additions and removals and its lines; choosing a line writes a comment on
/// it, with the file and line, into the message being written. What it shows is the disk, so it says so.
class GitChangesPane extends StatelessWidget {
  const GitChangesPane({super.key, required this.scope, required this.files, required this.more, required this.onComment});

  final ChangeScope scope;
  final List<GitFile> files;

  /// Whether more files differ than the host listed.
  final bool more;
  final ValueChanged<String> onComment;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final muted = TextStyle(color: look.muted, fontSize: 12);
    return Material(
      color: context.look.rail,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
          child: Text('${scope.note} Choose a line to comment on it.', key: const ValueKey('git note'), style: muted),
        ),
        const Divider(height: 1),
        Expanded(
          child: files.isEmpty
              ? const Center(child: Text('No file differs here.'))
              : ListView(children: [
                  for (final file in files) _GitFileView(key: ValueKey('git ${file.path} ${file.print}'), file: file, onComment: onComment),
                  if (more) Padding(padding: const EdgeInsets.all(16), child: Text('More files differ than are listed here.', style: muted)),
                ]),
        ),
      ]),
    );
  }
}

class _GitFileView extends StatefulWidget {
  const _GitFileView({super.key, required this.file, required this.onComment});

  final GitFile file;
  final ValueChanged<String> onComment;

  @override
  State<_GitFileView> createState() => _GitFileViewState();
}

class _GitFileViewState extends State<_GitFileView> {
  bool _open = true;

  @override
  Widget build(BuildContext context) {
    final file = widget.file;
    final look = context.chatLook;
    final mono = TextStyle(fontFamily: look.codeFont, fontSize: 12, height: 1.35);
    final lines = file.change.lines;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      InkWell(
        onTap: () => setState(() => _open = !_open),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
          child: Row(children: [
            Text(file.status, style: mono.copyWith(color: look.muted)),
            const SizedBox(width: 8),
            Expanded(child: Text(file.path, style: mono, overflow: TextOverflow.ellipsis)),
            Text('+${file.added}', style: mono.copyWith(color: look.success)),
            const SizedBox(width: 6),
            Text('−${file.removed}', style: mono.copyWith(color: look.danger)),
            Icon(_open ? Icons.expand_less : Icons.expand_more, size: 18),
          ]),
        ),
      ),
      if (_open) ...[
        if (file.binary || file.patch == null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: Text(file.binary ? 'A binary file: no lines to show.' : 'Too large to show here.', style: TextStyle(color: look.muted, fontSize: 12)),
          )
        else ...[
          if (file.cut)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 4),
              child: Text('Cut short here, so this is not the whole change.', style: TextStyle(color: look.muted, fontSize: 12, fontWeight: FontWeight.w600)),
            ),
          Container(
            margin: const EdgeInsets.fromLTRB(12, 0, 12, 8),
            constraints: const BoxConstraints(maxHeight: 480),
            decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
            child: SingleChildScrollView(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  for (final line in lines)
                    InkWell(
                      key: ValueKey('git line ${file.path} ${line.now != null ? 'new ${line.now}' : 'old ${line.old}'}'),
                      onTap: () => _comment(context, file, line),
                      child: Container(
                        color: switch (line.kind) {
                          DiffKind.added => look.added,
                          DiffKind.removed => look.removed,
                          DiffKind.same => null,
                        },
                        padding: const EdgeInsets.only(right: 8),
                        child: Text(
                          '${'${line.now ?? ''}'.padLeft(4)} ${switch (line.kind) { DiffKind.added => '+', DiffKind.removed => '-', DiffKind.same => ' ' }} ${line.text}',
                          style: mono,
                          softWrap: false,
                        ),
                      ),
                    ),
                ]),
              ),
            ),
          ),
        ],
      ],
    ]);
  }

  Future<void> _comment(BuildContext context, GitFile file, ChangeLine line) async {
    final at = line.now != null ? '${file.path}:${line.now}' : '${file.path}, removed line ${line.old}';
    final text = await showSelectableDialog<String>(context: context, builder: (context) => _CommentDialog(at: at));
    if (text == null || text.trim().isEmpty) return;
    widget.onComment('$at: `${line.text}`\n${text.trim()}\n');
  }
}

class _CommentDialog extends StatefulWidget {
  const _CommentDialog({required this.at});

  final String at;

  @override
  State<_CommentDialog> createState() => _CommentDialogState();
}

class _CommentDialogState extends State<_CommentDialog> {
  final _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text('Comment on ${widget.at}'),
    content: SizedBox(
      width: 420,
      child: TextField(
        key: const Key('change-comment'),
        controller: _text,
        autofocus: true,
        minLines: 2,
        maxLines: 6,
        decoration: const InputDecoration(hintText: 'What should change here?'),
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, _text.text), child: const Text('Add to message')),
    ],
  );
}
