import 'dart:io';

import 'permission_rules.dart' show RealPath, RepoRoot;

/// [path] with the links resolved in the deepest part of it that exists, and what does not exist yet after it as written; null
/// when a link cannot be followed (a dangling one) or nothing of it can be found.
String? resolveExisting(String path) {
  var current = path;
  final rest = <String>[];
  while (true) {
    try {
      if (FileSystemEntity.typeSync(current, followLinks: false) != FileSystemEntityType.notFound) {
        final resolved = File(current).resolveSymbolicLinksSync();
        return rest.isEmpty ? resolved : '$resolved/${rest.reversed.join('/')}';
      }
    } on FileSystemException {
      return null;
    }
    final parent = File(current).parent.path;
    if (parent == current) return null;
    rest.add(current.substring(parent.length).replaceAll(RegExp(r'^[\\/]+'), ''));
    current = parent;
  }
}

const RealPath platformRealPath = resolveExisting;

/// The nearest folder at or above [folder] that holds a `.git`, as written, or null when none does. A linked worktree or a
/// submodule has a `.git` that is a file naming a folder inside its main repository's `.git`: that main repository is the answer,
/// so the worktrees of one repository count as one.
String? findRepoRoot(String folder) {
  var current = folder;
  while (true) {
    try {
      final kind = FileSystemEntity.typeSync('$current/.git', followLinks: false);
      if (kind == FileSystemEntityType.file) return _mainRepository(current) ?? current;
      if (kind != FileSystemEntityType.notFound) return current;
    } on FileSystemException {
      return null;
    }
    final parent = File(current).parent.path;
    if (parent == current) return null;
    current = parent;
  }
}

const RepoRoot platformRepoRoot = findRepoRoot;

/// The main repository a `.git` file in [folder] points into (`gitdir: <main>/.git/worktrees/<name>`), or null when it names none.
String? _mainRepository(String folder) {
  try {
    final line = File('$folder/.git').readAsStringSync().trim();
    if (!line.startsWith('gitdir:')) return null;
    final slash = String.fromCharCode(92);
    var target = line.substring(7).trim().replaceAll(slash, '/');
    if (!target.startsWith('/') && !RegExp(r'^[A-Za-z]:/').hasMatch(target)) target = '${folder.replaceAll(slash, '/')}/$target';
    final at = target.indexOf('/.git/');
    return at <= 0 ? null : target.substring(0, at);
  } on FileSystemException {
    return null;
  }
}
