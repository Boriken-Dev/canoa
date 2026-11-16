import 'dart:io';

import 'open_in.dart';

OpenInSystem systemOpenIn() => _IoSystem();

final class _IoSystem implements OpenInSystem {
  @override
  HostOs? get os => Platform.isWindows
      ? HostOs.windows
      : Platform.isMacOS
          ? HostOs.macos
          : Platform.isLinux
              ? HostOs.linux
              : null;

  @override
  Map<String, String> get environment => Platform.environment;

  @override
  bool isDirectory(String path) => FileSystemEntity.typeSync(path) == FileSystemEntityType.directory;

  @override
  bool isFile(String path) => FileSystemEntity.typeSync(path) == FileSystemEntityType.file;

  @override
  String? registryDefault(String key) {
    if (!Platform.isWindows) return null;
    final root = Platform.environment['SystemRoot'] ?? Platform.environment['SYSTEMROOT'];
    if (root == null) return null;
    try {
      final result = Process.runSync('$root\\System32\\reg.exe', ['query', key, '/ve']);
      return result.exitCode == 0 ? parseRegistryDefault('${result.stdout}', Platform.environment) : null;
    } on ProcessException {
      return null;
    }
  }

  @override
  String? bundlePath(String id) {
    if (!Platform.isMacOS) return null;
    try {
      final result = Process.runSync('mdfind', ['kMDItemCFBundleIdentifier == "$id"']);
      if (result.exitCode == 0) {
        for (final line in '${result.stdout}'.split('\n')) {
          if (line.endsWith('.app') && isDirectory(line)) return line;
        }
      }
    } on ProcessException {
      // Not there: the usual places are tried.
    }
    final home = Platform.environment['HOME'];
    for (final folder in ['/Applications', if (home != null) '$home/Applications']) {
      final app = '$folder/Visual Studio Code.app';
      if (isDirectory(app)) return app;
    }
    return null;
  }

  @override
  Future<void> start(String executable, List<String> arguments) async {
    try {
      // No shell, and nothing is waited for: Explorer, for one, answers 1 after it succeeds.
      await Process.start(executable, arguments, mode: ProcessStartMode.detached);
    } on ProcessException catch (error) {
      throw OpenInFailure('Could not start $executable: ${error.message}');
    }
  }
}
