// The client side of delegation: the tasks a session handed to other sources, built from the host's events; what a task's agent asks
// of the person, which no mode of the conversation's own answers for it; and Stop.
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/delegate.dart';
import 'package:canoa/chat/modes.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';

Future<void> _settle() => Future<void>.delayed(const Duration(milliseconds: 20));

void main() {
  group('the task list', () {
    test('events build it in the order tasks were first heard of, and a task keeps its place', () {
      final tasks = DelegateTasks();
      tasks.apply(taskNotice('b', taskEvent('b', 'SUBMITTED', title: 'Second', source: 'claude'))['params']);
      tasks.apply(taskNotice('a', taskEvent('a', 'SUBMITTED', title: 'First', at: '2026-11-13T10:00:00Z'))['params']);
      tasks.apply(taskNotice('b', statusEvent('WORKING', at: '2026-11-13T10:00:01Z'))['params']);
      tasks.apply(taskNotice('a', statusEvent('INPUT_REQUIRED', note: 'waiting for the person: First'))['params']);
      expect([for (final task in tasks.all) task.id], ['b', 'a']);
      expect([for (final task in tasks.all) task.title], ['Second', 'First']);
      expect(tasks['b']!.source, 'claude');
      expect(tasks['b']!.phase, TaskPhase.working);
      expect(tasks['a']!.phase, TaskPhase.inputRequired);
      expect(tasks['a']!.message, 'waiting for the person: First');
      expect(tasks['a']!.started, DateTime.utc(2026, 11, 13, 10));
      expect([for (final task in tasks.live) task.id], ['b', 'a']);
    });

    test('a duplicate changes nothing, a late state cannot reopen an ended task, and a missing end time is no end time', () {
      final tasks = DelegateTasks();
      final first = taskNotice('a', taskEvent('a', 'SUBMITTED'))['params'];
      tasks.apply(first);
      tasks.apply(first);
      expect(tasks.all, hasLength(1));
      tasks.apply(taskNotice('a', statusEvent('COMPLETED'))['params']);
      expect(tasks['a']!.phase, TaskPhase.completed);
      expect(tasks['a']!.ended, isNull, reason: 'the host sent no time, and none is made up');
      tasks.apply(taskNotice('a', statusEvent('WORKING', at: '2026-11-13T10:00:05Z'))['params']);
      tasks.apply(taskNotice('a', statusEvent('FAILED', at: '2026-11-13T10:00:06Z'))['params']);
      expect(tasks['a']!.phase, TaskPhase.completed);
      expect(tasks['a']!.ended, isNull);
      expect(tasks.live, isEmpty);
    });

    test('a state for a task never announced lists it by its id, and what is not an event is ignored', () {
      final tasks = DelegateTasks();
      expect(tasks.apply(taskNotice('lone', statusEvent('WORKING'))['params']), isTrue);
      expect(tasks['lone']!.title, 'lone');
      expect(tasks.apply(null), isFalse);
      expect(tasks.apply({'taskId': 5}), isFalse);
      expect(tasks.apply({'taskId': 'x', 'event': 'nope'}), isFalse);
      expect(tasks.apply({'taskId': 'x'}), isFalse);
      expect(tasks.apply(taskNotice('lone', statusEvent('SOMETHING_NEW'))['params']), isTrue);
      expect(tasks['lone']!.phase, TaskPhase.working, reason: 'a state this client does not know changes nothing');
    });

    test('the reply is the result artifact, and the agent\'s updates are kept as it sent them', () {
      final tasks = DelegateTasks();
      tasks.apply(taskNotice('a', taskEvent('a', 'WORKING'))['params']);
      tasks.apply({
        'sessionId': 's1',
        'taskId': 'a',
        'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Hi'}},
      });
      tasks.apply(taskNotice('a', {
        'artifactUpdate': {
          'artifact': {
            'artifactId': 'result',
            'parts': [
              {'text': 'Done.'},
            ],
          },
        },
      })['params']);
      expect(tasks['a']!.result, 'Done.');
      expect(tasks['a']!.updates.single['sessionUpdate'], 'agent_message_chunk');
    });

    test('a session lists no more tasks than it holds, and forgets the oldest that ended before it refuses one', () {
      final tasks = DelegateTasks();
      for (var i = 0; i < maxTasks + 5; i++) {
        tasks.apply(taskNotice('t$i', taskEvent('t$i', 'WORKING'))['params']);
      }
      expect(tasks.all, hasLength(maxTasks));
      expect(tasks['t$maxTasks'], isNull, reason: 'every one runs: no room');

      tasks.apply(taskNotice('t3', statusEvent('COMPLETED'))['params']);
      tasks.apply(taskNotice('t1', statusEvent('FAILED'))['params']);
      expect(tasks.apply(taskNotice('new', taskEvent('new', 'WORKING'))['params']), isTrue);
      expect(tasks['new'], isNotNull);
      expect(tasks['t1'], isNull, reason: 'the oldest ended is forgotten');
      expect(tasks['t3'], isNotNull);
      expect(tasks.all, hasLength(maxTasks));
    });
  });

  group('on a connection', () {
    test('a client that answers permission requests says so in initialize, and one that cannot does not', () async {
      final store = SessionStore();
      final host = DelegateHost();
      await host.start(store);
      expect(host.sentOf('initialize').single['params']['_meta'], {delegateMeta: true});
      final silent = DelegateHost();
      await silent.start(SessionStore(), decide: false);
      expect(silent.sentOf('initialize').single['params']['_meta'], isNull);
      expect(silent.client.takesTasks, isFalse);
    });

    test('the host\'s events reach the conversation they name and no other', () async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      host.say(taskNotice('a', taskEvent('a', 'SUBMITTED')));
      host.say({
        'jsonrpc': '2.0',
        'method': delegateEventMethod,
        'params': {'sessionId': 'other', 'taskId': 'z', 'event': taskEvent('z', 'WORKING')},
      });
      host.say(taskNotice('a', statusEvent('WORKING')));
      host.say(taskNotice('b', taskEvent('b', 'WORKING', title: 'Lint', source: 'claude')));
      await _settle();
      expect([for (final task in session.tasks.all) task.id], ['a', 'b']);
      expect(session.tasks['a']!.phase, TaskPhase.working);
      expect(session.tasks['b']!.source, 'claude');
    });

    test('a connection that is lost ends the live tasks as canceled, and a reconnect asks the host for nothing', () async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      host.say(taskNotice('a', taskEvent('a', 'WORKING')));
      host.say(taskNotice('b', taskEvent('b', 'WORKING', title: 'Lint')));
      host.say(taskNotice('b', statusEvent('COMPLETED')));
      await _settle();
      await host.incoming.close();
      await _settle();
      expect(session.tasks['a']!.phase, TaskPhase.canceled);
      expect(session.tasks['b']!.phase, TaskPhase.completed, reason: 'an ended task stays as it ended');

      final again = DelegateHost();
      final client = VerifiedAgent.fromChild(
        label: 'Scripted',
        messages: again.incoming.stream,
        decider: store.askPermission,
        close: () async {},
        send: (line) async {
          final message = jsonDecode(line) as Map<String, dynamic>;
          again.sent.add(message);
          if (message['method'] == 'initialize') {
            again.reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
          }
        },
      );
      await client.initialize();
      store.attachVerifiedClient(session.id, client);
      await _settle();
      expect(again.sentOf(delegateHistoryMethod), isEmpty, reason: 'the host keeps no task for another connection');
      expect(session.tasks['a']!.phase, TaskPhase.canceled);
      await client.close();
    });
  });

  group('the wire', () {
    test('an answer the app did not ask for cannot resolve its history or stop request', () async {
      final sent = <Map<String, dynamic>>[];
      final wire = DelegateWire((line) async => sent.add(jsonDecode(line) as Map<String, dynamic>), optIn: true);
      var stopped = true;
      final stop = wire.stop('s1', timeout: const Duration(milliseconds: 300)).then((done) => stopped = done);
      final history = wire.history('s1', 't1', timeout: const Duration(milliseconds: 300));
      await Future<void>.delayed(const Duration(milliseconds: 20));
      final ids = [for (final message in sent) message['id'] as String];
      expect(ids, hasLength(2));
      expect(ids.toSet(), hasLength(2));
      for (final id in ids) {
        expect(id, matches(RegExp(r'^canoa-delegate-\d+-[0-9a-f]{16}$')));
      }
      // What an agent would guess: the numbering with no nonce.
      for (final guess in ['canoa-delegate-0', 'canoa-delegate-1', 'canoa-delegate-0-0000000000000000']) {
        expect(wire.takes(jsonEncode({'jsonrpc': '2.0', 'id': guess, 'result': {'updates': <Object?>[]}})), isFalse, reason: guess);
      }
      await stop;
      expect(stopped, isFalse, reason: 'no answer of the host came');
      expect(await history, isNull);

      // The host's own answer, with the id the app issued, does resolve.
      final again = wire.stop('s1');
      await Future<void>.delayed(const Duration(milliseconds: 20));
      final id = sent.last['id'];
      expect(wire.takes(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': <String, Object?>{}})), isTrue);
      expect(await again, isTrue);
      wire.close();
    });
  });

  group('what a task asks of the person', () {
    Future<(DelegateHost, ChatSession, SessionStore)> start() async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      host.say(taskNotice('t1', taskEvent('t1', 'WORKING')));
      host.say(taskNotice('t2', taskEvent('t2', 'WORKING', title: 'Lint', source: 'claude')));
      await _settle();
      return (host, session, store);
    }

    test('a task\'s request waits for the person with its task, and the conversation needs them', () async {
      final (host, session, store) = await start();
      host.ask(10, 'Edit a.txt', task: 't1');
      await _settle();
      final question = session.pendingPermissions.single;
      expect(question.task!.title, 'Fix the tests');
      expect(question.task!.source, 'codex');
      expect(session.needsYou, isTrue);
      expect(store.waiting, 1);
      expect(host.answerTo(10), isNull);
      store.choosePermission(session, question.options.firstWhere((option) => option.kind == 'allow_once'));
      await _settle();
      expect(host.answerTo(10)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
      expect(session.toolEntries, isEmpty, reason: 'a task\'s tool call is not a line of this conversation');
    });

    testWidgets('the card says who asks: the task\'s title and its source', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final store = SessionStore();
      final host = DelegateHost();
      await tester.runAsync(() => host.start(store));
      await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig()));
      host.say(taskNotice('t1', taskEvent('t1', 'WORKING')));
      host.ask(10, 'Edit a.txt', task: 't1');
      await tester.runAsync(_settle);
      await tester.pump();
      expect(find.byKey(const Key('task-asks')), findsOneWidget);
      expect(find.textContaining('Asked by a subagent: Fix the tests (codex)'), findsOneWidget);
    });

    test('allow for the session covers that task\'s next request of the kind and nobody else\'s', () async {
      final (host, session, store) = await start();
      host.ask(10, 'Edit a.txt', task: 't1');
      await _settle();
      store.choosePermission(session, session.pendingPermissions.single.options.firstWhere((option) => option.kind == 'allow_always'));
      await _settle();
      expect(host.answerTo(10)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-always'}});

      host.ask(11, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions, isEmpty, reason: 'the same task, the same request');
      expect(host.answerTo(11)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});

      host.ask(12, 'Edit a.txt');
      await _settle();
      expect(session.pendingPermissions.single.task, isNull, reason: 'the conversation\'s own agent is asked');
      expect(host.answerTo(12), isNull);
      store.cancelPermission(session);

      host.ask(13, 'Edit a.txt', task: 't2');
      await _settle();
      expect(session.pendingPermissions.single.task!.id, 't2', reason: 'another task is asked');
      expect(host.answerTo(13), isNull);
      store.cancelPermission(session);

      host.ask(14, 'Run rm -rf build', task: 't1');
      await _settle();
      expect(session.pendingPermissions.single.title, 'Run rm -rf build', reason: 'a different request of the same task is asked');
      store.cancelPermission(session);

      // It ends with the task: a request that names it afterwards is no task's.
      host.say(taskNotice('t1', statusEvent('COMPLETED')));
      await _settle();
      expect(session.taskAllowances, isEmpty);
      host.ask(15, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions.single.task, isNull);
      expect(host.answerTo(15), isNull);
      // The conversation's own "always" was never touched.
      expect(session.alwaysAnsweredAt, isNull);
    });

    test('a task that titles every request alike is asked about each different file or command', () async {
      final (host, session, store) = await start();
      host.ask(10, 'Edit', task: 't1');
      await _settle();
      store.choosePermission(session, session.pendingPermissions.single.options.firstWhere((option) => option.kind == 'allow_always'));
      host.ask(11, 'Edit', task: 't1', input: {'file_path': 'a.txt'});
      await _settle();
      expect(session.pendingPermissions, isEmpty, reason: 'the same file');
      host.ask(12, 'Edit', task: 't1', input: {'file_path': '/home/me/.ssh/authorized_keys'});
      await _settle();
      expect(session.pendingPermissions, hasLength(1), reason: 'another file is asked');
      expect(host.answerTo(12), isNull);
      store.cancelPermission(session);
      host.ask(13, 'Edit', task: 't1', input: {'unfamiliar': 'shape'});
      await _settle();
      expect(session.pendingPermissions, hasLength(1), reason: 'an input Canoa cannot read is not the one allowed');
    });

    test('a task that ends takes its waiting card with it', () async {
      final (host, session, _) = await start();
      host.ask(10, 'Edit a.txt', task: 't1');
      host.ask(11, 'Edit b.txt', task: 't2');
      await _settle();
      expect(session.pendingPermissions, hasLength(2));
      host.say(taskNotice('t1', statusEvent('FAILED')));
      await _settle();
      expect(session.pendingPermissions.single.task!.id, 't2');
    });

    test('Bypass answers a task\'s request only while the conversation is in Bypass', () async {
      final (host, session, store) = await start();
      host.ask(10, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions, hasLength(1), reason: 'not in Bypass: the person is asked');
      expect(host.answerTo(10), isNull);
      store.cancelPermission(session);

      await store.chooseMode(session, ModeEntry(ownBypass, 'Bypass', own: true));
      expect(session.bypass, isTrue);
      host.ask(11, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions, isEmpty);
      expect(host.answerTo(11)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
      expect(session.messages.last.text, contains('Bypass allowed: Edit a.txt for Fix the tests'));

      await store.leaveBypass(session);
      expect(session.bypass, isFalse);
      host.ask(12, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions, hasLength(1), reason: 'out of Bypass again: the next request is asked');
      expect(host.answerTo(12), isNull);
    });

    test('a question or a plan of a task is for the person even in Bypass, and still needs them', () async {
      final (host, session, store) = await start();
      await store.chooseMode(session, ModeEntry(ownBypass, 'Bypass', own: true));
      host.say({
        'jsonrpc': '2.0',
        'id': 20,
        'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'toolCall': {'toolCallId': 't1:q', 'title': 'Which one?', 'kind': 'other', 'rawInput': {'questions': <Object?>[]}},
          'options': permissionOptions,
        },
      });
      await _settle();
      expect(session.pendingPermissions.single.forPerson, isTrue);
      expect(session.needsYou, isTrue);
    });
  });

  group('Stop', () {
    test('sends one stop, and every task ends canceled, those waiting on the person included', () async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      host.say(taskNotice('a', taskEvent('a', 'WORKING')));
      host.say(taskNotice('b', taskEvent('b', 'WORKING', title: 'Lint')));
      host.say(taskNotice('c', taskEvent('c', 'WORKING', title: 'Docs')));
      host.say(taskNotice('c', statusEvent('COMPLETED')));
      await _settle();
      host.ask(10, 'Edit a.txt', task: 'a');
      await _settle();
      expect(session.pendingPermissions, hasLength(1));

      await store.cancelSession(session);
      final stops = host.sentOf(delegateStopMethod);
      expect(stops, hasLength(1));
      expect(stops.single['params'], {'sessionId': 's1'});
      expect(session.tasks['a']!.phase, TaskPhase.canceled);
      expect(session.tasks['b']!.phase, TaskPhase.canceled);
      expect(session.tasks['c']!.phase, TaskPhase.completed, reason: 'an ended task stays as it ended');
      expect(session.pendingPermissions, isEmpty);
      expect(host.answerTo(10)?['result'], {'outcome': {'outcome': 'cancelled'}});

      // Nothing left running: another Stop sends no second request.
      await store.cancelSession(session);
      expect(host.sentOf(delegateStopMethod), hasLength(1));
    });

    test('a conversation with no tasks sends no stop', () async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      await store.cancelSession(session);
      expect(host.sentOf(delegateStopMethod), isEmpty);
      expect(host.sentOf('session/cancel'), hasLength(1));
    });

    test('one task can be stopped by itself', () async {
      final store = SessionStore();
      final host = DelegateHost();
      final session = await host.start(store);
      host.say(taskNotice('a', taskEvent('a', 'WORKING')));
      host.say(taskNotice('b', taskEvent('b', 'WORKING', title: 'Lint')));
      await _settle();
      await store.stopTask(session, 'b');
      expect(host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1', 'taskId': 'b'});
      expect(session.tasks['b']!.phase, TaskPhase.canceled);
      expect(session.tasks['a']!.phase, TaskPhase.working);
    });
  });
}
