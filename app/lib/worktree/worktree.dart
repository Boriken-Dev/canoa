import 'dart:math';

/// Every branch Canoa makes for a session starts with this.
const worktreeBranchPrefix = 'canoa/';

/// The longest branch name taken, prefix included; `canoa-host` holds the same bound.
const maxWorktreeBranch = 70;

final _safeBranch = RegExp(r'^canoa/[a-z0-9]+(?:-[a-z0-9]+)*$');

/// `canoa/` and then lower-case letters and digits in runs joined by single hyphens: nothing git or a shell reads as an option,
/// a range or a path step, and no control character. `canoa-host` refuses any other.
bool safeWorktreeBranch(String branch) => branch.length <= maxWorktreeBranch && _safeBranch.hasMatch(branch);

/// A branch for a session named [label] (its title, or its agent's when it has none): the label's letters and digits, lower
/// case, then six random hexadecimal digits so two sessions of one name do not meet.
String worktreeBranch(String label, {Random? random}) {
  final words = label
      .toLowerCase()
      .replaceAll(RegExp('[^a-z0-9]+'), '-')
      .replaceAll(RegExp(r'^-+|-+$'), '');
  var slug = words.length > 40 ? words.substring(0, 40).replaceAll(RegExp(r'-+$'), '') : words;
  if (slug.isEmpty) slug = 'session';
  final draw = random ?? Random.secure();
  final suffix = List.generate(6, (_) => draw.nextInt(16).toRadixString(16)).join();
  return '$worktreeBranchPrefix$slug-$suffix';
}

/// Where a session's worktree is, and on which branch. The agent works in [path]; [repo] is the repository's main worktree.
final class SessionWorktree {
  const SessionWorktree({required this.path, required this.branch, required this.repo});

  final String path, branch, repo;

  Map<String, Object?> toJson() => {'path': path, 'branch': branch, 'repo': repo};

  /// Null for anything but a whole record with a branch Canoa would make, its path where Canoa puts that branch beside
  /// that repository.
  static SessionWorktree? fromJson(Object? json) {
    if (json is! Map) return null;
    final path = json['path'], branch = json['branch'], repo = json['repo'];
    if (path is! String || path.isEmpty || repo is! String || repo.isEmpty || branch is! String || !safeWorktreeBranch(branch)) {
      return null;
    }
    final slashed = repo.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '');
    if (!slashed.contains('/') || path.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '') != worktreeLocation(repo, branch)) {
      return null;
    }
    return SessionWorktree(path: path, branch: branch, repo: repo);
  }
}

/// What a worktree holds now.
final class WorktreeState {
  const WorktreeState.gone() : exists = false, branch = null, changed = 0, commits = 0, detached = false, listing = '';
  const WorktreeState({
    required this.branch,
    required this.changed,
    required this.commits,
    this.detached = false,
    this.listing = '',
  }) : exists = true;

  final bool exists;

  /// The branch checked out there now; null when its HEAD is detached.
  final String? branch;
  final bool detached;

  /// Stands for the whole list of what [changed] counts: two states with the same listing list the same files.
  final String listing;

  /// Files `git status` lists, untracked ones included: what removing it deletes.
  final int changed;

  /// Commits on its branch that no other branch, tag or remote has.
  final int commits;
}

/// What removing a worktree did.
final class WorktreeRemoval {
  const WorktreeRemoval({required this.branch, required this.branchKept, required this.commits});
  final String branch;

  /// The branch stays while it holds commits of its own.
  final bool branchKept;
  final int commits;
}

/// Why a worktree could not be made, read or removed, in words to show.
final class WorktreeFailure implements Exception {
  const WorktreeFailure(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Makes, reads and removes a session's worktree: by git on this computer, or by the host an agent is served from.
abstract interface class Worktrees {
  /// A new worktree of the repository holding [cwd], from its current commit, on the new branch [branch].
  Future<SessionWorktree> create(String cwd, String branch);

  Future<WorktreeState> status(SessionWorktree worktree);

  /// Removes [worktree]. Without [discard] a worktree holding changes or untracked files is refused, whatever a status said
  /// before. Its branch goes too unless it holds commits of its own.
  Future<WorktreeRemoval> remove(SessionWorktree worktree, {bool discard = false});
}

/// Where the worktree of [branch] goes: in `<repository>.worktrees/` beside [repo], named for [branch] without its prefix,
/// written with `/`.
String worktreeLocation(String repo, String branch) {
  final slashed = repo.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '');
  final cut = slashed.lastIndexOf('/');
  final parent = slashed.substring(0, cut), name = slashed.substring(cut + 1);
  return '$parent/$name.worktrees/${branch.substring(worktreeBranchPrefix.length)}';
}
