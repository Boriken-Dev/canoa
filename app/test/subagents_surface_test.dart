// The Subagents surface and the named child cards: the lineage as a tree, the states and the summary that counts each child once, a
// child's question on the collapsed row and in the list, a thread to read with its facts, and Stop at three reaches.

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/delegate.dart';
import 'package:canoa/chat/delegation_view.dart';
import 'package:canoa/rail/session_groups.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatTranscript;
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';
import 'fixtures/delegation_scene.dart';

/// The scrollable of the child's conversation, not of the facts above it.
Finder _transcriptScroll(String id) => find
    .descendant(of: find.descendant(of: find.byKey(ValueKey('body-$id')), matching: find.byType(ChatTranscript)), matching: find.byType(Scrollable))
    .first;

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _frames(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

/// Opens the Subagents surface in the bottom panel.
Future<void> _openSurface(WidgetTester tester) async {
  await tester.tap(_key('layout-bottom'));
  await _frames(tester);
  await tester.tap(_key('launch subagents'));
  await _frames(tester);
}

/// The call in which child [parent] handed task [child] out, as its agent reports it.
Map<String, Object?> _nestedCall(String parent, String child) => childUpdate(parent, {
  'sessionUpdate': 'tool_call',
  'toolCallId': 'nest-$child',
  'title': 'canoa-delegate: delegate_task',
  'kind': 'other',
  'status': 'completed',
  'content': [
    {'type': 'content', 'content': {'type': 'text', 'text': jsonEncode({'taskId': child, 'state': 'working'})}},
  ],
});

/// The first updates of the recorded subagent turn that has a tool call: the child has started and has not ended.
Future<void> _nativeStillWorking(Scene scene) async {
  final recorded = jsonDecode(File('test/fixtures/claude_subagent_updates.json').readAsStringSync()) as Map<String, dynamic>;
  for (final update in (recorded['subagent_tool'] as List).take(3)) {
    scene.host.say({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': update}});
  }
  await scene.settle();
}

/// The four children, and one more that the first handed out itself.
Future<void> _withNested(Scene scene) async {
  await scene.fourChildren();
  await scene.say(taskNotice('n0', taskEvent('n0', 'WORKING', title: 'Share link API tests', source: 'codex', at: sceneAt(5))));
  await scene.say(_nestedCall('c0', 'n0'));
}

void main() {
  group('the lineage', () {
    testWidgets('a task whose delegation call named another is its parent, and the one waiting on a task says so', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await _withNested(scene);
      final tasks = scene.session.tasks;
      expect(tasks.parentOf(tasks['n0']!)?.id, 'c0');
      expect(tasks.descendantsOf('c0').map((task) => task.id), ['n0']);
      final views = lineage(scene.session);
      expect([for (final view in views) view.id], ['c0', 'n0', 'c1', 'c2', 'c3'], reason: 'each task is followed by those it started');
      expect([for (final view in views) view.depth], [0, 1, 0, 0, 0]);
      expect(views.first.word, 'Waiting for a task');
      expect(views[1].parent, 'c0');

      await scene.end('n0', 'COMPLETED', at: 30);
      expect(lineage(scene.session).first.word, 'Working', reason: 'nothing below it runs any more');
    });

    testWidgets('the host\'s metadata can name the parent too, and a loop of parents still shows every task', (tester) async {
      final scene = await openScene(tester);
      await scene.say(taskNotice('a', {
        'task': {'id': 'a', 'status': {'state': 'TASK_STATE_WORKING'}, 'metadata': {'title': 'A', 'source': 'codex', 'parentTaskId': 'b'}},
      }));
      await scene.say(taskNotice('b', {
        'task': {'id': 'b', 'status': {'state': 'TASK_STATE_WORKING'}, 'metadata': {'title': 'B', 'source': 'codex', 'parentTaskId': 'a'}},
      }));
      expect(lineage(scene.session).map((view) => view.id), unorderedEquals(['a', 'b']));
    });
  });

  group('the card', () {
    testWidgets('counts every child once, and the counts and lengths stop with the children', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.end('c0', 'COMPLETED', at: 20);
      await scene.end('c1', 'INPUT_REQUIRED', at: 12);
      expect(DelegationView(lineage(scene.session)).summary, '4 subagents · 2 working · 1 needs input · 1 done');
      expect(find.text('2 working'), findsOneWidget);
      expect(find.text('1 needs input'), findsOneWidget);
      expect(find.text('1 done'), findsOneWidget);

      await scene.end('c1', 'COMPLETED', at: 25);
      await scene.end('c2', 'FAILED', at: 26, note: 'no login');
      await scene.end('c3', 'CANCELED', at: 27);
      expect(DelegationView(lineage(scene.session)).summary, '4 subagents · 2 done · 1 failed · 1 cancelled');
      expect(find.text('27s'), findsOneWidget);
      scene.now = scene.now.add(const Duration(hours: 2));
      await tester.pump(const Duration(seconds: 3));
      expect(find.text('27s'), findsOneWidget, reason: 'it froze when the last child ended');
      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      expect(find.descendant(of: find.byKey(const Key('child-c0')), matching: find.text('20s')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c3')), matching: find.text('Cancelled')), findsOneWidget);
    });

    testWidgets('a child\'s approval is on the closed card and the conversation is in Needs you', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.turn();
      expect(rowStateOf(scene.session), RowState.waiting);
      scene.host.ask(10, 'Edit a.txt', task: 'c1');
      await scene.settle();
      expect(find.byKey(const Key('delegation-row')), findsOneWidget);
      expect(find.byKey(const Key('child-c1')), findsNothing, reason: 'the card is closed');
      expect(find.byKey(const Key('child-asks-c1')), findsOneWidget);
      expect(find.text('Share link API asks: Edit a.txt'), findsOneWidget);
      expect(rowStateOf(scene.session), RowState.needsYou);
      expect(subagentsNeedingYou(scene.session), 1);

      await tester.tap(find.byKey(const Key('child-asks-c1')));
      await tester.pump();
      expect(scene.session.openChild, 'c1', reason: 'it opens the child, where the person answers');
    });

    testWidgets('a child that asks for input with no card yet still puts the conversation in Needs you', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      expect(rowStateOf(scene.session), isNot(RowState.needsYou));
      await scene.end('c2', 'INPUT_REQUIRED', at: 9, note: 'Which branch?');
      expect(rowStateOf(scene.session), RowState.needsYou);
      expect(find.text('Share link tests asks: Which branch?'), findsOneWidget);
      await scene.end('c2', 'COMPLETED', at: 11);
      expect(rowStateOf(scene.session), isNot(RowState.needsYou));
    });

    testWidgets('a child whose connection went away says Unknown with when it was last heard of, not that it was stopped', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.end('c0', 'COMPLETED', at: 10);
      await scene.say(childUpdate('c1', {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Still reading.'}}));
      await scene.host.incoming.close();
      await scene.settle();
      final views = lineage(scene.session);
      expect(views.first.word, 'Completed');
      expect(views[1].word, 'Unknown');
      expect(views[1].unknown, isTrue);
      expect(DelegationView(views).summary, '4 subagents · 1 done · 3 unknown');
      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      expect(find.descendant(of: find.byKey(const Key('child-c1')), matching: find.textContaining('last update')), findsOneWidget);
    });

    testWidgets('a conversation that was reopened keeps the subagents it recorded, and starts nothing', (tester) async {
      final scene = await openScene(tester);
      await scene.nativeChildren('subagents');
      expect(find.text('2 subagents'), findsOneWidget);
      expect(DelegationView(lineage(scene.session)).summary, '2 subagents · 2 done');
      scene.now = scene.now.add(const Duration(minutes: 30));
      await tester.pump(const Duration(seconds: 3));
      expect(DelegationView(lineage(scene.session)).summary, '2 subagents · 2 done');
      expect(scene.host.sentOf('session/prompt'), isEmpty);
      expect(scene.host.sentOf(delegateStopMethod), isEmpty);
    });
  });

  group('the surface', () {
    testWidgets('is offered once the session has a subagent, lists the lineage nested, and opens a child with its facts', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      expect(_key('layout-bottom'), findsNothing, reason: 'no subagent yet');
      await _withNested(scene);
      await _openSurface(tester);
      expect(find.byKey(const Key('subagents-summary')), findsOneWidget);
      expect(find.text('5 subagents · 5 working'), findsOneWidget);
      final parentX = tester.getTopLeft(find.descendant(of: find.byKey(const Key('subagent-c0')), matching: find.text('Dashboard share button'))).dx;
      final childX = tester.getTopLeft(find.descendant(of: find.byKey(const Key('subagent-n0')), matching: find.text('Share link API tests'))).dx;
      expect(childX, greaterThan(parentX), reason: 'the task it started is indented under it');
      expect(find.descendant(of: find.byKey(const Key('subagent-c0')), matching: find.text('Waiting for a task')), findsOneWidget);

      await scene.say(childUpdate('n0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k1', 'title': 'Run the tests', 'kind': 'execute', 'status': 'completed'}));
      await scene.say(childUpdate('n0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k2', 'title': 'Run the tests', 'kind': 'execute', 'status': 'completed'}));
      await scene.say(childUpdate('n0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k3', 'title': 'Read the router', 'kind': 'read', 'status': 'completed'}));
      await tester.tap(find.byKey(const Key('subagent-n0')));
      await _frames(tester);
      expect(find.byKey(const Key('subagent-back')), findsOneWidget);
      expect(find.byKey(const Key('subagent-expand')), findsOneWidget);
      expect(find.byKey(const Key('child-context')), findsOneWidget);
      expect(find.textContaining('started by Dashboard share button'), findsOneWidget);
      expect(find.text('Run the tests ×2 · Read the router'), findsOneWidget, reason: 'the tools it used, once each');
      expect(find.byKey(const Key('child-task')), findsNothing, reason: 'nobody supplied a task');
      expect(find.byKey(const Key('child-result')), findsNothing);

      expect(scene.session.openChild, isNull, reason: 'the surface reads a child without taking the conversation over');

      await tester.tap(find.byKey(const Key('subagent-expand')));
      await _frames(tester);
      expect(scene.session.openChild, 'n0');
      expect(find.byKey(const Key('open-parent')), findsOneWidget);
    });

    testWidgets('Back to parent goes to the task that started the child, and to the list from a first-level one', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await _withNested(scene);
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-n0')));
      await _frames(tester);
      expect(find.byKey(const Key('child-context')), findsOneWidget);
      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      expect(find.byKey(const Key('subagent-back')), findsOneWidget, reason: 'it is now c0\'s page');
      expect(find.textContaining('Waiting for a task'), findsWidgets);
      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      expect(find.byKey(const Key('subagent-back')), findsNothing);
      expect(find.byKey(const Key('subagents-list')), findsOneWidget);
    });

    testWidgets('a child shows the tokens the host reported for it, and Not reported for what nobody did', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await scene.fourChildren();
      await scene.say(taskNotice('c0', taskEvent('c0', 'COMPLETED', title: 'Share link API tests', source: 'claude', usage: {'input': 120, 'output': 30, 'cacheRead': 4200})));
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-c0')));
      await _frames(tester);
      expect(find.byKey(const Key('child-tokens')), findsOneWidget);
      expect(find.text('Input 120 · Output 30 · Cache read 4.2k · Cache write Not reported'), findsOneWidget);
      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('subagent-c1')));
      await _frames(tester);
      expect(find.text('Not reported'), findsOneWidget);
    });

    testWidgets('a native child shows the task it was given, its result, and that no progress was reported', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await scene.nativeChildren('subagents');
      await _openSurface(tester);
      final first = lineage(scene.session).first;
      await tester.tap(find.byKey(Key('subagent-${first.id}')));
      await _frames(tester);
      expect(find.byKey(const Key('child-task')), findsOneWidget);
      expect(find.byKey(const Key('child-result')), findsOneWidget);
      expect(find.byKey(const Key('child-not-reported')), findsOneWidget);
      expect(find.textContaining("started by the conversation's agent"), findsOneWidget);
      expect(find.byKey(Key('subagent-stop-${first.id}')), findsNothing);
    });

    testWidgets('each child\'s scroll is kept while the person reads another', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await scene.fourChildren();
      for (final id in ['c0', 'c1']) {
        for (var i = 0; i < 60; i++) {
          scene.host.say(childUpdate(id, {
            'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Line $i of $id.\n\n'},
          }));
        }
      }
      await scene.settle();
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-c0')));
      await _frames(tester);
      final first = _transcriptScroll('c0');
      final controller = tester.state<ScrollableState>(first).position;
      controller.jumpTo(40);
      await tester.pump();
      final at = controller.pixels;
      expect(at, isNonZero);

      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('subagent-c1')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('subagent-c0')));
      await _frames(tester);
      final again = _transcriptScroll('c0');
      expect(tester.state<ScrollableState>(again).position.pixels, at);
    });
  });

  group('Stop', () {
    testWidgets('one stops that child and says it is Cancelled; its descendants and the others go on', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await _withNested(scene);
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-stop-c0')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('stop-one-c0')));
      await scene.settle();
      await _frames(tester);
      expect(scene.host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1', 'taskId': 'c0'});
      expect(scene.session.tasks['c0']!.phase, TaskPhase.canceled);
      expect(scene.session.tasks['n0']!.phase, TaskPhase.working);
      expect(scene.session.tasks['c1']!.phase, TaskPhase.working);
      expect(find.text('Stopped Dashboard share button. It is Cancelled.'), findsOneWidget);
    });

    testWidgets('with descendants stops the child and what it started, deepest first', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await _withNested(scene);
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-stop-c0')));
      await _frames(tester);
      await tester.tap(find.byKey(const Key('stop-tree-c0')));
      await scene.settle();
      await _frames(tester);
      expect([for (final sent in scene.host.sentOf(delegateStopMethod)) (sent['params'] as Map)['taskId']], ['n0', 'c0']);
      expect(scene.session.tasks['c0']!.phase, TaskPhase.canceled);
      expect(scene.session.tasks['n0']!.phase, TaskPhase.canceled);
      expect(scene.session.tasks['c1']!.phase, TaskPhase.working);
      expect(find.text('Stopped Dashboard share button and what it started: 2 subagents Cancelled.'), findsOneWidget);
    });

    testWidgets('all stops every child the host runs, and says the conversation\'s own agent\'s children keep running', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await scene.fourChildren();
      await _nativeStillWorking(scene);
      await _openSurface(tester);
      expect(scene.session.tasks.live.where((task) => task.native), isNotEmpty);
      await tester.tap(find.byKey(const Key('subagents-stop-all')));
      await scene.settle();
      await _frames(tester);
      expect(scene.host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1'});
      expect(scene.session.tasks.hostLive, isEmpty);
      expect(find.textContaining('Stopped all: 4 subagents Cancelled.'), findsOneWidget);
      expect(find.textContaining("started by the conversation's agent keep running"), findsOneWidget);
    });

    testWidgets('the detail page stops the open child, and offers descendants only when there are some', (tester) async {
      final scene = await openScene(tester, size: const Size(1100, 1000));
      await _withNested(scene);
      await _openSurface(tester);
      await tester.tap(find.byKey(const Key('subagent-n0')));
      await _frames(tester);
      expect(find.byKey(const Key('subagent-stop')), findsOneWidget);
      expect(find.byKey(const Key('subagent-stop-tree')), findsNothing, reason: 'it started nothing');
      await tester.tap(find.byKey(const Key('subagent-back')));
      await _frames(tester);
      expect(find.byKey(const Key('subagent-stop-tree')), findsOneWidget, reason: 'back at c0, which started it');
      await tester.tap(find.byKey(const Key('subagent-stop-tree')));
      await scene.settle();
      await _frames(tester);
      expect(scene.session.tasks['n0']!.phase, TaskPhase.canceled);
      expect(find.byKey(const Key('subagent-stop')), findsNothing, reason: 'it ended');
    });
  });
}

