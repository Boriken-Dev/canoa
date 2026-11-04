// The ACP v2 session updates beyond messages and tools, each shaped as `@agentclientprotocol/sdk` schema/v2/schema.json has it,
// through the store and onto the screen.
import 'dart:convert';

import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/v2_agent.dart';

Future<(V2Agent, ChatSession, SessionStore)> _fed(WidgetTester tester) async {
  final agent = V2Agent();
  final (store, session) = await tester.runAsync(agent.start) as (SessionStore, ChatSession);
  return (agent, session, store);
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pumpAndSettle();
}

Future<void> _show(WidgetTester tester, SessionStore store) async {
  await tester.binding.setSurfaceSize(const Size(1200, 1400));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(CanoaApp(store: store));
}

void main() {
  testWidgets('plan_update replaces the plan panel, and a cancelled entry is not shown as still to do', (tester) async {
    final (agent, session, store) = await _fed(tester);
    await _show(tester, store);
    agent.update({'sessionUpdate': 'plan_update', 'plan': {'type': 'items', 'planId': 'p1', 'entries': [
      {'content': 'Read the code', 'priority': 'high', 'status': 'completed'},
      {'content': 'Change it', 'priority': 'high', 'status': 'in_progress'},
      {'content': 'Try another way', 'priority': 'low', 'status': 'cancelled'},
    ]}});
    await _settle(tester);
    expect(session.plan.map((entry) => entry.status), [PlanStatus.done, PlanStatus.running, PlanStatus.cancelled]);
    expect(find.text('Plan: 1 of 3'), findsOneWidget);

    agent.update({'sessionUpdate': 'plan_update', 'plan': {'type': 'items', 'planId': 'p1', 'entries': [
      {'content': 'Read the code', 'priority': 'high', 'status': 'completed'},
      {'content': 'Change it', 'priority': 'high', 'status': 'completed'},
    ]}});
    // A plan content type this client does not know leaves the plan as it is.
    agent.update({'sessionUpdate': 'plan_update', 'plan': {'type': 'markdown', 'planId': 'p2', 'text': '# later'}});
    await _settle(tester);
    expect(find.text('Plan: 2 of 2'), findsOneWidget);
  });

  testWidgets('tool_call_content_chunk adds one item to its tool, keeping what came before', (tester) async {
    final (agent, session, store) = await _fed(tester);
    await _show(tester, store);
    agent.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 't', 'title': 'Run the tests', 'kind': 'execute',
      'status': 'in_progress', 'content': [{'type': 'content', 'content': {'type': 'text', 'text': 'first'}}]});
    agent.update({'sessionUpdate': 'tool_call_content_chunk', 'toolCallId': 't',
      'content': {'type': 'content', 'content': {'type': 'text', 'text': 'second'}}});
    agent.update({'sessionUpdate': 'tool_call_content_chunk', 'toolCallId': 't',
      'content': {'type': 'terminal', 'terminalId': 'term'}});
    await _settle(tester);
    final tool = session.toolEntries['t']!.tool!;
    expect(tool.blocks.whereType<ToolText>().map((block) => block.text), ['first', 'second']);
    expect(tool.blocks.last, isA<ToolTerminal>());
    await tester.tap(find.text('Run the tests'));
    await tester.pumpAndSettle();
    expect(find.text('second'), findsOneWidget);
  });

  testWidgets('a notice shows in the conversation as the agent\'s word, with its severity', (tester) async {
    final (agent, session, store) = await _fed(tester);
    await _show(tester, store);
    agent.update({'sessionUpdate': 'notice', 'severity': 'warning', 'title': 'Rate limit close',
      'description': 'About 10 requests are left this hour.'});
    agent.update({'sessionUpdate': 'notice', 'severity': 'info', 'title': 'Model switched'});
    await _settle(tester);
    expect(session.messages.map((message) => message.role), [MessageRole.notice, MessageRole.notice]);
    expect(find.text('Warning from the agent: Rate limit close\nAbout 10 requests are left this hour.'), findsOneWidget);
    expect(find.text('From the agent: Model switched'), findsOneWidget);
  });

  testWidgets('a compaction is one entry, updated by its id, with its summary folded away', (tester) async {
    final (agent, session, store) = await _fed(tester);
    await _show(tester, store);
    agent.update({'sessionUpdate': 'compaction_summary_chunk', 'compactionId': 'c1', 'content': {'type': 'text', 'text': 'Fixed calc; '}});
    await _settle(tester);
    expect(find.text('Compacting the context…'), findsOneWidget, reason: 'a first chunk starts a compaction in progress');
    agent.update({'sessionUpdate': 'compaction_summary_chunk', 'compactionId': 'c1', 'content': {'type': 'text', 'text': 'tests pass.'}});
    agent.update({'sessionUpdate': 'compaction_update', 'compactionId': 'c1', 'status': 'completed'});
    agent.update({'sessionUpdate': 'compaction_update', 'compactionId': 'c2', 'status': 'failed', 'error': 'Too little room'});
    await _settle(tester);
    expect(session.messages.map((message) => message.role), [MessageRole.compaction, MessageRole.compaction]);
    expect(find.text('Context compacted'), findsOneWidget);
    expect(find.text('Compacting the context failed: Too little room'), findsOneWidget);
    await tester.tap(find.text('Context compacted'));
    await tester.pumpAndSettle();
    expect(find.text('Fixed calc; tests pass.'), findsOneWidget);

    // A whole summary replaces what the chunks built, and null clears it.
    agent.update({'sessionUpdate': 'compaction_update', 'compactionId': 'c1', 'status': 'completed',
      'summary': [{'type': 'text', 'text': 'One line.'}]});
    await _settle(tester);
    expect(session.messages.first.compaction!.summary, 'One line.');
    agent.update({'sessionUpdate': 'compaction_update', 'compactionId': 'c1', 'status': 'completed', 'summary': null});
    await _settle(tester);
    expect(session.messages.first.compaction!.summary, isEmpty);
  });

  testWidgets('an agent_thought patch without content keeps the thought, and one with content replaces it', (tester) async {
    final (agent, session, _) = await _fed(tester);
    agent.update({'sessionUpdate': 'agent_thought_chunk', 'messageId': 'th', 'content': {'type': 'text', 'text': 'Looking.'}});
    agent.update({'sessionUpdate': 'agent_thought', 'messageId': 'th', '_meta': {'done': true}});
    await _settle(tester);
    expect(session.messages.single.text, 'Looking.');
    agent.update({'sessionUpdate': 'agent_thought', 'messageId': 'th', 'content': [{'type': 'text', 'text': 'Decided.'}]});
    await _settle(tester);
    expect(session.messages.single.text, 'Decided.');
  });

  testWidgets('a replayed message says what went with it, and a live one says it once', (tester) async {
    final (agent, session, store) = await _fed(tester);
    await _show(tester, store);
    agent.update({'sessionUpdate': 'user_message', 'messageId': 'old1', 'content': [
      {'type': 'text', 'text': 'Do what the notes say.'},
      {'type': 'resource', 'resource': {'uri': 'attachment:notes.txt', 'text': '1. Add a line.'}},
    ]});
    await _settle(tester);
    expect(session.messages.map((message) => (message.role, message.text)), [
      (MessageRole.user, 'Do what the notes say.'),
      (MessageRole.notice, 'Sent with this: notes.txt'),
    ]);
    expect(find.text('Sent with this: notes.txt'), findsOneWidget);

    expect(store.addAttachment(session, 'plan.md', utf8.encode('# plan')), isNull);
    await tester.runAsync(() => store.sendText('And this one.'));
    await _settle(tester);
    expect(session.messages.where((message) => message.text == 'Sent with this: plan.md'), hasLength(1));
    expect(session.messages.where((message) => message.text == 'And this one.'), hasLength(1));
  });
}
