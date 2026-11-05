import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'host_tool.dart';

String _file(String name) => Platform.isWindows ? '$name.exe' : name;

HostTool? findHostTool() => findHostToolIn(
  environment: Platform.environment,
  directory: File(Platform.resolvedExecutable).parent.path,
  log: stderr.writeln,
);

/// The host `CANOA_HOST` in [environment] names, else the `canoa` in [directory] (a Windows runner's folder, or a Linux
/// bundle's root), else the `canoa-host` there from before the rename, which [log] says once.
HostTool? findHostToolIn({
  required Map<String, String> environment,
  required String directory,
  required void Function(String line) log,
}) {
  final named = environment['CANOA_HOST'];
  if (named != null && named.isNotEmpty) return ProcessHostTool(named);
  final beside = File('$directory${Platform.pathSeparator}${_file('canoa')}');
  if (beside.existsSync()) return ProcessHostTool(beside.path);
  final old = File('$directory${Platform.pathSeparator}${_file('canoa-host')}');
  if (!old.existsSync()) return null;
  log('found the host under its old name, ${old.path}: rename it to ${_file('canoa')}');
  return ProcessHostTool(old.path);
}

Future<int> freeLoopbackPort() async {
  final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final port = socket.port;
  await socket.close();
  return port;
}

Future<T> withFileBeside<T>(String path, String text, Future<T> Function(String file) use) async {
  final directory = File(path).parent;
  await directory.create(recursive: true);
  // A name of its own, created only if nothing is there: in a shared directory someone else could have put a file or a
  // link at a name known in advance.
  final random = Random.secure();
  final suffix = List.generate(16, (_) => random.nextInt(16).toRadixString(16)).join();
  final file = File('${directory.path}${Platform.pathSeparator}.canoa.cfg.$suffix.check');
  await file.create(exclusive: true);
  final writing = await file.open(mode: FileMode.writeOnlyAppend);
  try {
    await writing.writeString(text);
    await writing.flush();
  } finally {
    await writing.close();
  }
  try {
    return await use(file.path);
  } finally {
    try {
      await file.delete();
    } on FileSystemException {
      // Already gone.
    }
  }
}

/// `canoa` at [path], run with this app's environment plus [environment].
final class ProcessHostTool implements HostTool {
  ProcessHostTool(this.path, {this.environment = const {}});

  @override
  final String path;
  final Map<String, String> environment;

  @override
  Future<HostChild> start(List<String> args) async => _Child(await Process.start(path, args, environment: environment));

  @override
  Future<HostOutput> run(List<String> args) async {
    final ran = await Process.run(path, args, environment: environment, stdoutEncoding: utf8, stderrEncoding: utf8);
    return HostOutput(ran.exitCode, ran.stdout as String, ran.stderr as String);
  }
}

final class _Child implements HostChild {
  _Child(this._process)
    : lines = _process.stdout.transform(utf8.decoder).transform(const LineSplitter()).asBroadcastStream(),
      errors = _process.stderr.transform(utf8.decoder).transform(const LineSplitter()).asBroadcastStream();

  final Process _process;
  @override
  final Stream<String> lines;
  @override
  final Stream<String> errors;

  @override
  int get pid => _process.pid;

  @override
  Future<int> get exitCode => _process.exitCode;

  @override
  Future<void> closeInput() async {
    try {
      await _process.stdin.close();
    } on Object {
      // It has gone already.
    }
  }

  @override
  void kill() => _process.kill(ProcessSignal.sigkill);
}
