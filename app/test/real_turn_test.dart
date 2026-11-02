// A real turn with a real agent, which spends its subscription, so it runs only when asked. It writes one file in a temporary
// directory, with the permission answered through the app's own store, and prints the shape of what the agent sent, never its text.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

String _shape(Object? value, [int depth = 0]) => switch (value) {
  final Map<dynamic, dynamic> map when depth < 4 =>
    '{${map.entries.map((entry) => '${entry.key}: ${_shape(entry.value, depth + 1)}').join(', ')}}',
  final List<dynamic> list when depth < 4 => list.isEmpty ? '[]' : '[${_shape(list.first, depth + 1)} x${list.length}]',
  final String text when const {'pending', 'in_progress', 'completed', 'failed'}.contains(text) => text,
  String() => 'text',
  _ => '${value.runtimeType}',
};

void main() {
  final real = Platform.environment['CANOA_TEST_REAL_TURN'];
  test('a real agent asks, is answered, and does it', () async {
    final command = agentPresets.firstWhere((preset) => preset.label == real);
    final dir = Directory.systemTemp.createTempSync('canoa-turn-');
    final settings = File('${dir.path}${Platform.pathSeparator}.claude${Platform.pathSeparator}settings.json');
    settings.parent.createSync();
    settings.writeAsStringSync('{"permissions": {"ask": ["Write", "Edit", "Bash"]}}');
    final allow = Platform.environment['CANOA_TEST_REAL_TURN_ANSWER'] != 'refuse';
    final store = SessionStore(includePreview: false);
    final client = await startLocalAgent(command, decider: store.askPermission, workingDirectory: dir.path);
    final remote = await client.createSession(cwd: dir.path);
    final session = store.addProviderSession(title: 'T', agent: command.label, peerId: client.peerId, remoteSessionId: remote);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    final seen = <String, Set<String>>{};
    final watching = client.updates.listen((update) => (seen[update.kind] ??= {}).add(_shape(update.raw)));
    var answered = 0;
    final answering = Timer.periodic(const Duration(milliseconds: 200), (_) {
      if (session.pendingPermissions.isNotEmpty) {
        answered++;
        store.answerPermission(session, allow: allow);
      }
    });
    await store.sendText(
      'Run the shell command `echo hi`, read the file hello.txt if it exists, create hello.txt in the current directory '
      'containing the word hello, change that word to bye, then reply with the single word done.',
    );
    final until = DateTime.now().add(const Duration(minutes: 4));
    final written = File('${dir.path}${Platform.pathSeparator}hello.txt');
    while (session.status == SessionStatus.working && DateTime.now().isBefore(until)) {
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    answering.cancel();
    await watching.cancel();
    seen.forEach((kind, shapes) {
      for (final shape in shapes) {
        stdout.writeln('KIND $kind $shape');
      }
    });
    stdout.writeln('ANSWERED $answered WRITTEN ${written.existsSync()}');
    stdout.writeln('ENTRIES ${session.messages.map((message) => message.role.name).toSet()} TOOLS ${session.toolEntries.length} COMMANDS ${session.commands.length} USAGE ${session.usage.describe() != null}');
    await client.close();
    expect(answered, greaterThan(0), reason: 'the agent has to have asked');
    expect(written.existsSync(), allow, reason: 'a refusal has to stop it');
    // Windows holds a directory for as long as a process has it as its working directory, so this says when the last one left.
    final closed = DateTime.now();
    while (true) {
      try {
        dir.deleteSync(recursive: true);
        stdout.writeln('DIRECTORY FREE AFTER ${DateTime.now().difference(closed).inMilliseconds} ms');
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closed) > const Duration(seconds: 30)) {
          fail('something the agent started is still running there');
        }
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
  }, skip: real == null ? 'set CANOA_TEST_REAL_TURN' : false, timeout: const Timeout(Duration(minutes: 6)));
}
