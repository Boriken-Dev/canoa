import 'dart:io';

import 'worktree.dart';

/// Worktrees by git on this computer, for agents this app runs. Git runs from an argument list, never a shell, as the person
/// would run it in their own repository; no `GIT_` variable of the app's reaches it.
Worktrees? localWorktrees() => LocalWorktrees();

final class LocalWorktrees implements Worktrees {
  LocalWorktrees({this.git = 'git'});

  /// The git program, found as the operating system finds a command.
  final String git;

  static final Map<String, String> _environment = {
    for (final MapEntry(:key, :value) in Platform.environment.entries)
      if (!key.toUpperCase().startsWith('GIT_')) key: value,
    'GIT_TERMINAL_PROMPT': '0',
  };

  Future<String> _git(String dir, List<String> args) async {
    final ProcessResult result;
    try {
      result = await Process.run(git, ['-C', dir, ...args], environment: _environment, includeParentEnvironment: false);
    } on ProcessException {
      throw const WorktreeFailure('git could not be started: is it installed?');
    }
    if (result.exitCode != 0) {
      final said = '${result.stderr}'.replaceAll(RegExp(r'[\x00-\x1f\x7f]+'), ' ').trim();
      throw WorktreeFailure('git failed: ${said.length > 1024 ? said.substring(0, 1024) : said}');
    }
    return '${result.stdout}';
  }

  /// [path] with `/` and no trailing one, for comparing paths as git writes them; without case on Windows.
  static String _same(String path) {
    final slashed = path.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '');
    return Platform.isWindows ? slashed.toLowerCase() : slashed;
  }

  static String _native(String path) => Platform.isWindows ? path.replaceAll('/', r'\') : path;

  /// The repository's main worktree, as git writes it.
  Future<String> _repo(String dir) async {
    final fields = (await _git(dir, ['worktree', 'list', '--porcelain', '-z'])).split('\x00');
    final first = fields.first;
    if (!first.startsWith('worktree ')) throw const WorktreeFailure('That directory is not in a git repository.');
    if (fields.takeWhile((field) => field.isNotEmpty).contains('bare')) {
      throw const WorktreeFailure('A bare repository has no worktree to start from.');
    }
    return first.substring('worktree '.length);
  }

  @override
  Future<SessionWorktree> create(String cwd, String branch) async {
    if (!safeWorktreeBranch(branch)) throw WorktreeFailure('$branch is not a branch Canoa makes.');
    if (!await Directory(cwd).exists()) throw WorktreeFailure('$cwd is not a directory.');
    final repo = await _repo(cwd);
    if (!repo.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '').contains('/') ||
        RegExp(r'^[A-Za-z]:/?$').hasMatch(repo)) {
      throw const WorktreeFailure('A repository at the top of a drive has no directory beside it.');
    }
    final location = worktreeLocation(repo, branch);
    final holder = location.substring(0, location.lastIndexOf('/'));
    final holderType = FileSystemEntity.typeSync(holder, followLinks: false);
    if (holderType != FileSystemEntityType.notFound && holderType != FileSystemEntityType.directory) {
      throw WorktreeFailure('$holder is a link or not a directory, so no worktree goes in it.');
    }
    if (FileSystemEntity.typeSync(location, followLinks: false) != FileSystemEntityType.notFound) {
      throw WorktreeFailure('$location is already there.');
    }
    await _git(cwd, ['worktree', 'add', '-b', branch, '--', location, 'HEAD']);
    return SessionWorktree(path: _native(location), branch: branch, repo: _native(repo));
  }

  /// The branch of the worktree at [path], when it is one Canoa made where it puts that branch; null otherwise.
  Future<({String repo, String branch})?> _ours(String path) async {
    try {
      final top = (await _git(path, ['rev-parse', '--show-toplevel'])).trim();
      if (_same(top) != _same(path)) return null;
      final repo = await _repo(path);
      final branch = (await _git(path, ['symbolic-ref', '--quiet', '--short', 'HEAD'])).trim();
      if (_same(repo) == _same(path) || !safeWorktreeBranch(branch)) return null;
      if (_same(worktreeLocation(repo, branch)) != _same(path)) return null;
      return (repo: repo, branch: branch);
    } on WorktreeFailure {
      return null;
    }
  }

  Future<int> _changed(String path) async {
    final fields = (await _git(path, ['status', '--porcelain=v1', '-z', '--untracked-files=all']))
        .split('\x00')
        .where((field) => field.isNotEmpty)
        .iterator;
    var count = 0;
    while (fields.moveNext()) {
      count++;
      // A rename or copy is followed by the name it had.
      if (fields.current.startsWith('R') || fields.current.startsWith('C')) fields.moveNext();
    }
    return count;
  }

  Future<int> _commits(String repo, String branch) async {
    final counted = await _git(repo, [
      'rev-list', '--count', 'refs/heads/$branch', '--not', '--exclude=$branch', '--branches', '--remotes', '--tags', '--',
    ]);
    return int.tryParse(counted.trim()) ?? (throw const WorktreeFailure('git did not count the branch\'s commits'));
  }

  @override
  Future<WorktreeState> status(SessionWorktree worktree) async {
    if (FileSystemEntity.typeSync(worktree.path, followLinks: false) != FileSystemEntityType.directory) {
      return const WorktreeState.gone();
    }
    final ours = await _ours(worktree.path);
    if (ours == null) return const WorktreeState.gone();
    return WorktreeState(
      branch: ours.branch,
      changed: await _changed(worktree.path),
      commits: await _commits(ours.repo, ours.branch),
    );
  }

  @override
  Future<WorktreeRemoval> remove(SessionWorktree worktree, {bool discard = false}) async {
    if (FileSystemEntity.typeSync(worktree.path, followLinks: false) != FileSystemEntityType.directory) {
      throw WorktreeFailure('${worktree.path} is gone.');
    }
    final ours = await _ours(worktree.path);
    if (ours == null) throw WorktreeFailure('${worktree.path} is not a worktree Canoa made.');
    final commits = await _commits(ours.repo, ours.branch);
    await _git(ours.repo, ['worktree', 'remove', if (discard) '--force', '--', worktree.path]);
    // A branch holding commits nothing else has stays: removing it would lose them.
    if (commits == 0) await _git(ours.repo, ['branch', '-D', '--', ours.branch]);
    return WorktreeRemoval(branch: ours.branch, branchKept: commits > 0, commits: commits);
  }
}
