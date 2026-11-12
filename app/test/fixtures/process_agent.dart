// A program speaking ACP on its standard streams, started by the test itself: for the tests of the store and the conversation
// that want a real agent process (the scripted `stdio_agent`) and nothing of the host. The app starts no program of its own.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';

/// Starts [command] as a child and speaks ACP to it; closing the client ends the child. [environment] is added to this
/// process's own.
Future<VerifiedAgent> startProcessAgent(
  AgentCommand command, {
  AgentDecider? decider,
  AgentElicitor? elicitor,
  String? workingDirectory,
  Map<String, String> environment = const {},
}) async {
  final process = await Process.start(
    command.program,
    command.args,
    workingDirectory: workingDirectory,
    environment: environment,
    // `dart` is a script on some systems, which only a shell starts.
    runInShell: Platform.isWindows,
  );
  final errors = StringBuffer();
  process.stderr.transform(utf8.decoder).listen(errors.write);
  var writing = Future<void>.value();
  final client = VerifiedAgent.fromChild(
    label: command.label,
    messages: process.stdout.transform(utf8.decoder).transform(const LineSplitter()),
    send: (message) {
      final written = writing.then((_) async {
        process.stdin.writeln(message);
        await process.stdin.flush();
      });
      writing = written.then((_) {}, onError: (Object _) {});
      return written;
    },
    close: () async {
      process.kill();
      await process.exitCode;
    },
    decider: decider,
    elicitor: elicitor,
  );
  try {
    await client.initialize();
    return client;
  } on Object catch (error) {
    await client.close();
    throw AcpFailure('${command.label} did not start: ${errors.isEmpty ? error : errors.toString().trim()}');
  }
}
