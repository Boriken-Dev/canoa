// A conversation shown in a window of its own: the window is a view of the conversation the main window holds, so a running turn and
// a waiting request survive popping out and docking back, and an answer given in either place settles it in both.
import 'dart:async';

import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show Settlement;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show PermissionCard, PermissionChoice;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart' show pumpApp;
import 'fixtures/popout_windows.dart';
import 'fixtures/scripted_agent.dart';

/// Lets what happens outside the test's clock (the agent's lines, a link's messages, the share's timer) happen, then draws.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 120)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
  // What the last frames set off (a session's events reach their listeners on the real clock) and its frames.
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 300));
}

Finder _choice(String name) => find.descendant(of: find.byType(PermissionCard), matching: find.widgetWithText(PermissionChoice, name));

/// A conversation with a turn running and the agent waiting on an edit, in a store whose pop-outs are the test's.
Future<(ScriptedAgent, ChatSession, SessionStore, PopoutHub, FakeWindows)> _running(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(900, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  addTearDown(store.dispose);
  final agent = ScriptedAgent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  final windows = FakeWindows();
  final hub = PopoutHub(store, windows, shareInterval: const Duration(milliseconds: 10));
  addTearDown(hub.dispose);
  // A turn that stays running: v1 answers a prompt when the turn ends, and this one never does.
  unawaited(store.sendText('Fix addNumbers'));
  agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
  agent.update({'sessionUpdate': 'tool_call', 'toolCallId': 'r', 'title': 'Read calc.js', 'kind': 'read', 'status': 'completed'});
  agent.ask(5, 'Edit a.txt');
  await _settle(tester);
  expect(session.status, SessionStatus.working);
  expect(session.pendingPermissions, hasLength(1));
  return (agent, session, store, hub, windows);
}

void main() {
  testWidgets('a conversation popped out shows what the main window holds: its turn, its messages and the request that waits', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    expect(hub.isPoppedOut(session), isTrue);
    expect(windows.opened.single.title, 'Fix calc');
    await showWindow(tester, windows.opened.single);

    expect(find.text('Fix calc'), findsOneWidget);
    expect(find.text('Fix addNumbers'), findsOneWidget);
    expect(find.text('Reading calc.js.'), findsOneWidget);
    expect(find.text('Read calc.js'), findsOneWidget);
    expect(find.byType(PermissionCard), findsOneWidget);
    expect(find.text('Edit a.txt'), findsWidgets);
    expect(find.byTooltip('Stop'), findsOneWidget, reason: 'the turn is running');
  });

  testWidgets('a running turn and a waiting request survive popping out and docking back, and are the same ones after', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);

    // Dock back with the window's own button.
    await tester.tap(find.byKey(const Key('dock-back')));
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(windows.opened.single.isClosed, isTrue);
    expect(session.status, SessionStatus.working, reason: 'the turn was not touched');
    expect(session.pendingPermissions, hasLength(1), reason: 'the request still waits');
    expect(agent.answerTo(5), isNull, reason: 'nothing answered it');

    // Out again: the same request is there to answer, once.
    await tester.runAsync(() => hub.popOut(session));
    expect(windows.opened, hasLength(2));
    await showWindow(tester, windows.opened.last);
    expect(find.byType(PermissionCard), findsOneWidget);
    await tester.tap(_choice('Yes'));
    await _settle(tester);
    expect(agent.answers().map((m) => m['id']), [5]);
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
    expect(session.pendingPermissions, isEmpty);
    expect(find.byType(PermissionCard), findsNothing, reason: 'the window shows it settled');
    expect(session.status, SessionStatus.working, reason: 'an answer does not end the turn');
  });

  testWidgets('an answer in the main window settles the window, which says where it was answered', (tester) async {
    final (agent, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    final view = await showWindow(tester, windows.opened.single);
    expect(find.byType(PermissionCard), findsOneWidget);

    store.choosePermission(session, session.pendingPermissions.first.options.last);
    await _settle(tester);
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
    expect(find.byType(PermissionCard), findsNothing);
    final outcome = view.session.settledPermissions.single.outcome!;
    expect((outcome.how, outcome.by), (Settlement.answered, 'main'));
    expect(find.text('Answered in Canoa.'), findsOneWidget);
  });

  testWidgets('of an answer in the window and one in the main window the first wins, and the agent hears one', (tester) async {
    final (agent, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);

    // The main window answers; before the window has been told, its person answers too.
    store.choosePermission(session, session.pendingPermissions.first.options.first);
    await tester.tap(_choice('No'));
    await _settle(tester);
    expect(agent.answers(), hasLength(1));
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
    expect(session.pendingPermissions, isEmpty);
    expect(find.byType(PermissionCard), findsNothing);
  });

  testWidgets('a second request waits behind the first, and an answer to the first is not taken for the second', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    agent.ask(6, 'Edit b.txt');
    await _settle(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);
    expect(find.textContaining('1 more'), findsOneWidget);

    await tester.tap(_choice('Yes'));
    await _settle(tester);
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
    expect(agent.answerTo(6), isNull);
    expect(session.pendingPermissions, hasLength(1));
    expect(find.text('Edit b.txt'), findsWidgets);
  });

  testWidgets('a message written in the window is sent through the main window\'s conversation, and waits there while a turn runs', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);

    await tester.enterText(find.byType(TextField), 'and add a test');
    await tester.tap(find.byTooltip('Queue'));
    await _settle(tester);
    expect(session.queue.map((m) => m.text), ['and add a test']);
    expect(agent.sent.where((m) => m['method'] == 'session/prompt'), hasLength(1), reason: 'only the first message went');
    expect(find.byKey(const Key('popout-queue')), findsOneWidget);
    expect(find.text('1 message waits for this turn to end.'), findsOneWidget);
  });

  testWidgets('Stop in the window ends the turn the main window runs', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);
    await tester.tap(find.byTooltip('Stop'));
    await _settle(tester);
    expect(agent.sent.where((m) => m['method'] == 'session/cancel'), hasLength(1));
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'cancelled'}}, reason: 'the request that waited is answered cancelled');
    expect(session.pendingPermissions, isEmpty);
  });

  testWidgets('a window closed by its person leaves the conversation as it was, and it can be popped out again', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    windows.opened.single.closeByUser();
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(session.pendingPermissions, hasLength(1));
    expect(session.status, SessionStatus.working);
    await tester.runAsync(() => hub.popOut(session));
    expect(windows.opened, hasLength(2));
  });

  testWidgets('popping out a conversation that is out brings its window forward, and does not open another', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await tester.runAsync(() => hub.popOut(session));
    expect(windows.opened, hasLength(1));
    expect(windows.opened.single.focused, 1);
  });

  testWidgets('a conversation closed in the main window leaves its window saying so', (tester) async {
    final (_, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showWindow(tester, windows.opened.single);
    await tester.runAsync(() => store.close(session));
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(find.text('The main window no longer holds this conversation.'), findsOneWidget);
    expect(find.byType(PermissionCard), findsNothing, reason: 'what waited was withdrawn with it');
  });

  testWidgets('a draft has nothing to show yet, and a platform with no windows offers none', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final draft = store.createDraft();
    final hub = PopoutHub(store, FakeWindows());
    addTearDown(hub.dispose);
    await expectLater(hub.popOut(draft), throwsStateError);
    expect(PopoutHub(store, const NoPopoutWindows()).available, isFalse);
  });

  testWidgets('the conversation\'s own header opens and brings back the window', (tester) async {
    final store = SessionStore();
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    final windows = FakeWindows();
    await pumpApp(tester, store, popoutWindows: windows);

    await tester.tap(find.byTooltip('Open in its own window'));
    await tester.pump();
    await tester.pump();
    expect(windows.opened, hasLength(1));
    expect(find.byTooltip('Bring back from its own window'), findsOneWidget);

    await tester.tap(find.byTooltip('Bring back from its own window'));
    await _settle(tester);
    expect(windows.opened.single.isClosed, isTrue);
    expect(find.byTooltip('Open in its own window'), findsOneWidget);
    expect(session.status, SessionStatus.connected);
  });

  testWidgets('no header action where the platform has no windows', (tester) async {
    final store = SessionStore();
    final agent = ScriptedAgent();
    await tester.runAsync(() => agent.start(store));
    await pumpApp(tester, store, popoutWindows: const NoPopoutWindows());
    expect(find.byTooltip('Open in its own window'), findsNothing);
  });
}
