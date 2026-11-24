// A conversation in a window of the app's own engine: the window draws the very objects the main window holds, so there is no link,
// no second attachment and no copy, and what either window does is one change that both show.
import 'dart:async';

import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_output.dart' show SpeakChoices;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show Settlement;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show PermissionCard;
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart' show pumpApp;
import 'fixtures/fake_voice.dart';
import 'fixtures/popout_windows.dart';
import 'fixtures/scripted_agent.dart';

/// Lets what happens outside the test's clock (the agent's lines) happen, then draws.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 120)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 300));
}

typedef _Scene = (ScriptedAgent, ChatSession, SessionStore, PopoutHub, FakeWindows);

/// A conversation with a turn running and the agent waiting on an edit, in a store whose windows are the test's own engine's.
Future<_Scene> _running(WidgetTester tester, {ReplyReader? reader, ValueChanged<bool>? onVoiceMode}) async {
  await tester.binding.setSurfaceSize(const Size(900, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  addTearDown(store.dispose);
  final agent = ScriptedAgent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  final windows = FakeWindows(sameEngine: true);
  final hub = PopoutHub(store, windows, reader: reader, onVoiceMode: onVoiceMode);
  addTearDown(hub.dispose);
  unawaited(store.sendText('Fix addNumbers'));
  agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
  agent.ask(5, 'Edit a.txt');
  await _settle(tester);
  expect(session.pendingPermissions, hasLength(1));
  return (agent, session, store, hub, windows);
}

void main() {
  testWidgets('a window of the engine draws the conversation the main window holds: its turn, its messages and the request that waits', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    final window = windows.opened.single;
    expect(window.link, isNull, reason: 'nothing links it to the main window');
    expect(window.title, 'Fix calc');
    expect(hub.isPoppedOut(session), isTrue);
    await showLiveWindow(tester, window);

    expect(find.text('Fix addNumbers'), findsOneWidget);
    expect(find.text('Reading calc.js.'), findsOneWidget);
    expect(find.byType(PermissionCard), findsOneWidget);
    expect(find.byTooltip('Stop'), findsOneWidget, reason: 'the turn is running');
  });

  testWidgets('the window is attached to the conversation as a client of its own, and not a second copy of it', (tester) async {
    final (_, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    expect(hub.viewsOf(session), ['window-0']);
    expect(session.chat.clients.map((c) => c.id), contains('window-0'));
    expect(session.chat.clients.firstWhere((c) => c.id == 'window-0').name, 'the pop-out window');

    // Whatever the window shows is read from the main window's own model: a change there is on screen with no message between.
    await showLiveWindow(tester, windows.opened.single);
    store.rename(session, 'Renamed here');
    await tester.pump();
    expect(find.text('Renamed here'), findsOneWidget);

    await tester.runAsync(() => hub.dockBack(session));
    expect(hub.viewsOf(session), isEmpty);
    expect(session.chat.clients.map((c) => c.id), isNot(contains('window-0')));
  });

  testWidgets('the request is answered once from the window, by the window, and the turn goes on', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);

    await tester.tap(find.byKey(const Key('permission-primary')));
    await _settle(tester);
    expect(agent.answers().map((m) => m['id']), [5]);
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
    expect(session.pendingPermissions, isEmpty);
    final outcome = session.chat.settledPermissions.single.outcome!;
    expect((outcome.how, outcome.by), (Settlement.answered, 'window-0'));
    expect(find.byType(PermissionCard), findsNothing);
    expect(session.status, SessionStatus.working, reason: 'an answer does not end the turn');
  });

  testWidgets('an answer in the main window settles the window, which says where it was answered', (tester) async {
    final (agent, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);
    expect(find.byType(PermissionCard), findsOneWidget);

    store.choosePermission(session, session.pendingPermissions.first.options.last);
    await _settle(tester);
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
    expect(find.byType(PermissionCard), findsNothing);
    expect(find.text('Answered in Canoa.'), findsOneWidget);
  });

  testWidgets('the main window and the window both write at once: each message waits its turn in the one queue, and each shows both', (tester) async {
    final (agent, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);

    await store.sendText('from the main window');
    await tester.enterText(find.byType(TextField), 'from the window');
    await tester.pump();
    await tester.tap(find.byTooltip('Queue'));
    await _settle(tester);

    expect(session.queue.map((m) => m.text), ['from the main window', 'from the window']);
    expect(agent.sent.where((m) => m['method'] == 'session/prompt'), hasLength(1), reason: 'only the first message went');
    expect(find.text('2 messages wait for this turn to end.'), findsOneWidget, reason: 'the window shows what the main window queued');
    expect(session.queue.map((m) => m.from), [null, 'window-0'], reason: 'each message knows which window wrote it');
  });

  testWidgets('Stop in the window ends the turn the main window runs', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);
    await tester.tap(find.byTooltip('Stop'));
    await _settle(tester);
    expect(agent.sent.where((m) => m['method'] == 'session/cancel'), hasLength(1));
    expect(session.pendingPermissions, isEmpty);
  });

  testWidgets('the dock button closes the window and leaves the turn and the request as they were', (tester) async {
    final (agent, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);
    await tester.tap(find.byKey(const Key('dock-back')));
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(windows.opened.single.isClosed, isTrue);
    expect(windows.windows, isEmpty);
    expect(session.status, SessionStatus.working);
    expect(session.pendingPermissions, hasLength(1));
    expect(agent.answerTo(5), isNull);
  });

  testWidgets('a window closed by its person leaves the conversation as it was, and it can be popped out again', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    windows.opened.single.closeByUser();
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(session.chat.clients.map((c) => c.id), isNot(contains('window-0')));
    expect(session.pendingPermissions, hasLength(1));
    expect(session.status, SessionStatus.working);
    await tester.runAsync(() => hub.popOut(session));
    expect(windows.opened, hasLength(2));
    expect(hub.viewsOf(session), ['window-1']);
  });

  testWidgets('popping out a conversation that is out brings its window forward, and does not open another', (tester) async {
    final (_, session, _, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await tester.runAsync(() => hub.popOut(session));
    expect(windows.opened, hasLength(1));
    expect(windows.opened.single.focused, 1);
  });

  testWidgets('a conversation closed in the main window takes its window with it', (tester) async {
    final (_, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    await showLiveWindow(tester, windows.opened.single);
    await tester.runAsync(() => store.close(session));
    await _settle(tester);
    expect(hub.isPoppedOut(session), isFalse);
    expect(windows.opened.single.isClosed, isTrue);
  });

  testWidgets('a conversation\'s window follows its title', (tester) async {
    final (_, session, store, hub, windows) = await _running(tester);
    await tester.runAsync(() => hub.popOut(session));
    store.rename(session, 'Another name');
    expect(windows.opened.single.title, 'Another name');
  });

  testWidgets('the conversation\'s own header opens and brings back the window', (tester) async {
    final store = SessionStore();
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    final windows = FakeWindows(sameEngine: true);
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

  testWidgets('two conversations are out at once, each in its own window, and each window shows its own', (tester) async {
    final (_, first, store, hub, windows) = await _running(tester);
    final agent2 = ScriptedAgent();
    final second = await tester.runAsync(() => agent2.start(store)) as ChatSession;
    second.title = 'Second';
    await tester.runAsync(() => hub.popOut(first));
    await tester.runAsync(() => hub.popOut(second));
    expect(windows.windows.map((w) => w.title), ['Fix calc', 'Second']);
    expect(hub.viewsOf(first), ['window-0']);
    expect(hub.viewsOf(second), ['window-1']);

    await showLiveWindow(tester, windows.opened.last);
    expect(find.byType(PermissionCard), findsNothing, reason: 'the other conversation\'s request is not here');
    expect(find.text('Second'), findsOneWidget);
  });

  group('reading aloud', () {
    testWidgets('a window of the engine reads with the app\'s own reader, and voice mode follows its conversation until it goes', (tester) async {
      final output = FakeOutput('stand-in', 'Stand-in', hold: true);
      final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices()), player: FakePlayer());
      addTearDown(reader.dispose);
      final kept = <bool>[];
      final (_, session, _, hub, windows) = await _running(tester, reader: reader, onVoiceMode: kept.add);
      await tester.runAsync(() => hub.popOut(session));
      await showLiveWindow(tester, windows.opened.single);

      await tester.tap(find.byKey(const Key('composer-plus')));
      await _settle(tester);
      await tester.tap(find.byKey(const Key('voice-mode')));
      await _settle(tester);
      expect(find.byKey(const Key('voice-mode-on')), findsOneWidget);
      expect(reader.voiceMode, isTrue);
      expect(reader.pinned, session.id);
      expect(kept, [true]);

      await tester.runAsync(() => hub.dockBack(session));
      expect(reader.pinned, isNull, reason: 'voice mode goes back to following the main window');
    });
  });
}
