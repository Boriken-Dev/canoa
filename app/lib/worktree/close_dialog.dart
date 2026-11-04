import 'package:flutter/material.dart';

import 'worktree.dart';

/// What becomes of a session's worktree when the session closes.
enum WorktreeChoice {
  keep,

  /// Removed only while it holds no change: git refuses otherwise.
  remove,

  /// Removed with the changes the person was told of.
  discard,
}

/// Asked when a session with a worktree closes: keep it or remove it. Changes not committed are named, and removing them is a
/// button of its own that says so; a branch with commits of its own is kept either way. [state] is null when it could not be
/// read, [problem] saying why, and then only keeping it is offered. Asked once the session's agent has ended, so nothing changes
/// there meanwhile; dismissed, it keeps the worktree.
class WorktreeCloseDialog extends StatelessWidget {
  const WorktreeCloseDialog({super.key, required this.worktree, required this.state, this.problem});

  final SessionWorktree worktree;
  final WorktreeState? state;
  final String? problem;

  static String _files(int count) => count == 1 ? '1 file' : '$count files';
  static String _commits(int count) => count == 1 ? '1 commit' : '$count commits';

  @override
  Widget build(BuildContext context) {
    final state = this.state;
    final error = Theme.of(context).colorScheme.error;
    return AlertDialog(
      title: const Text('Keep this session\'s worktree?'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SelectableText('It worked in ${worktree.path}, on branch ${worktree.branch}.'),
            const SizedBox(height: 8),
            if (state == null)
              Text('Its state could not be read${problem == null ? '' : ': $problem'}. It is kept.',
                  style: TextStyle(color: error))
            else ...[
              if (state.changed > 0)
                Text(
                  '${_files(state.changed)} there ${state.changed == 1 ? 'has' : 'have'} changes not committed. Removing the '
                  'worktree deletes them.',
                  key: const ValueKey('worktree changes'),
                  style: TextStyle(color: error, fontWeight: FontWeight.w600),
                )
              else
                const Text('Nothing there is left uncommitted.'),
              const SizedBox(height: 4),
              Text(state.commits > 0
                  ? 'Branch ${worktree.branch} holds ${_commits(state.commits)} no other branch has, so it stays either way.'
                  : 'Branch ${worktree.branch} holds nothing of its own, so it goes with the worktree.'),
            ],
          ],
        ),
      ),
      actions: [
        if (state != null && state.changed > 0)
          TextButton(
            onPressed: () => Navigator.pop(context, WorktreeChoice.discard),
            style: TextButton.styleFrom(foregroundColor: error),
            child: Text('Delete ${_files(state.changed)} and remove'),
          )
        else if (state != null)
          TextButton(onPressed: () => Navigator.pop(context, WorktreeChoice.remove), child: const Text('Remove worktree')),
        FilledButton(onPressed: () => Navigator.pop(context, WorktreeChoice.keep), child: const Text('Keep worktree')),
      ],
    );
  }
}
