import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../workspace/open_in.dart';
import '../workspace/open_in_ui.dart';
import 'outputs.dart';

/// The files a session made or changed, newest first: a row opens the file in Files, and a menu offers the editor and the file
/// manager where this computer can start them. A deleted file is shown as deleted and opens nothing.
class OutputsList extends StatefulWidget {
  const OutputsList({super.key, required this.outputs, required this.root, this.onOpenInFiles, this.opener});

  final List<SessionOutput> outputs;

  /// The workspace folder, which the rows' paths are shown relative to.
  final String? root;

  /// Opens a path in the Files surface; null where that surface has nothing to show for this session.
  final ValueChanged<String>? onOpenInFiles;

  /// What can be done with a path on this computer; null when the session's files are on another host.
  final WorkspaceOpener? opener;

  /// How many rows show before "Show more".
  static const int shown = 5;

  @override
  State<OutputsList> createState() => _OutputsListState();
}

class _OutputsListState extends State<OutputsList> {
  bool _all = false;

  List<WorkspaceAction> _actions(SessionOutput output) {
    final opener = widget.opener;
    if (opener == null || output.deleted) return const [];
    return opener.actionsFor(WorkspaceContext.files, WorkspaceTarget(output.path, item: true));
  }

  Future<void> _choose(BuildContext context, SessionOutput output, String id) async {
    if (id == 'files') {
      widget.onOpenInFiles?.call(output.path);
      return;
    }
    final target = WorkspaceTarget(output.path, item: true);
    final action = _actions(output).where((each) => each.id == id).firstOrNull;
    if (action != null) await runWorkspaceAction(context, action, target);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final outputs = widget.outputs;
    final visible = _all ? outputs : outputs.take(OutputsList.shown).toList();
    return Column(key: const ValueKey('outputs'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final output in visible) _row(context, look, output),
      if (outputs.length > OutputsList.shown)
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(
            key: const ValueKey('outputs more'),
            style: TextButton.styleFrom(visualDensity: VisualDensity.compact, padding: EdgeInsets.zero),
            onPressed: () => setState(() => _all = !_all),
            child: Text(_all ? 'Show fewer' : 'Show ${outputs.length - OutputsList.shown} more'),
          ),
        ),
    ]);
  }

  Widget _row(BuildContext context, ChatLook look, SessionOutput output) {
    final actions = _actions(output);
    final canFiles = widget.onOpenInFiles != null && !output.deleted;
    final shown = output.shownIn(widget.root);
    final muted = look.muted;
    return InkWell(
      key: ValueKey('output ${output.path}'),
      borderRadius: BorderRadius.circular(look.radius / 2),
      onTap: canFiles ? () => widget.onOpenInFiles!(output.path) : null,
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: Row(children: [
          Icon(output.deleted ? Icons.delete_outline : Icons.insert_drive_file_outlined, size: 16, color: muted),
          const SizedBox(width: 8),
          Expanded(
            child: Tooltip(
              message: output.path,
              child: Text(
                shown,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: look.codeFont,
                  fontSize: 12,
                  color: output.deleted ? muted : null,
                  decoration: output.deleted ? TextDecoration.lineThrough : null,
                ),
              ),
            ),
          ),
          if (output.deleted) Padding(padding: const EdgeInsets.only(left: 8), child: Text('deleted', key: ValueKey('output deleted ${output.path}'), style: TextStyle(color: muted, fontSize: 12))),
          if (canFiles || actions.isNotEmpty)
            PopupMenuButton<String>(
              key: ValueKey('output menu ${output.path}'),
              tooltip: 'Open ${output.name}',
              iconSize: 18,
              padding: EdgeInsets.zero,
              style: IconButton.styleFrom(minimumSize: const Size(28, 28), padding: EdgeInsets.zero, tapTargetSize: MaterialTapTargetSize.shrinkWrap),
              icon: const Icon(Icons.more_horiz),
              onSelected: (id) => unawaited(_choose(context, output, id)),
              itemBuilder: (_) => [
                if (canFiles) const PopupMenuItem(key: ValueKey('output action files'), value: 'files', child: Text('Open in Files')),
                for (final action in actions) PopupMenuItem(key: ValueKey('output action ${action.id}'), value: action.id, child: Text(action.label)),
              ],
            ),
        ]),
      ),
    );
  }
}
