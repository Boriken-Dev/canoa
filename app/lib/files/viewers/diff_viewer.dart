import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../../theme/look.dart';
import 'file_sample.dart';
import 'file_viewer.dart';
import 'text_viewer.dart';

/// What one line of a unified diff is.
enum DiffRow { meta, hunk, added, removed, context }

const List<String> _fileHeaders = ['--- ', '+++ ', 'index ', 'new file', 'deleted file', 'similarity ', 'rename '];

/// The lines of a unified diff [text] as shown, each with what it is. A line that starts a file (`diff `, `index `, `--- `, `+++ `)
/// is its header; inside a hunk a `+` or `-` line is a change and anything else is context.
List<(DiffRow, String)> classifyDiff(String text) {
  final rows = <(DiffRow, String)>[];
  var inHunk = false;
  for (final line in text.split('\n')) {
    final DiffRow row;
    if (line.startsWith('@@')) {
      inHunk = true;
      row = DiffRow.hunk;
    } else if (line.startsWith('diff ') || line.startsWith('Index: ')) {
      inHunk = false;
      row = DiffRow.meta;
    } else if (!inHunk && _fileHeaders.any(line.startsWith)) {
      row = DiffRow.meta;
    } else if (inHunk && line.startsWith('+')) {
      row = DiffRow.added;
    } else if (inHunk && line.startsWith('-')) {
      row = DiffRow.removed;
    } else {
      row = DiffRow.context;
    }
    // A long line is cut into pieces, each keeping what the line is.
    for (final piece in previewLines(line)) {
      rows.add((row, piece));
    }
  }
  return rows;
}

/// A unified diff (`git diff`, `diff -u`, a `.patch` file): added and removed lines on the look's colours, the hunk headers and
/// file headers set apart.
class DiffViewer extends FileViewer {
  const DiffViewer();

  static const String viewerId = 'diff';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.diff';
  @override
  Set<String> get mimeTypes => const {'text/x-diff', 'text/x-patch'};
  @override
  int get priority => 10;

  @override
  bool canShow(FileSample sample) => sample.isText;

  @override
  Widget build(BuildContext context, FileViewRequest request) {
    final rows = request.sample.memo('diff rows', () => classifyDiff(request.sample.text ?? ''));
    final look = context.chatLook;
    return ListView.builder(
      key: const ValueKey('diff lines'),
      padding: const EdgeInsets.symmetric(vertical: 8),
      itemCount: rows.length,
      itemBuilder: (context, at) {
        final (row, line) = rows[at];
        final color = switch (row) {
          DiffRow.added => look.added,
          DiffRow.removed => look.removed,
          DiffRow.hunk => look.code,
          _ => null,
        };
        final style = TextStyle(
          fontFamily: CanoaLook.monospaceFont,
          fontSize: 12,
          color: switch (row) {
            DiffRow.hunk => look.accent,
            DiffRow.meta => look.muted,
            _ => null,
          },
          fontWeight: row == DiffRow.meta ? FontWeight.w600 : null,
        );
        return Container(
          key: ValueKey('diff ${row.name} $at'),
          color: color,
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: SelectableText(visibleText(line), style: style),
        );
      },
    );
  }
}
