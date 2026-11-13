import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';

/// One file of a session's folder that differs from a commit, as the repository on disk says: nothing an agent claimed.
final class GitFile {
  const GitFile({
    required this.path,
    required this.status,
    required this.added,
    required this.removed,
    required this.binary,
    required this.patch,
    required this.cut,
    required this.print,
  });

  final String path;

  /// `A` added (or untracked), `M` modified, `D` deleted, as git writes them.
  final String status;
  final int added, removed;
  final bool binary;

  /// The unified hunks, with no file header; null when the file has none to show (binary, or too large).
  final String? patch;

  /// Whether [patch] stops short of the whole change.
  final bool cut;

  /// Stands for this file's change: two reads with one print show the file changed the same way.
  final String print;

  /// A whole record from the host; null for anything less, so a host's odd answer drops one file and not the list.
  static GitFile? fromJson(Object? json) {
    if (json is! Map) return null;
    final path = json['path'], status = json['status'], print = json['print'];
    final added = json['added'], removed = json['removed'];
    if (path is! String || path.isEmpty || status is! String || print is! String || added is! int || removed is! int) return null;
    final patch = json['patch'];
    return GitFile(
      path: path,
      status: status,
      added: added,
      removed: removed,
      binary: json['binary'] == true,
      patch: patch is String ? patch : null,
      cut: json['cut'] == true,
      print: print,
    );
  }

  /// The file as the changes view draws it, its lines taken from the hunks.
  FileChange get change {
    final file = FileChange(path)..cut = cut;
    if (patch != null) file.patches.add(patch!);
    return file;
  }
}

/// What a session's folder differs in from [base], read from git at one time.
final class GitState {
  const GitState({required this.head, required this.branch, required this.base, required this.added, required this.removed, required this.files, required this.more});

  /// The commit checked out; null before the repository's first commit.
  final String? head;

  /// The branch checked out; null when HEAD is detached.
  final String? branch;

  /// The commit the files are compared with.
  final String base;

  /// Lines added and removed over every changed file, the ones [files] leaves out too.
  final int added, removed;
  final List<GitFile> files;

  /// Whether more files changed than [files] holds.
  final bool more;

  /// Null when [json] is not a repository's answer.
  static GitState? fromJson(Map<String, Object?> json) {
    if (json['exists'] != true) return null;
    final base = json['base'], added = json['added'], removed = json['removed'], files = json['files'];
    if (base is! String || added is! int || removed is! int || files is! List) return null;
    return GitState(
      head: json['head'] is String ? json['head']! as String : null,
      branch: json['branch'] is String ? json['branch']! as String : null,
      base: base,
      added: added,
      removed: removed,
      files: [for (final file in files) ?GitFile.fromJson(file)],
      more: json['more'] == true,
    );
  }
}

/// Reads what the repository holds in a session's folder: through the host the agent runs on, never by a widget running git.
abstract interface class GitReader {
  /// The folder's files against [base] (HEAD when null); null when [folder] is not in a repository the host reads.
  Future<GitState?> read(String folder, {String? base});
}

/// Which changes the pane shows.
enum ChangeScope {
  /// What the agent's tools reported, never the disk.
  reported('Changes', 'As the agent sent them, not as the files are on disk now.'),

  /// The disk against the last commit.
  uncommitted('Uncommitted', 'The files on disk against the last commit.'),

  /// The files the last turn changed, as the disk shows them against where the session began.
  lastTurn('Last turn', 'The files the last turn changed, as the disk shows them.'),

  /// The disk against where the session began.
  wholeSession('Whole session', 'The files on disk against where this session began.');

  const ChangeScope(this.label, this.note);
  final String label, note;
}

/// What a session knows of its repository: the commit it began on, the disk as last read, and the disk as it was when the last
/// turn began.
final class SessionGit {
  /// The commit the session began on, the first time the repository was read.
  String? base;

  /// The disk against [base].
  GitState? whole;

  /// The disk against HEAD; [whole] again while nothing was committed.
  GitState? uncommitted;

  /// Each file's print when the turn in progress, or the last one, began.
  Map<String, String>? turnStart;

  /// Why the last read gave nothing, in words; null when it did or none was tried.
  String? problem;

  int _reads = 0;

  /// Whether a read has been tried, whatever it found.
  bool get tried => _reads > 0;

  /// Whether any read has said the folder is a repository.
  bool get known => whole != null;

  /// Reads [folder] with [reader]; a read that another overtook is dropped. Returns whether anything changed.
  Future<bool> refresh(GitReader reader, String folder) async {
    final turn = ++_reads;
    try {
      final first = await reader.read(folder, base: base);
      if (turn != _reads) return false;
      if (first == null) {
        problem = null;
        whole = uncommitted = null;
        return true;
      }
      base ??= first.base;
      final head = first.head;
      final now = base == first.base ? first : await reader.read(folder, base: base);
      final plain = head == null || head == base ? now : await reader.read(folder);
      if (turn != _reads) return false;
      whole = now;
      uncommitted = plain;
      problem = null;
    } on Object catch (error) {
      if (turn != _reads) return false;
      problem = error.toString();
    }
    return true;
  }

  /// A turn begins: what the disk holds now is what the turn is measured from.
  Future<void> beginTurn(GitReader reader, String folder) async {
    await refresh(reader, folder);
    turnStart = {for (final file in whole?.files ?? const <GitFile>[]) file.path: file.print};
  }

  /// The files the last turn changed or added, from what the disk held when it began. Empty before any turn.
  List<GitFile> get lastTurn {
    final start = turnStart;
    if (start == null) return const [];
    return [for (final file in whole?.files ?? const <GitFile>[]) if (start[file.path] != file.print) file];
  }

  /// The files [scope] shows, git's own; empty for [ChangeScope.reported].
  List<GitFile> filesOf(ChangeScope scope) => switch (scope) {
    ChangeScope.reported => const [],
    ChangeScope.uncommitted => uncommitted?.files ?? const [],
    ChangeScope.lastTurn => lastTurn,
    ChangeScope.wholeSession => whole?.files ?? const [],
  };
}

/// The lines [changes] add and remove together.
(int, int) totalOf(Iterable<FileChange> changes) {
  var added = 0, removed = 0;
  for (final file in changes) {
    added += file.added;
    removed += file.removed;
  }
  return (added, removed);
}
