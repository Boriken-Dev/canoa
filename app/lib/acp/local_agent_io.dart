import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'agent_commands.dart';
import 'client.dart';

const bool canRunLocalAgents = true;

/// What a child may say on standard error before the oldest goes, for saying why it would not start.
const int _maxStderr = 8 * 1024;

/// How long a child gets to leave on its own once its input closes.
const Duration _leaving = Duration(seconds: 2);

/// A `.cmd` such as `npx` on Windows can only be started through a shell.
final bool _useShell = Platform.isWindows;

Future<String?> _find(String program) async {
  final finder = Platform.isWindows ? 'where' : 'which';
  try {
    final found = await Process.run(finder, [program], runInShell: _useShell);
    if (found.exitCode != 0) return null;
    final first = LineSplitter.split(found.stdout as String).where((line) => line.trim().isNotEmpty);
    return first.isEmpty ? null : first.first.trim();
  } on ProcessException {
    return null;
  }
}

/// Whether what it needs is on this computer, so nothing is offered that cannot start.
Future<bool> isInstalled(AgentCommand command) async {
  for (final program in [command.program, ...command.requires]) {
    if (await _find(program) == null) return false;
  }
  return true;
}

/// Everything under [pid], found while it can still be: a child that has died leaves its own children with no link back to it.
Future<List<int>> _descendants(int pid) async {
  final ProcessResult found;
  try {
    found = await Process.run('pgrep', ['-P', '$pid']);
  } on ProcessException {
    // A system without it still has to end the child itself.
    return const [];
  }
  final children = LineSplitter.split(found.stdout as String).map(int.tryParse).whereType<int>().toList();
  return [
    ...children,
    for (final child in children) ...await _descendants(child),
  ];
}

/// Ends a child and everything it started, which an adapter usually has. Input closes first where that can be seen through, since a
/// well-made agent leaves on its own. On Windows the child is a shell, and a shell can leave on that and strand what it started, so
/// the tree goes first: ACP keeps nothing that a clean exit would save.
Future<void> _end(Process process) async {
  if (Platform.isWindows) {
    await Process.run('taskkill', ['/T', '/F', '/PID', '${process.pid}']);
    await process.exitCode;
    return;
  }
  final tree = await _descendants(process.pid);
  try {
    await process.stdin.close();
  } on Object {
    // It has gone already.
  }
  await process.exitCode.timeout(_leaving, onTimeout: () {
    process.kill(ProcessSignal.sigkill);
    return process.exitCode;
  });
  for (final pid in tree) {
    Process.killPid(pid, ProcessSignal.sigkill);
  }
}

/// Starts an agent as a child of this app, and speaks ACP to it. A child is the person's own program, so there is no peer to prove,
/// and the client refuses nothing on its behalf: [decider] puts each permission request to the person.
Future<VerifiedAgent> startLocalAgent(
  AgentCommand command, {
  AcpDecider? decider,
  String? workingDirectory,
}) async {
  final environment = <String, String>{};
  for (final entry in command.programPaths.entries) {
    final path = await _find(entry.value);
    if (path == null) throw AcpFailure('${entry.value} is not installed');
    environment[entry.key] = path;
  }
  final Process process;
  try {
    process = await Process.start(
      command.program,
      command.args,
      workingDirectory: workingDirectory,
      environment: environment,
      runInShell: _useShell,
    );
  } on ProcessException catch (error) {
    throw AcpFailure('${command.label} could not start: ${error.message}');
  }
  final errors = StringBuffer();
  process.stderr.transform(utf8.decoder).listen((text) {
    errors.write(text);
    if (errors.length > _maxStderr) {
      final kept = errors.toString();
      errors
        ..clear()
        ..write(kept.substring(kept.length - _maxStderr));
    }
  });
  var closing = false;
  final client = VerifiedAgent.fromChild(
    label: command.label,
    messages: process.stdout.transform(utf8.decoder).transform(const LineSplitter()),
    send: (message) async {
      process.stdin.writeln(message);
      await process.stdin.flush();
    },
    close: () async {
      if (closing) return;
      closing = true;
      await _end(process);
    },
    decider: decider,
  );
  try {
    await client.initialize();
    return client;
  } on Object catch (error) {
    await client.close();
    final said = errors.toString().trim();
    throw AcpFailure('${command.label} did not start: ${said.isEmpty ? error : said}');
  }
}
