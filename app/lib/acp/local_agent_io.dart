import 'dart:async';
import 'dart:convert';
import 'dart:io';


const bool canRunLocalAgents = true;

/// A `.cmd` such as `npx` on Windows can only be started through a shell.
final bool _useShell = Platform.isWindows;

/// [directory] with `~` and `%NAME%` filled in, or null when a variable is not set.
String? _expand(String directory) {
  var out = directory;
  if (out == '~' || out.startsWith('~/')) {
    final home = environmentVariable(Platform.isWindows ? 'USERPROFILE' : 'HOME');
    if (home == null) return null;
    out = '$home${out.substring(1)}';
  }
  for (final match in RegExp(r'%([A-Za-z0-9_]+)%').allMatches(out).toList()) {
    final value = environmentVariable(match[1]!);
    if (value == null) return null;
    out = out.replaceAll(match[0]!, value);
  }
  return out;
}

/// [program] in one of [directories], as an installer leaves it.
String? _findIn(String program, List<String> directories) {
  final names = Platform.isWindows ? ['$program.exe', '$program.cmd', '$program.bat', '$program.com'] : [program];
  for (final directory in directories) {
    final expanded = _expand(directory);
    if (expanded == null) continue;
    for (final name in names) {
      final file = File('$expanded/$name');
      if (file.existsSync()) return file.path;
    }
  }
  return null;
}

Future<String?> _find(String program, [List<String> searchDirs = const []]) async =>
    await _findOnPath(program) ?? _findIn(program, searchDirs);

Future<String?> _findOnPath(String program) async {
  // A program named by its full path is found when the file is there; `where` and `which` look only along PATH.
  if (File(program).isAbsolute && File(program).existsSync()) return program;
  final finder = Platform.isWindows ? 'where' : 'which';
  try {
    final found = await Process.run(finder, [program], runInShell: _useShell);
    if (found.exitCode != 0) return null;
    final lines = LineSplitter.split(found.stdout as String).map((line) => line.trim()).where((line) => line.isNotEmpty);
    // `where npx` names the shell script before `npx.cmd`; Windows starts only the second.
    final runnable = Platform.isWindows
        ? lines.where((line) => RegExp(r'\.(exe|cmd|bat|com)$', caseSensitive: false).hasMatch(line))
        : lines;
    return runnable.firstOrNull ?? lines.firstOrNull;
  } on ProcessException {
    return null;
  }
}

/// [program]'s absolute path, as PATH finds it, else in [searchDirs] where its installer puts it, or null.
Future<String?> findProgram(String program, {List<String> searchDirs = const []}) => _find(program, searchDirs);

/// This process's environment variable [name]; on Windows names compare without case.
String? environmentVariable(String name) {
  final environment = Platform.environment;
  if (environment[name] case final value?) return value;
  if (!Platform.isWindows) return null;
  final upper = name.toUpperCase();
  return environment.entries.where((entry) => entry.key.toUpperCase() == upper).firstOrNull?.value;
}

/// The bytes of the file at [path] on this computer.
Future<List<int>> readLocalFile(String path) => File(path).readAsBytes();

/// Whether [folder] is a folder on this computer now.
bool folderExists(String folder) => Directory(folder).existsSync();

/// Whether [folder] is in a git repository: it or a folder above it holds `.git`.
bool inGitRepository(String folder) {
  var dir = Directory(folder);
  if (!dir.existsSync()) return false;
  while (true) {
    if (FileSystemEntity.typeSync('${dir.path}${Platform.pathSeparator}.git', followLinks: false) != FileSystemEntityType.notFound) {
      return true;
    }
    final parent = dir.parent;
    if (parent.path == dir.path) return false;
    dir = parent;
  }
}

/// The size in bytes of the file at [path], or null for anything that is not a file that can be read.
Future<int?> localFileSize(String path) async {
  final file = File(path);
  return await file.exists() ? file.length() : null;
}

/// A file or folder inside a session's folder, as [resolveInside] answers.
typedef InsideFile = ({String path, bool folder, int size});

/// Characters a name must not hold to be offered: controls, and the ones that hide or reorder text.
final _unsafeName = RegExp('[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]');

/// The files and folders under [folder], as paths relative to it with `/` (a folder ends in `/`), nearest first. In a git
/// repository they are the files git lists by the repository's own rules, tracked or untracked and not ignored; elsewhere
/// the folder is walked. Either way links are neither followed nor listed, names with control or direction characters are
/// left out, and at most [maxEntries] are read and [maxDepth] levels walked.
Future<List<String>> listFolderEntries(String folder, {int maxEntries = 5000, int maxDepth = 8}) async {
  if (!await Directory(folder).exists()) return const [];
  if (inGitRepository(folder)) {
    final listed = await _gitListing(folder, maxEntries);
    if (listed != null) return listed;
  }
  return _walkListing(folder, maxEntries, maxDepth);
}

Future<List<String>> _walkListing(String folder, int maxEntries, int maxDepth) async {
  final found = <String>[];
  final queue = <(Directory, String, int)>[(Directory(folder), '', 0)];
  for (var next = 0; next < queue.length && found.length < maxEntries; next++) {
    final (dir, prefix, depth) = queue[next];
    final here = <(String, bool)>[];
    try {
      await for (final entry in dir.list(followLinks: false)) {
        if (entry is Link) continue;
        final name = entry.path.split(RegExp(r'[\\/]')).last;
        if (name.isEmpty || _unsafeName.hasMatch(name)) continue;
        if (entry is Directory) {
          here.add((name, true));
        } else if (entry is File) {
          here.add((name, false));
        }
      }
    } on FileSystemException {
      continue;
    }
    here.sort((a, b) => a.$1.compareTo(b.$1));
    for (final (name, isFolder) in here) {
      if (found.length >= maxEntries) break;
      found.add(isFolder ? '$prefix$name/' : '$prefix$name');
      if (isFolder && depth < maxDepth) queue.add((Directory('${dir.path}${Platform.pathSeparator}$name'), '$prefix$name/', depth + 1));
    }
  }
  return found;
}

const int _gitOutputLimit = 16 * 1024 * 1024;

/// What `git ls-files` lists in [folder] (tracked, and untracked that no ignore rule covers), as files and their folders, or null
/// when git is not there, fails or takes too long. git is found along PATH and started with no shell, with nothing from the
/// repository's configuration that could run a program (`core.fsmonitor`), nothing asked of the person, and no optional lock.
Future<List<String>?> _gitListing(String folder, int maxEntries) async {
  final git = await findProgram('git');
  if (git == null) return null;
  Process process;
  try {
    process = await Process.start(
      git,
      ['-C', folder, '-c', 'core.fsmonitor=false', '-c', 'core.quotepath=false', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', '.'],
      environment: const {'GIT_TERMINAL_PROMPT': '0', 'GIT_OPTIONAL_LOCKS': '0', 'GIT_LITERAL_PATHSPECS': '1'},
    );
  } on ProcessException {
    return null;
  }
  final bytes = <int>[];
  var tooMuch = false;
  final done = process.stdout.forEach((chunk) {
    if (bytes.length + chunk.length > _gitOutputLimit) {
      tooMuch = true;
      process.kill();
    } else {
      bytes.addAll(chunk);
    }
  });
  unawaited(process.stderr.drain<void>());
  try {
    final code = await process.exitCode.timeout(const Duration(seconds: 10));
    await done;
    if (code != 0 || tooMuch) return null;
  } on TimeoutException {
    process.kill();
    return null;
  }
  final base = folder.endsWith(Platform.pathSeparator) || folder.endsWith('/') ? folder.substring(0, folder.length - 1) : folder;
  final files = <String>[];
  for (final path in utf8.decode(bytes, allowMalformed: true).split('\u0000')) {
    if (path.isEmpty || _unsafeName.hasMatch(path) || path.startsWith('/') || path.contains(r'\') || path.split('/').any((part) => part == '..' || part == '.')) {
      continue;
    }
    // A tracked file that was deleted, a link and a submodule are not offered.
    final local = '$base${Platform.pathSeparator}${path.replaceAll('/', Platform.pathSeparator)}';
    if (FileSystemEntity.typeSync(local, followLinks: false) != FileSystemEntityType.file) continue;
    files.add(path);
  }
  final entries = <String>{...files};
  for (final path in files) {
    final parts = path.split('/');
    for (var i = 1; i < parts.length; i++) {
      entries.add('${parts.sublist(0, i).join('/')}/');
    }
  }
  int depth(String entry) => '/'.allMatches(entry.endsWith('/') ? entry.substring(0, entry.length - 1) : entry).length;
  final ordered = entries.toList()
    ..sort((a, b) => depth(a) != depth(b) ? depth(a).compareTo(depth(b)) : a.compareTo(b));
  return ordered.take(maxEntries).toList();
}

/// The file or folder [relative] names under [folder], or null when it names anything else: a path that climbs, is absolute or
/// has a drive, backslash or control character, one that is not there, and one that is, or leads through, a link out of [folder].
InsideFile? resolveInside(String folder, String relative) {
  if (relative.isEmpty || _unsafeName.hasMatch(relative) || relative.contains(r'\') || relative.startsWith('/')) return null;
  if (RegExp(r'^[A-Za-z]:').hasMatch(relative)) return null;
  final parts = relative.split('/');
  if (parts.last.isEmpty) parts.removeLast();
  if (parts.isEmpty || parts.any((part) => part.isEmpty || part == '.' || part == '..')) return null;
  final sep = Platform.pathSeparator;
  final base = folder.endsWith(sep) || folder.endsWith('/') ? folder.substring(0, folder.length - 1) : folder;
  final candidate = '$base$sep${parts.join(sep)}';
  try {
    final rootReal = Directory(folder).resolveSymbolicLinksSync();
    final type = FileSystemEntity.typeSync(candidate);
    if (type != FileSystemEntityType.file && type != FileSystemEntityType.directory) return null;
    final isFolder = type == FileSystemEntityType.directory;
    final real = isFolder ? Directory(candidate).resolveSymbolicLinksSync() : File(candidate).resolveSymbolicLinksSync();
    final inside = Platform.isWindows
        ? real.toLowerCase().startsWith('${rootReal.toLowerCase()}$sep')
        : real.startsWith('$rootReal$sep');
    if (!inside) return null;
    return (path: candidate, folder: isFolder, size: isFolder ? 0 : File(candidate).lengthSync());
  } on FileSystemException {
    return null;
  }
}
