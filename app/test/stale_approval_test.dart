// A permission card from a connection that is gone cannot be answered: the card goes, the tool line it was about is marked
// stopped, and nothing is sent. A session opened again is opened by `session/resume` where the agent offers it.
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/chat/permission_card.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

SessionStore _store([SessionLedger? ledger]) => SessionStore(includePreview: false, ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));

const _allow = [
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

Future<void> _started(WidgetTester tester, OptionsAgent agent, SessionStore store) async {
  await pumpApp(tester, store, linked: agent.openLink);
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
  await send(tester, 'Say hello');
}

ToolCall _tool(ChatSession session, String id) => session.toolEntries[id]!.tool!;

LedgerEntry _recorded(SessionLedger ledger, {String reopenBy = 'session/load'}) {
  ledger.record(peerId: 'local:Scripted', sessionId: 'o1', entry: 'Scripted', place: SessionPlace.local, reopenBy: reopenBy, cwd: '/w');
  return ledger.entries.single;
}

void main() {
  group('a card from a connection that is gone', () {
    testWidgets('after the agent went away is not shown, its line is stopped, and no answer is sent', (tester) async {
      final agent = OptionsAgent();
      final store = _store();
      await _started(tester, agent, store);
      agent.ask(11, _allow);
      await settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_tool(store.selected!, 't11').status, ToolStatus.pending);

      agent.disconnect();
      await settle(tester);
      expect(find.byType(PermissionCard), findsNothing, reason: 'gone, not disabled');
      expect(store.selected!.pendingPermissions, isEmpty);
      expect(_tool(store.selected!, 't11').status, ToolStatus.stopped);
      expect(find.text('stopped'), findsOneWidget);
      expect(agent.answers, isEmpty, reason: 'nobody was told anything for a request nobody can answer');
      await finish(tester);
    });

    testWidgets('after the conversation was connected to another agent is not shown, its line is stopped, and no answer is sent',
        (tester) async {
      final agent = OptionsAgent();
      final store = _store();
      await _started(tester, agent, store);
      final session = store.selected!;
      agent.ask(12, _allow);
      await settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      final option = session.pendingPermissions.first.options.first;

      await tester.runAsync(() async {
        final again = await agent.start(scripted);
        store.attachVerifiedClient(session.id, again, record: false);
      });
      await settle(tester);
      expect(find.byType(PermissionCard), findsNothing);
      expect(session.pendingPermissions, isEmpty);
      expect(_tool(session, 't12').status, ToolStatus.stopped);
      store.answerPermission(session, allow: true);
      store.choosePermission(session, option);
      await settle(tester);
      expect(agent.answers, isEmpty);
      await finish(tester);
    });

    testWidgets('a tool that finished before the connection ended stays as it was', (tester) async {
      final agent = OptionsAgent();
      final store = _store();
      await _started(tester, agent, store);
      agent.ask(13, _allow);
      await settle(tester);
      store.answerPermission(store.selected!, allow: true);
      store.selected!.toolEntries['t13']!.tool!.status = ToolStatus.done;
      agent.ask(14, _allow);
      await settle(tester);
      agent.disconnect();
      await settle(tester);
      expect(_tool(store.selected!, 't13').status, ToolStatus.done);
      expect(_tool(store.selected!, 't14').status, ToolStatus.stopped);
      await finish(tester);
    });
  });

  group('a session opened again after a restart', () {
    testWidgets('shows a tool its earlier run left waiting or running as stopped, and one that finished as done', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final agent = OptionsAgent()
        ..replay.addAll([
          {'sessionUpdate': 'tool_call', 'toolCallId': 'w1', 'title': 'Edit a.txt', 'kind': 'edit', 'status': 'pending'},
          {'sessionUpdate': 'tool_call', 'toolCallId': 'w2', 'title': 'Run tests', 'kind': 'execute', 'status': 'in_progress'},
          {'sessionUpdate': 'tool_call', 'toolCallId': 'w3', 'title': 'Read b.txt', 'kind': 'read', 'status': 'completed'},
        ]);
      final store = _store(ledger);
      await tester.runAsync(() async {
        final client = await agent.start(scripted);
        final session = await store.reopen(_recorded(ledger), client);
        expect(agent.methods, contains('session/load'));
        expect(_tool(session, 'w1').status, ToolStatus.stopped);
        expect(_tool(session, 'w2').status, ToolStatus.stopped);
        expect(_tool(session, 'w3').status, ToolStatus.done);
        expect(session.pendingPermissions, isEmpty, reason: 'the request itself is not replayed');
      });
    });
  });

  group('reopening', () {
    testWidgets('uses session/resume when the agent offers it beside session/load, and keeps what its answer said', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..offersResume = true;
      final store = _store(ledger);
      await tester.runAsync(() async {
        final client = await agent.start(scripted);
        expect(client.identity.reopenBy, 'session/resume');
        final session = await store.reopen(_recorded(ledger, reopenBy: 'session/resume'), client);
        expect(agent.methods, contains('session/resume'));
        expect(agent.methods, isNot(contains('session/load')));
        expect(session.options.map((option) => option.id), contains('mode'), reason: 'the answer to the resume was read');
      });
    });

    testWidgets('uses session/load when that is all the agent offers', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(ledger);
      await tester.runAsync(() async {
        final client = await agent.start(scripted);
        expect(client.identity.reopenBy, 'session/load');
        final session = await store.reopen(_recorded(ledger), client);
        expect(agent.methods, contains('session/load'));
        expect(agent.methods, isNot(contains('session/resume')));
        expect(session.options.map((option) => option.id), contains('mode'));
      });
    });
  });
}
