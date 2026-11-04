import 'dart:io';

import 'package:flutter/foundation.dart' show visibleForTesting;

import 'worktree.dart';

/// Worktrees by git on this computer, for agents this app runs. Git runs from an argument list, never a shell, with hooks
/// and fsmonitor off and no `GIT_` variable of the app's; a repository whose own configuration names a filter or an include
/// is refused before git reads its files, as `canoa-host` does.
Worktrees? localWorktrees() => LocalWorktrees();

final class LocalWorktrees implements Worktrees {
  LocalWorktrees({this.git = 'git', this.afterCount});

  /// The git program, found as the operating system finds a command.
  final String git;

  /// Runs in [remove] once the branch's commits are counted, as another process might meanwhile.
  @visibleForTesting
  final Future<void> Function()? afterCount;

  static final Map<String, String> _environment = {
    for (final MapEntry(:key, :value) in Platform.environment.entries)
      if (!key.toUpperCase().startsWith('GIT_')) key: value,
    'GIT_TERMINAL_PROMPT': '0',
  };

  /// Settings that make git run a program (a filter) or read another file of settings (an include), and where attributes
  /// naming a filter come from.
  static const _unsafeSettings = r'^(filter|include|includeif)\.|^core\.attributesfile$';

  Future<ProcessResult> _run(String dir, List<String> args) async {
    try {
      return await Process.run(
        git,
        ['-C', dir, '-c', 'core.hooksPath=${Platform.isWindows ? 'NUL' : '/dev/null'}', '-c', 'core.fsmonitor=false', ...args],
        environment: _environment,
        includeParentEnvironment: false,
      );
    } on ProcessException {
      throw const WorktreeFailure('git could not be started: is it installed?');
    }
  }

  Future<String> _git(String dir, List<String> args) async {
    final result = await _run(dir, args);
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

  /// The repository's main worktree, as git writes it, once its own settings name no filter or include and its git
  /// directory is its work tree's own `.git`, which a submodule's is not.
  Future<String> _repo(String dir) async {
    for (final scope in ['--local', '--worktree']) {
      final found = await _run(dir, ['config', scope, '--name-only', '--get-regexp', _unsafeSettings]);
      if (found.exitCode == 0) {
        throw const WorktreeFailure('The repository\'s own configuration names a filter, an include or an attributes file, '
            'which would run or read what Canoa does not check: no worktree is made, read or removed there.');
      }
    }
    final fields = (await _git(dir, ['worktree', 'list', '--porcelain', '-z'])).split('\x00');
    final first = fields.first;
    if (!first.startsWith('worktree ')) throw const WorktreeFailure('That directory is not in a git repository.');
    if (fields.takeWhile((field) => field.isNotEmpty).contains('bare')) {
      throw const WorktreeFailure('A bare repository has no worktree to start from.');
    }
    final repo = first.substring('worktree '.length);
    final common = (await _git(dir, ['rev-parse', '--path-format=absolute', '--git-common-dir'])).trim();
    if (_same(Directory(common).resolveSymbolicLinksSync()) !=
        _same(Directory('$repo/.git').existsSync() ? Directory('$repo/.git').resolveSymbolicLinksSync() : '$repo/.git')) {
      throw const WorktreeFailure('The repository\'s git directory is apart from its work tree, as a submodule\'s is: '
          'no worktree is made there.');
    }
    return repo;
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

  /// The worktree at [path], when it is one Canoa made: its own top level, a linked worktree, where Canoa puts the branch
  /// its directory is named for. Found by where it is, so one whose HEAD was detached or moved is still found. Null when
  /// nothing like it is there.
  Future<({String repo, String branch, String? tip, String? head})?> _ours(String path) async {
    final String top;
    try {
      top = (await _git(path, ['rev-parse', '--show-toplevel'])).trim();
    } on WorktreeFailure {
      return null;
    }
    if (_same(top) != _same(path)) return null;
    final repo = await _repo(path);
    final branch = '$worktreeBranchPrefix${path.replaceAll(r'\', '/').replaceAll(RegExp(r'/+$'), '').split('/').last}';
    if (_same(repo) == _same(path) || !safeWorktreeBranch(branch)) return null;
    if (_same(worktreeLocation(repo, branch)) != _same(path)) return null;
    final tip = await _run(repo, ['rev-parse', '--verify', '--quiet', 'refs/heads/$branch']);
    final head = await _run(path, ['symbolic-ref', '--quiet', '--short', 'HEAD']);
    return (
      repo: repo,
      branch: branch,
      tip: tip.exitCode == 0 ? '${tip.stdout}'.trim() : null,
      head: head.exitCode == 0 ? '${head.stdout}'.trim() : null,
    );
  }

  /// What `git status` lists there, files git ignores and other repositories inside included: what removing it deletes.
  Future<(int, String)> _changed(String path) async {
    final listed = await _git(
        path, ['status', '--porcelain=v1', '-z', '--untracked-files=all', '--ignored=matching', '--ignore-submodules=all']);
    final fields = listed.split('\x00').where((field) => field.isNotEmpty).iterator;
    var count = 0;
    while (fields.moveNext()) {
      count++;
      // A rename or copy is followed by the name it had.
      if (fields.current.startsWith('R') || fields.current.startsWith('C')) fields.moveNext();
    }
    return (count, listed);
  }

  /// Commits [starts] reach that no branch but [besides], no tag and no remote does.
  Future<int> _commits(String dir, String? besides, List<String> starts) async {
    final counted = await _git(dir, [
      'rev-list', '--count', ...starts, '--not', if (besides != null) '--exclude=$besides', '--branches', '--remotes',
      '--tags', '--',
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
    final (changed, listing) = await _changed(worktree.path);
    return WorktreeState(
      branch: ours.head,
      detached: ours.head == null,
      changed: changed,
      commits: await _commits(worktree.path, ours.branch, ['HEAD', if (ours.tip != null) 'refs/heads/${ours.branch}']),
      listing: listing,
    );
  }

  @override
  Future<WorktreeRemoval> remove(SessionWorktree worktree, {bool discard = false}) async {
    if (FileSystemEntity.typeSync(worktree.path, followLinks: false) != FileSystemEntityType.directory) {
      throw WorktreeFailure('${worktree.path} is gone.');
    }
    final ours = await _ours(worktree.path);
    if (ours == null) throw WorktreeFailure('${worktree.path} is not a worktree Canoa made.');
    // A commit only a detached HEAD reaches is lost with the worktree, whatever was chosen.
    if (ours.head == null && await _commits(worktree.path, null, ['HEAD']) > 0) {
      throw const WorktreeFailure('Its HEAD is detached at commits no branch holds, which removing it would lose: '
          'put them on a branch first.');
    }
    // Git itself removes what it ignores without --force, so that takes discard too.
    final (changed, _) = await _changed(worktree.path);
    if (changed > 0 && !discard) {
      throw WorktreeFailure('$changed files or folders there are in no commit, which removing it deletes.');
    }
    final commits = ours.tip == null ? 0 : await _commits(ours.repo, ours.branch, ['refs/heads/${ours.branch}']);
    await afterCount?.call();
    await _git(ours.repo, ['worktree', 'remove', if (discard) '--force', '--', worktree.path]);
    // A branch holding commits nothing else has stays, and so does one that moved since it was counted: update-ref
    // deletes it only at the commit counted.
    var deleted = false;
    if (ours.tip != null && commits == 0) {
      deleted = (await _run(ours.repo, ['update-ref', '-d', 'refs/heads/${ours.branch}', ours.tip!])).exitCode == 0;
    }
    return WorktreeRemoval(branch: ours.branch, branchKept: ours.tip != null && !deleted, commits: commits);
  }
}
