// Delegation as the person sees it: one row for the children a turn handed out, the line that says what the turn waits on, the
// session list's "Waiting", a child's thread to look at, and the rows the session inspector's Lineage list takes.

import 'package:canoa/acp/delegate.dart';
import 'package:canoa/chat/delegation_view.dart';
import 'package:canoa/rail/session_groups.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';
import 'fixtures/delegation_scene.dart';

void main() {
  group('the row', () {
    testWidgets('four children read 4 subagents and 4 working, then 2 working as two end, the time advances, and a failed child is marked', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      expect(find.byKey(const Key('delegation-row')), findsOneWidget, reason: 'one row for all four calls');
      expect(find.text('4 subagents'), findsOneWidget);
      expect(find.text('4 working'), findsOneWidget);
      expect(find.text('25s'), findsOneWidget);
      expect(find.text('canoa-delegate: delegate_task'), findsNothing, reason: 'the calls fold into the row');

      scene.now = scene.now.add(const Duration(seconds: 5));
      await tester.pump(const Duration(seconds: 1));
      expect(find.text('30s'), findsOneWidget);

      await scene.end('c0', 'COMPLETED');
      await scene.end('c1', 'FAILED', at: 22, note: 'the source would not start');
      expect(find.text('2 working'), findsOneWidget);
      expect(find.text('1 failed'), findsOneWidget);

      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      expect(find.text('Share link API'), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c1')), matching: find.text('Failed')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c1')), matching: find.byIcon(Icons.error_outline)), findsOneWidget);
      expect(find.text('the source would not start'), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c0')), matching: find.text('Done')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c0')), matching: find.text('20s')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('child-c2')), matching: find.text('Working')), findsOneWidget);
    });

    testWidgets('once none works the length stops, and the row says how it ended', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      for (final id in ['c0', 'c1', 'c2', 'c3']) {
        await scene.end(id, 'COMPLETED', at: 40);
      }
      expect(find.text('all done'), findsOneWidget);
      expect(find.text('40s'), findsOneWidget);
      scene.now = scene.now.add(const Duration(minutes: 5));
      await tester.pump(const Duration(seconds: 2));
      expect(find.text('40s'), findsOneWidget);
    });

    testWidgets('the delegation tool call folds into the row, and the same call from another server does not', (tester) async {
      final scene = await openScene(tester);
      scene.host.say(taskNotice('c0', taskEvent('c0', 'WORKING', source: 'claude', at: sceneAt(0))));
      scene.host.say(delegationCall('own', 'c0'));
      scene.host.say(delegationCall('status', 'c0', tool: 'task_status'));
      scene.host.say(delegationCall('other', 'c0', server: 'other-server'));
      scene.host.say(delegationCall('lookalike', 'c0', server: 'canoa-delegate-two'));
      await scene.settle();
      expect(find.text('canoa-delegate: delegate_task'), findsNothing);
      expect(find.text('canoa-delegate: task_status'), findsNothing);
      expect(find.text('other-server: delegate_task'), findsOneWidget);
      expect(find.text('canoa-delegate-two: delegate_task'), findsOneWidget);
      expect(find.text('1 subagent'), findsOneWidget);
    });

    testWidgets('a call with no task of this connection behind it stays as the agent sent it', (tester) async {
      final scene = await openScene(tester);
      await scene.say(delegationCall('own', 'gone'));
      expect(find.byKey(const Key('delegation-row')), findsNothing);
      expect(find.text('canoa-delegate: delegate_task'), findsOneWidget);
    });

    testWidgets('each turn has its own row, with the children its calls name', (tester) async {
      final scene = await openScene(tester);
      scene.host.say(taskNotice('a', taskEvent('a', 'COMPLETED', title: 'First', at: sceneAt(0))));
      scene.host.say(delegationCall('call-a', 'a'));
      scene.host.say({
        'jsonrpc': '2.0',
        'method': 'session/update',
        'params': {'sessionId': 's1', 'update': {'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': 'And another'}}},
      });
      scene.host.say(taskNotice('b', taskEvent('b', 'WORKING', title: 'Second', at: sceneAt(30))));
      scene.host.say(taskNotice('c', taskEvent('c', 'WORKING', title: 'Third', at: sceneAt(30))));
      scene.host.say(delegationCall('call-b', 'b'));
      scene.host.say(delegationCall('call-c', 'c'));
      await scene.settle();
      expect(find.byKey(const Key('delegation-row')), findsNWidgets(2));
      expect(find.text('1 subagent'), findsOneWidget);
      expect(find.text('2 subagents'), findsOneWidget);
    });
  });

  group('waiting', () {
    testWidgets('the line lists the titles while the turn runs, and its Stop stops all four', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      expect(find.byKey(const Key('delegation-waiting')), findsNothing, reason: 'no turn runs');
      await scene.turn();
      expect(find.text('Waiting on 4 subagents'), findsOneWidget);
      for (final title in childTitles) {
        expect(find.descendant(of: find.byKey(const Key('delegation-waiting')), matching: find.text(title)), findsOneWidget);
      }
      await tester.tap(find.byKey(const Key('delegation-stop')));
      await scene.settle();
      expect(scene.host.sentOf(delegateStopMethod), hasLength(1));
      expect(scene.host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1'});
      expect([for (final task in scene.session.tasks.all) task.phase], everyElement(TaskPhase.canceled));
      expect(find.byKey(const Key('delegation-waiting')), findsNothing);
      expect(find.text('4 stopped'), findsOneWidget);
      expect(scene.host.sentOf('session/cancel'), isEmpty, reason: 'the turn itself goes on');
    });

    testWidgets('the session list reads Waiting while a turn runs with a working child, and not after', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      expect(rowStateOf(scene.session), isNot(RowState.waiting), reason: 'no turn runs');
      await scene.turn();
      expect(rowStateOf(scene.session), RowState.waiting);
      expect(find.text('Waiting'), findsOneWidget);
      expect(find.text('Working'), findsOneWidget, reason: "only the header's own state");
      for (final id in ['c0', 'c1', 'c2', 'c3']) {
        await scene.end(id, 'COMPLETED');
      }
      expect(rowStateOf(scene.session), RowState.working);
      expect(find.text('Waiting'), findsNothing);
      expect(find.text('Working'), findsNWidgets(2), reason: "the list's row and the header's state");
    });

    testWidgets('a child that needs the person already raises the parent\'s own mark', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.turn();
      scene.host.ask(10, 'Edit a.txt', task: 'c0');
      await scene.settle();
      expect(rowStateOf(scene.session), RowState.needsYou);
    });
  });

  group('a child\'s thread', () {
    testWidgets('opens read only from its line with its pending approval, and Open parent comes back', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.say(childUpdate('c0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k1', 'title': 'Edit share.dart', 'kind': 'edit', 'status': 'completed'}));
      scene.host.ask(10, 'Edit a.txt', task: 'c0');
      await scene.settle();
      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      await tester.tap(find.byKey(const Key('child-c0')));
      await tester.pump();

      expect(scene.session.openChild, 'c0');
      expect(find.byKey(const Key('open-parent')), findsOneWidget);
      expect(find.byKey(const Key('delegation-row')), findsNothing);
      expect(find.text('Edit share.dart'), findsOneWidget, reason: 'the child\'s own updates, through the transcript\'s cards');
      expect(find.textContaining('read only'), findsOneWidget);
      expect(find.byType(Composer), findsNothing, reason: 'the person cannot write to it');
      expect(find.byKey(const Key('task-asks')), findsOneWidget, reason: 'its pending approval');
      expect(find.textContaining('to edit a.txt?'), findsOneWidget);

      await scene.say(childUpdate('c0', {'sessionUpdate': 'tool_call', 'toolCallId': 'k2', 'title': 'Run the tests', 'kind': 'execute', 'status': 'completed'}));
      expect(find.text('Run the tests'), findsOneWidget, reason: 'it follows the child as it goes on');
      expect(find.text('Edit share.dart'), findsOneWidget);

      await tester.tap(find.byKey(const Key('open-parent')));
      await tester.pump();
      expect(scene.session.openChild, isNull);
      expect(find.byKey(const Key('delegation-row')), findsOneWidget);
      expect(find.byType(Composer), findsOneWidget);
    });

    testWidgets('Stop in a child\'s thread stops that child only', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      scene.store.openChild(scene.session, 'c2');
      await tester.pump();
      await tester.tap(find.byKey(const Key('child-stop')));
      await scene.settle();
      expect(scene.host.sentOf(delegateStopMethod).single['params'], {'sessionId': 's1', 'taskId': 'c2'});
      expect(scene.session.tasks['c2']!.phase, TaskPhase.canceled);
      expect(scene.session.tasks['c0']!.phase, TaskPhase.working);
      expect(find.byKey(const Key('child-stop')), findsNothing);
    });

    testWidgets('a child the app heard nothing of asks the host for what it kept', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      scene.host.histories['c1'] = [
        {'sessionUpdate': 'tool_call', 'toolCallId': 'h1', 'title': 'Read the router', 'kind': 'read', 'status': 'completed'},
      ];
      scene.store.openChild(scene.session, 'c1');
      await scene.settle();
      expect(scene.host.sentOf(delegateHistoryMethod).single['params'], {'sessionId': 's1', 'taskId': 'c1'});
      expect(find.text('Read the router'), findsOneWidget);
    });
  });

  group('the lineage', () {
    testWidgets('gives the inspector a row per child, and opens one by its id', (tester) async {
      final scene = await openScene(tester);
      await scene.fourChildren();
      await scene.end('c0', 'COMPLETED');
      await scene.end('c3', 'FAILED', at: 22);
      final rows = lineage(scene.session);
      expect([for (final row in rows) row.title], childTitles);
      expect([for (final row in rows) row.provider], ['claude', 'codex', 'codex', 'grok']);
      expect([for (final row in rows) row.word], ['Done', 'Working', 'Working', 'Failed']);
      expect(rows[0].elapsedAt(scene.now), const Duration(seconds: 20));
      expect(rows[1].elapsedAt(scene.now), const Duration(seconds: 25));
      expect(rows.where((row) => row.live), hasLength(2), reason: 'Lineage · 2 running');

      scene.store.openChild(scene.session, rows[1].id);
      expect(scene.session.openChild, 'c1');
      scene.store.openChild(scene.session, 'nobody');
      expect(scene.session.openChild, 'c1', reason: 'a task that is not there opens nothing');
      scene.store.closeChild(scene.session);
      expect(scene.session.openChild, isNull);
    });

    testWidgets('lists nothing for a conversation that handed out nothing', (tester) async {
      final scene = await openScene(tester);
      expect(lineage(scene.session), isEmpty);
    });
  });
}

