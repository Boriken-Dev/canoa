// A real turn with a real agent, which spends its subscription, so it runs only when asked. It writes one file in a temporary
// directory, with the permission answered through the app's own store, and prints the shape of what the agent sent, never its text.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
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

Future<void> _idle(ChatSession session) async {
  final until = DateTime.now().add(const Duration(minutes: 3));
  while (session.status == SessionStatus.working && DateTime.now().isBefore(until)) {
    await Future<void>.delayed(const Duration(milliseconds: 200));
  }
}

/// The processes whose working directory is [dir] or below it, by `/proc`, which only Linux has in this form.
List<String> _workingIn(Directory dir) {
  final root = dir.resolveSymbolicLinksSync();
  final found = <String>[];
  for (final entry in Directory('/proc').listSync(followLinks: false)) {
    final pid = entry.uri.pathSegments.lastWhere((part) => part.isNotEmpty);
    if (int.tryParse(pid) == null) continue;
    try {
      final cwd = Link('/proc/$pid/cwd').targetSync();
      if (cwd == root || cwd.startsWith('$root/')) found.add(pid);
    } on FileSystemException {
      // Gone already, or another user's.
    }
  }
  return found;
}

String _said(ChatSession session) =>
    session.messages.where((message) => message.role == MessageRole.agent).map((message) => message.text).join(' ').toLowerCase();

void main() {
  final real = Platform.environment['CANOA_TEST_REAL_TURN'];
  // Two short turns, and a third program that opens the conversation again. Prints whether each thing held, never what was said.
  test('a real agent reads what is attached, and a later program can open that conversation again', () async {
    final command = agentPresets.firstWhere((preset) => preset.label == real);
    final dir = Directory.systemTemp.createTempSync('canoa-earlier-');
    final store = SessionStore(includePreview: false);
    final first = await startLocalAgent(command, decider: store.askPermission, workingDirectory: dir.path);
    final remote = await first.createSession(cwd: dir.path);
    final session = store.addProviderSession(title: 'T', agent: command.label, peerId: first.peerId, remoteSessionId: remote, cwd: dir.path);
    store.attachVerifiedClient(session.id, first);
    store.select(session.id);
    expect(store.canAttach(session), isTrue);
    expect(store.addAttachment(session, 'notes.txt', 'The secret word is mango.'.codeUnits), isNull);
    await store.sendText('Reply with only the secret word from the attached notes.');
    await _idle(session);
    final readIt = _said(session).contains('mango');
    stdout.writeln('ATTACHED TEXT READ $readIt');
    // A one-pixel red PNG.
    final red = base64Decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC');
    expect(store.addAttachment(session, 'pixel.png', red), isNull);
    await store.sendText('Reply with only the colour of the attached image, one word.');
    await _idle(session);
    final sawIt = _said(session).contains('red');
    stdout.writeln('ATTACHED IMAGE SEEN $sawIt');
    await first.close();

    final later = SessionStore(includePreview: false);
    final second = await startLocalAgent(command, decider: later.askPermission, workingDirectory: dir.path);
    final other = await second.createSession(cwd: dir.path);
    final here = later.addProviderSession(title: 'New', agent: command.label, peerId: second.peerId, remoteSessionId: other, cwd: dir.path);
    later.attachVerifiedClient(here.id, second);
    later.select(here.id);
    final page = await later.earlierSessions(here);
    stdout.writeln('LISTED ${page.sessions.length} THE ONE ${page.sessions.any((item) => item.id == remote)}');
    final earlier = page.sessions.firstWhere((item) => item.id == remote);
    final opened = await later.openEarlier(here, earlier);
    final replayedUser = opened.messages.where((message) => message.role == MessageRole.user).length;
    final replayedAgent = _said(opened).contains('mango');
    stdout.writeln('REPLAYED user messages $replayedUser, the answer $replayedAgent');
    await later.sendText('Without reading anything, what was the secret word? Reply with only that word.');
    await _idle(opened);
    final remembered = _said(opened).split('mango').length > 2;
    stdout.writeln('REMEMBERED $remembered');
    await second.close();
    expect(readIt, isTrue, reason: 'it has to have read the file it was sent');
    expect(sawIt, isTrue, reason: 'it has to have seen the image it was sent');
    expect(replayedAgent, isTrue, reason: 'opening it again has to show what was said');
    expect(remembered, isTrue, reason: 'and it has to be the same conversation');
    final closed = DateTime.now();
    while (true) {
      try {
        dir.deleteSync(recursive: true);
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closed) > const Duration(seconds: 30)) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
  }, skip: real == null ? 'set CANOA_TEST_REAL_TURN' : false, timeout: const Timeout(Duration(minutes: 12)));

  test('a real agent asks, is answered, and does it', () async {
    final command = agentPresets.firstWhere((preset) => preset.label == real);
    final dir = Directory.systemTemp.createTempSync('canoa-turn-');
    final settings = File('${dir.path}${Platform.pathSeparator}.claude${Platform.pathSeparator}settings.json');
    settings.parent.createSync();
    settings.writeAsStringSync('{"permissions": {"ask": ["Write", "Edit", "Bash", "PowerShell"]}}');
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
    final closed = DateTime.now();
    // Linux lets a directory go while a process still works in it, so ask /proc who is there.
    if (Platform.isLinux) {
      while (true) {
        final left = _workingIn(dir);
        if (left.isEmpty) {
          stdout.writeln('NO PROCESS LEFT AFTER ${DateTime.now().difference(closed).inMilliseconds} ms');
          break;
        }
        if (DateTime.now().difference(closed) > const Duration(seconds: 30)) {
          fail('still running there: $left');
        }
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
    // Windows holds a directory for as long as a process has it as its working directory, so this says when the last one left.
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
