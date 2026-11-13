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
