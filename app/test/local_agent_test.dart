// A real child process through the app's own code, with the person's answer going through the store.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

const _echo = AgentCommand(label: 'Echo', program: 'dart', args: ['test/fixtures/echo_acp.dart']);

Future<void> _until(bool Function() test, String what) async {
  final until = DateTime.now().add(const Duration(seconds: 30));
  while (!test()) {
    if (DateTime.now().isAfter(until)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

Future<(VerifiedAgent, ChatSession)> _running(SessionStore store) async {
  final client = await startLocalAgent(_echo, decider: store.askPermission);
  final remote = await client.createSession(cwd: Directory.current.path);
  final session = store.addProviderSession(title: 'Echo', agent: 'Echo', peerId: client.peerId, remoteSessionId: remote);
  store.attachVerifiedClient(session.id, client);
  return (client, session);
}

Future<bool> _alive(int pid) async {
  if (Platform.isWindows) {
    final listed = await Process.run('tasklist', ['/FI', 'PID eq $pid', '/NH']);
    return (listed.stdout as String).contains('$pid');
  }
  return (await Process.run('kill', ['-0', '$pid'])).exitCode == 0;
}

Future<void> _until2(Future<bool> Function() test, String what) async {
  final until = DateTime.now().add(const Duration(seconds: 15));
  while (!await test()) {
    if (DateTime.now().isAfter(until)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

void main() {
  // Starts a real adapter and opens a session, which sends no prompt and costs nothing. Needs it installed and signed in.
  final real = Platform.environment['CANOA_TEST_REAL_AGENT'];
  test('a real adapter starts and opens a session', () async {
    final command = agentPresets.firstWhere((preset) => preset.label == real);
    expect(await isInstalled(command), isTrue, reason: 'it would not have been offered');
    final client = await startLocalAgent(command);
    final session = await client.createSession(cwd: Directory.current.path);
    expect(session, isNotEmpty);
    await client.close();
  }, skip: real == null ? 'set CANOA_TEST_REAL_AGENT' : false, timeout: const Timeout(Duration(minutes: 5)));

  test('a turn runs, the agent has to ask, and what the person says reaches it', () async {
    final store = SessionStore();
    final (client, session) = await _running(store);
    store.select(session.id);
    unawaited(store.sendText('do something'));
    await _until(() => session.pendingPermissions.isNotEmpty, 'the agent to ask');
    expect(session.pendingPermissions.single.title, 'Run a command');
    expect(session.pendingPermissions.single.kind, 'execute');
    await Future<void>.delayed(const Duration(milliseconds: 100));
    expect(session.messages.any((message) => message.text.contains('the person said')), isFalse, reason: 'it waits for the person');

    store.answerPermission(session, allow: true);
    expect(session.pendingPermissions, isEmpty);
    await _until(() => session.messages.any((message) => message.text.contains('the person said')), 'the answer');
    expect(session.messages.last.text, contains('yes'));
    await client.close();
  }, timeout: const Timeout(Duration(minutes: 2)));

  test('refusing reaches the agent as a refusal', () async {
    final store = SessionStore();
    final (client, session) = await _running(store);
    store.select(session.id);
    unawaited(store.sendText('do something'));
    await _until(() => session.pendingPermissions.isNotEmpty, 'the agent to ask');
    store.answerPermission(session, allow: false);
    await _until(() => session.messages.any((message) => message.text.contains('the person said')), 'the answer');
    expect(session.messages.last.text, contains('no'));
    await client.close();
  }, timeout: const Timeout(Duration(minutes: 2)));

  test('an agent that goes away refuses what was waiting for the person', () async {
    final store = SessionStore();
    final (client, session) = await _running(store);
    store.select(session.id);
    unawaited(store.sendText('do something'));
    await _until(() => session.pendingPermissions.isNotEmpty, 'the agent to ask');
    await client.close();
    await _until(() => session.pendingPermissions.isEmpty, 'the question to go');
    expect(session.status, SessionStatus.disconnected);
  }, timeout: const Timeout(Duration(minutes: 2)));

  for (final stubborn in [false, true]) {
    test('closing leaves nothing running${stubborn ? ', even an agent that ignores its input closing' : ''}', () async {
      final dir = Directory.systemTemp.createTempSync('canoa-agent-');
      final file = File('${dir.path}${Platform.pathSeparator}pid');
      final client = await startLocalAgent(
        AgentCommand(label: 'Agent', program: 'dart', args: [
          'test/fixtures/echo_acp.dart',
          '--pid-file=${file.path}',
          if (stubborn) '--stubborn',
        ]),
      );
      final agent = int.parse(file.readAsStringSync());
      expect(await _alive(agent), isTrue, reason: 'the check has to be able to see it');
      await client.close();
      expect(client.isClosed, isTrue);
      await _until2(() async => !await _alive(agent), 'the agent to be gone');
      dir.deleteSync(recursive: true);
    }, timeout: const Timeout(Duration(minutes: 2)));
  }

  test('a program that is not there says so, and is not offered', () async {
    const missing = AgentCommand(label: 'Nothing', program: 'canoa-no-such-program');
    expect(await isInstalled(missing), isFalse);
    expect(await isInstalled(_echo), isTrue);
    await expectLater(startLocalAgent(missing), throwsA(isA<Exception>()));
  }, timeout: const Timeout(Duration(minutes: 1)));

  test('an adapter is not offered without the tool it drives', () async {
    const adapter = AgentCommand(label: 'Adapter', program: 'dart', requires: ['canoa-no-such-tool']);
    expect(await isInstalled(adapter), isFalse);
  });
}
