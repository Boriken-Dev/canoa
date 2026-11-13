// The client side of delegation: the tasks a session handed to other sources, built from the host's events; what a task's agent asks
// of the person, which no mode of the conversation's own answers for it; and Stop.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/delegate.dart';
import 'package:canoa/chat/modes.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> _notice(String task, Map<String, Object?> event) => {
  'jsonrpc': '2.0',
  'method': delegateEventMethod,
  'params': {'sessionId': 's1', 'taskId': task, 'event': event},
};

Map<String, Object?> _task(String id, String state, {String title = 'Fix the tests', String source = 'codex', String? at}) => {
  'task': {
    'id': id,
    'status': {'state': 'TASK_STATE_$state', 'timestamp': ?at},
    'metadata': {'title': title, 'source': source, 'folder': '/work'},
  },
};

Map<String, Object?> _status(String state, {String? at, String? note}) => {
  'statusUpdate': {
    'status': {
      'state': 'TASK_STATE_$state',
      'timestamp': ?at,
      if (note != null)
        'message': {
          'parts': [
            {'text': note},
          ],
        },
    },
  },
};

const _options = [
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'allow-always', 'name': 'Yes, for the session', 'kind': 'allow_always'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

/// An agent in memory, speaking v1, that answers the app's own requests to the host the way the host does.
final class _Host {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  /// What the host answers a `history` request with, by task.
  final histories = <String, List<Object?>>{};

  Future<ChatSession> start(SessionStore store, {String label = 'Scripted', bool decide = true}) async {
    client = VerifiedAgent.fromChild(
      label: label,
      messages: incoming.stream,
      decider: decide ? store.askPermission : null,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        switch (message['method']) {
          case 'initialize':
            reply(id, {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
          case 'session/new':
            reply(id, {'sessionId': 's1'});
          case delegateStopMethod:
            reply(id, <String, Object?>{});
          case delegateHistoryMethod:
            reply(id, {'updates': histories[message['params']['taskId']] ?? const []});
          case 'session/prompt' || 'session/cancel' || null:
            break;
          default:
            if (id != null) reply(id, <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: label, agent: label, peerId: client.peerId, remoteSessionId: opened.id, cwd: '/work');
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void say(Map<String, Object?> message) => incoming.add(jsonEncode(message));

  /// A permission request of the conversation's own agent (`parent`), or of the task [task] (the host's `TASK:CALL`).
  void ask(int id, String title, {String? task, String call = 'c'}) => say({
    'jsonrpc': '2.0',
    'id': id,
    'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': task == null ? 'p$id' : '$task:$call$id', 'title': title, 'kind': 'edit', 'rawInput': {'file_path': 'a.txt'}},
      'options': _options,
      if (task != null)
        '_meta': {
          delegateMeta: {'taskId': task, 'title': 'Fix the tests', 'source': 'codex'},
        },
    },
  });

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;

  List<Map<String, dynamic>> sentOf(String method) => [for (final message in sent) if (message['method'] == method) message];
}

Future<void> _settle() => Future<void>.delayed(const Duration(milliseconds: 20));

void main() {
  group('the task list', () {
    test('events build it in the order tasks were first heard of, and a task keeps its place', () {
      final tasks = DelegateTasks();
      tasks.apply(_notice('b', _task('b', 'SUBMITTED', title: 'Second', source: 'claude'))['params']);
      tasks.apply(_notice('a', _task('a', 'SUBMITTED', title: 'First', at: '2026-11-13T10:00:00Z'))['params']);
      tasks.apply(_notice('b', _status('WORKING', at: '2026-11-13T10:00:01Z'))['params']);
      tasks.apply(_notice('a', _status('INPUT_REQUIRED', note: 'waiting for the person: First'))['params']);
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
      final first = _notice('a', _task('a', 'SUBMITTED'))['params'];
      tasks.apply(first);
      tasks.apply(first);
      expect(tasks.all, hasLength(1));
      tasks.apply(_notice('a', _status('COMPLETED'))['params']);
      expect(tasks['a']!.phase, TaskPhase.completed);
      expect(tasks['a']!.ended, isNull, reason: 'the host sent no time, and none is made up');
      tasks.apply(_notice('a', _status('WORKING', at: '2026-11-13T10:00:05Z'))['params']);
      tasks.apply(_notice('a', _status('FAILED', at: '2026-11-13T10:00:06Z'))['params']);
      expect(tasks['a']!.phase, TaskPhase.completed);
      expect(tasks['a']!.ended, isNull);
      expect(tasks.live, isEmpty);
    });

    test('a state for a task never announced lists it by its id, and what is not an event is ignored', () {
      final tasks = DelegateTasks();
      expect(tasks.apply(_notice('lone', _status('WORKING'))['params']), isTrue);
      expect(tasks['lone']!.title, 'lone');
      expect(tasks.apply(null), isFalse);
      expect(tasks.apply({'taskId': 5}), isFalse);
      expect(tasks.apply({'taskId': 'x', 'event': 'nope'}), isFalse);
      expect(tasks.apply({'taskId': 'x'}), isFalse);
      expect(tasks.apply(_notice('lone', _status('SOMETHING_NEW'))['params']), isTrue);
      expect(tasks['lone']!.phase, TaskPhase.working, reason: 'a state this client does not know changes nothing');
    });

    test('the reply is the result artifact, and the agent\'s updates are kept as it sent them', () {
      final tasks = DelegateTasks();
      tasks.apply(_notice('a', _task('a', 'WORKING'))['params']);
      tasks.apply({
        'sessionId': 's1',
        'taskId': 'a',
        'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Hi'}},
      });
      tasks.apply(_notice('a', {
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

    test('a session lists no more tasks than it holds', () {
      final tasks = DelegateTasks();
      for (var i = 0; i < maxTasks + 5; i++) {
        tasks.apply(_notice('t$i', _task('t$i', 'WORKING'))['params']);
      }
      expect(tasks.all, hasLength(maxTasks));
    });
  });

  group('on a connection', () {
    test('a client that answers permission requests says so in initialize, and one that cannot does not', () async {
      final store = SessionStore();
      final host = _Host();
      await host.start(store);
      expect(host.sentOf('initialize').single['params']['_meta'], {delegateMeta: true});
      final silent = _Host();
      await silent.start(SessionStore(), decide: false);
      expect(silent.sentOf('initialize').single['params']['_meta'], isNull);
      expect(silent.client.takesTasks, isFalse);
    });

    test('the host\'s events reach the conversation they name and no other', () async {
      final store = SessionStore();
      final host = _Host();
      final session = await host.start(store);
      host.say(_notice('a', _task('a', 'SUBMITTED')));
      host.say({
        'jsonrpc': '2.0',
        'method': delegateEventMethod,
        'params': {'sessionId': 'other', 'taskId': 'z', 'event': _task('z', 'WORKING')},
      });
      host.say(_notice('a', _status('WORKING')));
      host.say(_notice('b', _task('b', 'WORKING', title: 'Lint', source: 'claude')));
      await _settle();
      expect([for (final task in session.tasks.all) task.id], ['a', 'b']);
      expect(session.tasks['a']!.phase, TaskPhase.working);
      expect(session.tasks['b']!.source, 'claude');
    });

    test('a connection that is lost ends the live tasks as canceled, and a reconnect asks the host for each task\'s updates', () async {
      final store = SessionStore();
      final host = _Host();
      final session = await host.start(store);
      host.say(_notice('a', _task('a', 'WORKING')));
      host.say(_notice('b', _task('b', 'WORKING', title: 'Lint')));
      host.say(_notice('b', _status('COMPLETED')));
      await _settle();
      await host.incoming.close();
      await _settle();
      expect(session.tasks['a']!.phase, TaskPhase.canceled);
      expect(session.tasks['b']!.phase, TaskPhase.completed, reason: 'an ended task stays as it ended');

      final again = _Host()
        ..histories['a'] = [
          {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'half'}},
        ];
      final client = VerifiedAgent.fromChild(
        label: 'Scripted',
        messages: again.incoming.stream,
        decider: store.askPermission,
        close: () async {},
        send: (line) async {
          final message = jsonDecode(line) as Map<String, dynamic>;
          again.sent.add(message);
          switch (message['method']) {
            case 'initialize':
              again.reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
            case delegateHistoryMethod:
              again.reply(message['id'], {'updates': again.histories[message['params']['taskId']] ?? const []});
          }
        },
      );
      await client.initialize();
      store.attachVerifiedClient(session.id, client);
      await _settle();
      final asked = again.sentOf(delegateHistoryMethod);
      expect([for (final request in asked) request['params']['taskId']], ['a', 'b']);
      expect(asked.first['params']['sessionId'], 's1');
      expect(session.tasks['a']!.updates, hasLength(1));
      await client.close();
    });
  });

  group('what a task asks of the person', () {
    Future<(_Host, ChatSession, SessionStore)> start() async {
      final store = SessionStore();
      final host = _Host();
      final session = await host.start(store);
      host.say(_notice('t1', _task('t1', 'WORKING')));
      host.say(_notice('t2', _task('t2', 'WORKING', title: 'Lint', source: 'claude')));
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
      final host = _Host();
      await tester.runAsync(() => host.start(store));
      await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig()));
      host.say(_notice('t1', _task('t1', 'WORKING')));
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
      host.say(_notice('t1', _status('COMPLETED')));
      await _settle();
      expect(session.taskAllowances, isEmpty);
      host.ask(15, 'Edit a.txt', task: 't1');
      await _settle();
      expect(session.pendingPermissions.single.task, isNull);
      expect(host.answerTo(15), isNull);
      // The conversation's own "always" was never touched.
      expect(session.alwaysAnsweredAt, isNull);
    });

    test('a task that ends takes its waiting card with it', () async {
      final (host, session, _) = await start();
      host.ask(10, 'Edit a.txt', task: 't1');
      host.ask(11, 'Edit b.txt', task: 't2');
      await _settle();
      expect(session.pendingPermissions, hasLength(2));
      host.say(_notice('t1', _status('FAILED')));
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
          'options': _options,
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
      final host = _Host();
      final session = await host.start(store);
      host.say(_notice('a', _task('a', 'WORKING')));
      host.say(_notice('b', _task('b', 'WORKING', title: 'Lint')));
      host.say(_notice('c', _task('c', 'WORKING', title: 'Docs')));
      host.say(_notice('c', _status('COMPLETED')));
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
      final host = _Host();
      final session = await host.start(store);
      await store.cancelSession(session);
      expect(host.sentOf(delegateStopMethod), isEmpty);
      expect(host.sentOf('session/cancel'), hasLength(1));
    });

    test('one task can be stopped by itself', () async {
      final store = SessionStore();
      final host = _Host();
      final session = await host.start(store);
      host.say(_notice('a', _task('a', 'WORKING')));
      host.say(_notice('b', _task('b', 'WORKING', title: 'Lint')));
      await _settle();
      await store.stopTask(session, 'b');
      expect(host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1', 'taskId': 'b'});
      expect(session.tasks['b']!.phase, TaskPhase.canceled);
      expect(session.tasks['a']!.phase, TaskPhase.working);
    });
  });
}
