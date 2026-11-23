// Changing a running conversation's agent: the host exports it from the agent it is with and imports it into the other, which
// is started and opens it, so the whole conversation is there; switching back brings what was said since; an agent that cannot
// take a conversation says so in the picker; a refusal leaves the conversation where it was, with the reason, and its agent
// running. What the host does is a stand-in here (`FakeAgentMoves`); the host's own export and import are `agent_switch.rs`.
import 'dart:io' show Directory;

import 'package:canoa/history/agent_switch.dart';
import 'package:canoa/history/session_move.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

final class FakeAgentMoves implements AgentMoves {
  /// The moves the app asked for, in order.
  final moves = <({String entry, String session, String to, String? cwd})>[];

  /// What the host says each source can do; a source it does not list is not an agent of this computer.
  final ports = <String, Portability>{
    'Claude': const Portability(export: true, import: true),
    'Codex': const Portability(export: true, import: true),
    'Pi': const Portability(export: true, import: true),
    'Grok': const Portability(export: false, import: false, why: 'this provider does not seed a session in an agent it does not keep the sessions of'),
  };

  /// What the host answers a move with: a refusal, else a new session carrying [losses].
  String? refusal;
  List<MoveLoss> losses = const [];
  bool replayed = false;
  Object? portabilityFails;
  var _made = 0;

  @override
  Future<MoveReport> move({required String entry, required String session, required String to, String? cwd}) async {
    moves.add((entry: entry, session: session, to: to, cwd: cwd));
    if (refusal != null) throw StateError(refusal!);
    _made++;
    return MoveReport(session: 'moved-$_made', entry: to, cwd: cwd ?? '/work/repo', replayed: replayed, compacted: false, losses: losses);
  }

  @override
  Future<Map<String, Portability>> portability() async {
    if (portabilityFails != null) throw StateError('$portabilityFails');
    return Map.of(ports);
  }
}

Future<(SessionStore, OptionsAgent, ChatSession, FakeAgentMoves)> _running(WidgetTester tester, {FakeAgentMoves? moves}) async {
  final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
  final host = moves ?? FakeAgentMoves();
  final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
  await pumpApp(tester, store, linked: agent.openLink, agentMoves: host);
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  await settle(tester);
  await send(tester, 'Say hello');
  await settle(tester, 150);
  return (store, agent, store.selected!, host);
}

Future<void> _openPicker(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('option-model')));
  await frames(tester);
}

InkWell _tile(WidgetTester tester, String label) => tester.widget<InkWell>(find.byKey(Key('source $label')));

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/agent_switch_test.dart')));

  testWidgets('a running conversation goes to another agent with the whole conversation, and back, each time as the agent\'s own session', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    host.losses = const [MoveLoss('thinking', 2, null)];
    final claudeLink = agent.links.first;
    expect(session.agent, 'Claude');
    final first = session.remoteSessionId!;

    await _openPicker(tester);
    expect(_tile(tester, 'Codex').onTap, isNotNull, reason: 'the host said it can take one');
    expect(_tile(tester, 'Grok').onTap, isNull, reason: 'the host said it cannot');
    expect(find.text('Cannot take a conversation'), findsNWidgets(3), reason: 'Grok, which the host measured, and the two the host does not run as agents of its own');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/session_picker_agents_desktop.png'));
    await tester.tap(find.byKey(const Key('source Codex')));
    await settle(tester, 200);

    expect(host.moves.single.entry, 'Claude');
    expect(host.moves.single.session, first);
    expect(host.moves.single.to, 'Codex');
    expect(host.moves.single.cwd, '/work/repo');
    expect(agent.starts, ['Claude', 'Codex'], reason: 'the other agent was started; the first one stayed running until it had the conversation');
    expect((agent.last('session/load')['params'] as Map)['sessionId'], 'moved-1', reason: 'it opens the session the host made for it');
    expect(session.agent, 'Codex');
    expect(session.source?.label, 'Codex');
    expect(session.remoteSessionId, 'moved-1');
    expect(claudeLink.closed, isTrue, reason: 'the agent it left has ended');
    expect(store.ledger!.entries.single.entry, 'Codex', reason: 'the list follows the conversation');
    expect(store.ledger!.entries.single.sessionId, 'moved-1');
    expect(session.status, SessionStatus.connected);
    expect(find.textContaining('Agent: Claude → Codex.'), findsOneWidget);
    expect(find.textContaining('2 thoughts left out'), findsOneWidget, reason: 'what the move could not carry is in the same line');
    expect(find.text('Say hello'), findsWidgets, reason: 'what was said stays');

    await send(tester, 'Now on codex');
    expect(agent.methods.where((method) => method == 'session/prompt').length, 2, reason: 'the conversation works on the new agent');

    // Back: more was said on Codex, so Claude gets the whole conversation again, at another new id.
    await _openPicker(tester);
    expect(_tile(tester, 'Claude').onTap, isNotNull);
    await tester.tap(find.byKey(const Key('source Claude')));
    await settle(tester, 200);
    expect(host.moves.map((move) => (move.entry, move.session, move.to)), [('Claude', first, 'Codex'), ('Codex', 'moved-1', 'Claude')]);
    expect(session.agent, 'Claude');
    expect(session.remoteSessionId, 'moved-2');
    expect(agent.starts, ['Claude', 'Codex', 'Claude']);
    expect(find.textContaining('Agent: Codex → Claude.'), findsOneWidget);
    expect(store.ledger!.entries.single.entry, 'Claude');
    await finish(tester);
  });

  testWidgets('back to an agent that has heard nothing since: its own session as it was, and nothing is moved', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    final first = session.remoteSessionId!;
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('source Codex')));
    await settle(tester, 200);
    expect(host.moves, hasLength(1));
    // Nothing is said on Codex.
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('source Claude')));
    await settle(tester, 200);
    expect(host.moves, hasLength(1), reason: 'Claude\'s own session is the conversation: no export, no import');
    expect(session.agent, 'Claude');
    expect(session.remoteSessionId, first);
    expect((agent.last('session/load')['params'] as Map)['sessionId'], first);
    expect(find.textContaining('had its own copy of this conversation, and nothing was said since it left'), findsOneWidget);
    expect(store.ledger!.entries.single.sessionId, first);
    await finish(tester);
  });

  testWidgets('what the host measured of an agent is what the picker says of it', (tester) async {
    final host = FakeAgentMoves();
    host.ports.remove('Pi');
    final (_, agent, _, _) = await _running(tester, moves: host);
    await _openPicker(tester);
    expect(_tile(tester, 'Pi').onTap, isNull, reason: 'the host does not run it as an agent of its own');
    await tester.longPress(find.byKey(const Key('source Grok')));
    await frames(tester);
    expect(find.textContaining('cannot take a conversation from another agent'), findsWidgets);
    expect(find.textContaining('does not seed a session'), findsWidgets, reason: 'the reason is the provider\'s own');
    expect(agent.starts, ['Claude']);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    await finish(tester);
  });

  testWidgets('a refusal leaves the conversation with its agent, which was never stopped, and says why', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    host.refusal = 'Claude cannot export this conversation, and the host has no record of it: no session has this id';
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('source Codex')));
    await settle(tester, 200);
    expect(session.agent, 'Claude');
    expect(store.ledger!.entries.single.entry, 'Claude');
    expect(agent.starts, ['Claude'], reason: 'the other agent was not started for a conversation it could not be given');
    expect(find.textContaining('Agent: not moved to Codex: '), findsOneWidget);
    expect(find.textContaining('no session has this id'), findsOneWidget);
    expect(find.textContaining('stays with Claude'), findsOneWidget);
    expect(session.status, SessionStatus.connected);
    host.refusal = null;
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Still here');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1, reason: 'the first agent still takes the conversation');
    await finish(tester);
  });

  testWidgets('an agent that will not start leaves the conversation where it was, on the agent it had', (tester) async {
    final (store, agent, session, host) = await _running(tester);
    agent.failNextStarts = 1;
    await _openPicker(tester);
    await tester.tap(find.byKey(const Key('source Codex')));
    await settle(tester, 300);
    expect(host.moves, hasLength(1), reason: 'the host had made the copy before the start failed');
    expect(session.agent, 'Claude');
    expect(session.remoteSessionId, isNot('moved-1'));
    expect(store.ledger!.entries.single.entry, 'Claude');
    expect(find.textContaining('Agent: not moved to Codex: '), findsOneWidget);
    expect(find.textContaining('stays with Claude'), findsOneWidget);
    final prompts = agent.methods.where((method) => method == 'session/prompt').length;
    await send(tester, 'Still here');
    expect(agent.methods.where((method) => method == 'session/prompt').length, prompts + 1);
    await finish(tester);
  });

  testWidgets('while a turn runs the agents wait, and the picker says to wait', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..holdTurns = true;
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    await pumpApp(tester, store, linked: agent.openLink, agentMoves: FakeAgentMoves());
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(store.selected!.status, SessionStatus.working);
    await _openPicker(tester);
    expect(_tile(tester, 'Codex').onTap, isNull);
    await tester.longPress(find.byKey(const Key('source Codex')));
    await frames(tester);
    expect(find.textContaining('Wait for the turn to end'), findsWidgets);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    agent.finishTurn();
    await settle(tester);
    await finish(tester);
  });

  testWidgets('a host that cannot say which agents can take a conversation offers none, and says so', (tester) async {
    final host = FakeAgentMoves()..portabilityFails = 'no control channel';
    final (_, agent, _, _) = await _running(tester, moves: host);
    await _openPicker(tester);
    expect(_tile(tester, 'Codex').onTap, isNull);
    expect(find.text('Cannot take a conversation'), findsNothing, reason: 'unknown is not the same as no');
    await tester.longPress(find.byKey(const Key('source Codex')));
    await frames(tester);
    expect(find.textContaining('The host did not say'), findsWidgets);
    expect(agent.starts, ['Claude']);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    await finish(tester);
  });
}
