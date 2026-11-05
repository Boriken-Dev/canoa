import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:canoa_native/canoa_native.dart';

import 'host_tool.dart';

/// What another host is told holds their home.
const String _holder = 'CanoaUI';

/// How often a running command is asked what it has said. A read takes microseconds and starts no frame.
const Duration _pollEvery = Duration(milliseconds: 100);

HostTool? findHostTool() => findHostToolIn(environment: Platform.environment, open: CanoaNative.open, log: stderr.writeln);

/// The program `CANOA_HOST` in [environment] names, for tests, else the library [open] loads, else none, which [log] says.
HostTool? findHostToolIn({
  required Map<String, String> environment,
  required CanoaNative Function() open,
  required void Function(String line) log,
}) {
  final named = environment['CANOA_HOST'];
  if (named != null && named.isNotEmpty) return ProcessHostTool(named);
  try {
    return LibraryHostTool(open());
  } on Object catch (error) {
    log('the host library could not be loaded: $error');
    return null;
  }
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

/// `canoa` commands run in this process, through the library.
final class LibraryHostTool implements HostTool {
  LibraryHostTool(this._native, {this.home, this.program, this.programArgs});

  final CanoaNative _native;

  /// The home to use; this user's own when null.
  final String? home;

  /// What runs the host's own `provider`, `tool` and `launch`; this application, by its own flag, when null.
  final String? program;
  final List<String>? programArgs;

  @override
  String get path => _native.path;

  int _job(List<String> args) => _native.start({
    'args': args,
    'home': ?home,
    'holder': _holder,
    'program': ?program,
    'program_args': ?programArgs,
  });

  @override
  Future<HostChild> start(List<String> args) async => _LibraryChild(_native, _job(args));

  @override
  Future<HostOutput> run(List<String> args) async {
    final child = _LibraryChild(_native, _job(args));
    final (out, err) = (StringBuffer(), StringBuffer());
    final read = Future.wait([child.lines.forEach(out.writeln), child.errors.forEach(err.writeln)]);
    final status = await child.exitCode;
    await read;
    return HostOutput(status, out.toString(), err.toString());
  }
}

/// A command the library runs, read on a timer: nothing in the library calls back or blocks.
final class _LibraryChild implements HostChild {
  _LibraryChild(this._native, this._job) {
    _timer = Timer.periodic(_pollEvery, (_) => _read());
    // A command that ends at once need not cost a whole period.
    scheduleMicrotask(_read);
  }

  final CanoaNative _native;
  final int _job;
  final _out = StreamController<String>.broadcast();
  final _err = StreamController<String>.broadcast();
  final _exit = Completer<int>();
  late final Timer _timer;
  bool _inputClosed = false;

  /// Lines that came before anyone listened are kept for the first who does, as a process's pipe keeps them.
  final List<String> _outHeld = [], _errHeld = [];

  void _read() {
    if (_exit.isCompleted) return;
    final JobSaid said;
    try {
      said = _native.read(_job);
    } on Object catch (error) {
      _err.add('canoa: the host library failed: $error');
      return _ended(1);
    }
    _deliver(_out, _outHeld, said.out);
    _deliver(_err, _errHeld, said.err);
    if (said.dropped > 0) _deliver(_err, _errHeld, ['${said.dropped} lines of the log went unread']);
    final status = said.exit;
    if (status != null) _ended(status);
  }

  void _deliver(StreamController<String> to, List<String> held, List<String> lines) {
    if (to.hasListener) {
      held
        ..forEach(to.add)
        ..clear();
      lines.forEach(to.add);
    } else {
      held.addAll(lines);
    }
  }

  void _ended(int status) {
    _timer.cancel();
    try {
      _native.free(_job);
    } on Object {
      // Forgotten already.
    }
    _exit.complete(status);
    // A listener added late still gets its last lines before the streams close.
    Timer.run(() {
      _deliver(_out, _outHeld, const []);
      _deliver(_err, _errHeld, const []);
      unawaited(_out.close());
      unawaited(_err.close());
    });
  }

  @override
  int get pid => _thisProcess;

  @override
  Stream<String> get lines => _out.stream;

  @override
  Stream<String> get errors => _err.stream;

  @override
  Future<int> get exitCode => _exit.future;

  @override
  Future<void> closeInput() async {
    if (_inputClosed || _exit.isCompleted) return;
    _inputClosed = true;
    try {
      _native.stop(_job);
    } on Object {
      // It has gone already.
    }
  }

  /// There is no process to end. A command that did not leave when asked is abandoned to the process's own end, which ends
  /// its agents with it.
  @override
  void kill() {
    unawaited(closeInput());
    if (!_exit.isCompleted) _ended(-9);
  }
}

/// This process's id, the one every library command runs in.
int get _thisProcess => pid;

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
