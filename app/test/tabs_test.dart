// Sessions as tabs: a strip in the main window and in each window of its own, moving a tab between them, and the rule that a
// session is a tab of one window at a time. The windows are the fake seam's, drawn beside the app as the platform's root does.
import 'dart:async';

import 'package:canoa/popout/hub_tabs.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_view.dart' show PopoutChat;
import 'package:canoa/session_store.dart';
import 'package:canoa/tabs/session_tab_strip.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/popout_windows.dart';
import 'fixtures/scripted_agent.dart';

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 60)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

typedef _Scene = ({SessionStore store, PopoutHub hub, FakeWindows windows, List<ChatSession> sessions, List<ScriptedAgent> agents});

/// [count] sessions in a store, the last one selected, and a hub over windows of the engine drawn beside the app.
Future<_Scene> _scene(WidgetTester tester, {int count = 3, bool narrow = false}) async {
  await tester.binding.setSurfaceSize(narrow ? const Size(400, 800) : const Size(1500, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  addTearDown(store.dispose);
  final windows = FakeWindows(sameEngine: true, draw: true);
  final hub = PopoutHub(store, windows);
  addTearDown(hub.dispose);
  final sessions = <ChatSession>[];
  final agents = <ScriptedAgent>[];
  for (var i = 0; i < count; i++) {
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    store.rename(session, 'Session ${String.fromCharCode(65 + i)}');
    sessions.add(session);
    agents.add(agent);
  }
  await tester.pumpWidget(
    windows.host(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.light),
        home: Scaffold(body: Column(children: [HubTabStrip(hub: hub, windowKey: PopoutHub.mainKey, narrow: narrow), const Expanded(child: SizedBox())])),
      ),
    ),
  );
  await tester.pump();
  return (store: store, hub: hub, windows: windows, sessions: sessions, agents: agents);
}

Finder _tab(ChatSession session) => find.byKey(Key('tab-open-${session.id}'));
Finder _menuOf(ChatSession session) => find.byKey(Key('tab-menu-${session.id}'));

/// The strip of a window of its own, not the main window's.
Finder get _inWindow => find.descendant(of: find.byType(TabbedWindow), matching: find.byType(HubTabStrip));

void main() {
  testWidgets('opening a session adds a tab and shows it; opening one that is open shows its tab and adds none', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    expect(hub.mainTabs.sessions, [for (final s in sessions) s.id]);
    expect(store.selectedId, sessions[2].id);
    expect(find.text('Session A'), findsOneWidget);
    expect(find.text('Session C'), findsOneWidget);

    store.select(sessions[0].id);
    await tester.pump();
    expect(hub.mainTabs.length, 3, reason: 'its tab was there');
    expect(hub.currentIn(PopoutHub.mainKey), sessions[0].id);
    expect(hub.mainTabs.current, sessions[0].id, reason: 'the strip and the rail agree on the current session');
  });

  testWidgets('a tab is chosen by tapping it, which selects the session as the rail does', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.tap(_tab(sessions[1]));
    await tester.pump();
    expect(store.selectedId, sessions[1].id);
    expect(hub.mainTabs.current, sessions[1].id);
  });

  testWidgets('a lone tab draws no strip; closing the last tab of the main window shows the start screen, with no strip', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester, count: 1);
    ChatSession? start;
    hub.onNewSession = () => start = store.createDraft();
    expect(_tab(sessions[0]), findsNothing);

    final second = ScriptedAgent();
    final other = await tester.runAsync(() => second.start(store)) as ChatSession;
    await tester.pump();
    expect(_tab(other), findsOneWidget);
    expect(find.byKey(Key('tab-close-${other.id}')), findsOneWidget);
    hub.closeTab(PopoutHub.mainKey, other);
    await tester.pump();
    expect(hub.mainTabs.sessions, [sessions[0].id]);
    expect(store.selectedId, sessions[0].id);

    hub.closeTab(PopoutHub.mainKey, sessions[0]);
    await tester.pump();
    expect(hub.mainTabs.isEmpty, isTrue, reason: 'the last tab closes too');
    expect(start, isNotNull);
    expect(store.selectedId, start!.id, reason: 'the window shows the start screen of a new session');
    expect(find.byType(SessionTabStrip), findsNothing);
    expect(hub.mainTabs.contains(start!.id), isFalse);
  });

  testWidgets('the start screen of a new session is no tab: the strip keeps the sessions, shows none as current, and adds nothing', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester, count: 2);
    expect(find.byType(SessionTabStrip), findsOneWidget);
    final draft = store.createDraft();
    await tester.pump();
    expect(store.selectedId, draft.id);
    expect(hub.mainTabs.sessions, [for (final s in sessions) s.id], reason: 'no tab for the start screen');
    expect(hub.currentIn(PopoutHub.mainKey), isNull);
    expect(find.byType(SessionTabStrip), findsOneWidget);
    expect(tester.widget<SessionTabStrip>(find.byType(SessionTabStrip)).sessions, hasLength(2));
    expect(find.text(draft.title), findsNothing, reason: 'no New tab, and so no close button for it');

    await tester.tap(_tab(sessions[0]));
    await tester.pump();
    expect(store.selectedId, sessions[0].id);
    expect(hub.currentIn(PopoutHub.mainKey), sessions[0].id);
  });

  testWidgets('with a start screen over one session, the strip is drawn to go back to it; with no session it is not', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester, count: 1);
    expect(find.byType(SessionTabStrip), findsNothing);
    store.createDraft();
    await tester.pump();
    expect(_tab(sessions[0]), findsOneWidget);
    await tester.tap(find.byKey(Key('tab-close-${sessions[0].id}')));
    await tester.pump();
    expect(find.byType(SessionTabStrip), findsNothing, reason: 'no session is open: the start screen alone');
    expect(hub.mainTabs.isEmpty, isTrue);
  });

  testWidgets('a session moved out of the main window and back keeps its tab; the main window shows the start screen meanwhile', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester, count: 1);
    hub.onNewSession = store.createDraft;
    await tester.runAsync(() => hub.moveToNewWindow(sessions[0]));
    await _settle(tester);
    expect(hub.mainTabs.isEmpty, isTrue);
    expect(store.selected!.isDraft, isTrue, reason: 'the start screen');
    expect(find.byType(SessionTabStrip), findsNothing);
    expect(hub.placeOf(sessions[0]), isNot(PopoutHub.mainKey));

    await tester.runAsync(() => hub.moveBack(sessions[0]));
    await _settle(tester);
    expect(hub.mainTabs.sessions, [sessions[0].id]);
    expect(store.selectedId, sessions[0].id);
    expect(hub.placeOf(sessions[0]), PopoutHub.mainKey);
  });

  testWidgets('closing a tab shows the next one, and leaves the conversation where it is', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    store.select(sessions[1].id);
    await tester.pump();
    await tester.tap(find.byKey(Key('tab-close-${sessions[1].id}')));
    await tester.pump();
    expect(hub.mainTabs.sessions, [sessions[0].id, sessions[2].id]);
    expect(store.selectedId, sessions[2].id, reason: 'the tab that followed is shown');
    expect(store.sessions, contains(sessions[1]), reason: 'only the tab went; the session is in the list still');
    expect(_tab(sessions[1]), findsNothing);

    // Choosing it from the list brings its tab back.
    store.select(sessions[1].id);
    await tester.pump();
    expect(hub.mainTabs.sessions, contains(sessions[1].id));
    expect(hub.mainTabs.current, sessions[1].id);
  });

  testWidgets('a tab is dragged to a new place in the strip', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    final from = tester.getCenter(_tab(sessions[0]));
    final to = tester.getCenter(_tab(sessions[2]));
    final drag = await tester.startGesture(from);
    await tester.pump(const Duration(milliseconds: 100));
    await drag.moveTo(Offset((from.dx + to.dx) / 2, from.dy));
    await tester.pump(const Duration(milliseconds: 100));
    await drag.moveTo(Offset(to.dx + 330, to.dy));
    await tester.pump(const Duration(milliseconds: 400));
    await drag.moveBy(const Offset(10, 0));
    await tester.pump(const Duration(milliseconds: 400));
    await drag.up();
    await tester.pumpAndSettle();
    expect(hub.mainTabs.sessions, [sessions[1].id, sessions[2].id, sessions[0].id]);
    expect(store.selectedId, sessions[2].id, reason: 'moving a tab does not change the one shown');
  });

  testWidgets('Move to new window: the session is a tab of the new window and of no other, and the main window shows a neighbour', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    store.select(sessions[1].id);
    await tester.pump();
    await tester.tap(_menuOf(sessions[1]));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('tab-move-new')));
    await _settle(tester);

    expect(windows.opened, hasLength(1));
    final window = windows.opened.single;
    expect(window.title, 'Session B');
    expect(hub.placeOf(sessions[1]), window.id);
    expect(hub.mainTabs.sessions, [sessions[0].id, sessions[2].id], reason: 'moving it closed it in the main window');
    expect(store.selectedId, sessions[2].id);
    expect(hub.sessionsIn(window.id).map((s) => s.id), [sessions[1].id]);
    expect(find.byType(PopoutChat), findsOneWidget, reason: 'the window draws the conversation of its tab');
    expect(find.byType(PopoutChat).evaluate().length, 1);
    expect(hub.viewsOf(sessions[1]), [window.id]);
  });

  testWidgets('Move to window …: the tab joins another window\'s tabs, and a window left with no tab closes', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.runAsync(() => hub.moveToNewWindow(sessions[0]));
    await tester.runAsync(() => hub.moveToNewWindow(sessions[1]));
    await _settle(tester);
    expect(windows.opened, hasLength(2));
    final (first, second) = (windows.opened[0], windows.opened[1]);

    // The third session's tab, in the main window, offers the two windows.
    await tester.tap(_menuOf(sessions[2]));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('tab-move-to')));
    await tester.pumpAndSettle();
    expect(find.byKey(Key('tab-window-${first.id}')), findsOneWidget);
    expect(find.byKey(Key('tab-window-${second.id}')), findsOneWidget);
    await tester.tap(find.byKey(Key('tab-window-${first.id}')));
    await _settle(tester);
    expect(hub.sessionsIn(first.id).map((s) => s.id), [sessions[0].id, sessions[2].id]);
    expect(hub.currentIn(first.id), sessions[2].id);
    expect(hub.mainTabs.isEmpty, isTrue);
    expect(first.focused, greaterThan(0));

    // Moving the second window's only tab to the first closes the second.
    await tester.runAsync(() => hub.moveToWindow(sessions[1], first.id));
    await _settle(tester);
    expect(second.isClosed, isTrue);
    expect(hub.otherWindows(), hasLength(1));
    expect(hub.sessionsIn(first.id), hasLength(3));
    for (final session in sessions) {
      expect(hub.placeOf(session), first.id, reason: 'each session is a tab of exactly one window');
    }
  });

  testWidgets('Move back: the tab returns to the main window and is shown there, and the window it left closes', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.runAsync(() => hub.moveToNewWindow(sessions[2]));
    await _settle(tester);
    final window = windows.opened.single;
    expect(hub.mainTabs.sessions, [sessions[0].id, sessions[1].id]);

    await tester.runAsync(() => hub.moveToNewWindow(sessions[1]));
    await _settle(tester);
    await tester.runAsync(() => hub.moveToWindow(sessions[1], window.id));
    await _settle(tester);
    expect(hub.sessionsIn(window.id), hasLength(2));

    // The window's own strip, with two tabs, offers it.
    expect(_inWindow, findsOneWidget);
    await tester.tap(find.descendant(of: _inWindow, matching: find.byKey(Key('tab-menu-${sessions[2].id}'))));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('tab-move-back')), findsOneWidget);
    await tester.tap(find.byKey(const Key('tab-move-back')));
    await _settle(tester);

    expect(hub.placeOf(sessions[2]), PopoutHub.mainKey);
    expect(store.selectedId, sessions[2].id);
    expect(hub.mainTabs.current, sessions[2].id);
    expect(hub.sessionsIn(window.id).map((s) => s.id), [sessions[1].id]);

    await tester.runAsync(() => hub.moveBack(sessions[1]));
    await _settle(tester);
    expect(window.isClosed, isTrue, reason: 'a window with no tab closes');
    expect(hub.placeOf(sessions[1]), PopoutHub.mainKey);
  });

  testWidgets('closing a window of its own sends its tabs to the main window', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.runAsync(() => hub.moveToNewWindow(sessions[2]));
    await tester.runAsync(() => hub.moveToWindow(sessions[1], windows.opened.single.id));
    await _settle(tester);
    expect(hub.mainTabs.sessions, [sessions[0].id]);

    windows.opened.single.closeByUser();
    await _settle(tester);
    expect(hub.mainTabs.sessions.toSet(), {for (final s in sessions) s.id});
    expect(hub.otherWindows(), isEmpty);
    expect(sessions[2].chat.clients.map((c) => c.id), isNot(contains('window-0')), reason: 'the window let go of the conversation');
  });

  testWidgets('choosing a session that is a tab of another window brings that window forward and keeps the main window where it was', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.runAsync(() => hub.moveToNewWindow(sessions[0]));
    await _settle(tester);
    store.select(sessions[1].id);
    await tester.pump();
    final window = windows.opened.single;
    final before = window.focused;

    store.select(sessions[0].id);
    await _settle(tester);
    expect(window.focused, greaterThan(before));
    expect(store.selectedId, sessions[1].id, reason: 'the main window did not take the session a second time');
    expect(hub.mainTabs.contains(sessions[0].id), isFalse);
  });

  testWidgets('a session that is closed is no tab in any window', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    await tester.runAsync(() => hub.moveToNewWindow(sessions[0]));
    await _settle(tester);
    final window = windows.opened.single;
    await tester.runAsync(() => store.close(sessions[0]));
    await _settle(tester);
    expect(window.isClosed, isTrue);
    await tester.runAsync(() => store.close(sessions[1]));
    await tester.pump();
    expect(hub.mainTabs.sessions, [sessions[2].id]);
  });

  testWidgets('a tab says what its session is doing: working, and waiting for an answer', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester);
    expect(find.byKey(const Key('tab-state-working')), findsNothing);

    unawaited(store.sendText('go'));
    await _settle(tester);
    expect(find.descendant(of: _tab(sessions[2]), matching: find.byKey(const Key('tab-state-working'))), findsOneWidget);
    expect(find.descendant(of: _tab(sessions[0]), matching: find.byKey(const Key('tab-state-working'))), findsNothing);

    agents[2].ask(5, 'Edit a.txt');
    await _settle(tester);
    expect(find.descendant(of: _tab(sessions[2]), matching: find.byKey(const Key('tab-state-needsYou'))), findsOneWidget);
    expect(find.byTooltip('Needs you'), findsOneWidget);

    // The same marks are in a window of its own.
    await tester.runAsync(() => hub.moveToNewWindow(sessions[2]));
    await tester.runAsync(() => hub.moveToWindow(sessions[0], windows.opened.single.id));
    await _settle(tester);
    expect(find.descendant(of: _inWindow, matching: find.byKey(const Key('tab-state-needsYou'))), findsOneWidget);
  });

  testWidgets('on a narrow screen the strip scrolls, and only the tab shown carries its buttons', (tester) async {
    final (:store, :hub, :windows, :sessions, :agents) = await _scene(tester, count: 6, narrow: true);
    expect(tester.takeException(), isNull, reason: 'six tabs do not overflow a phone');
    expect(find.byKey(Key('tab-menu-${sessions[5].id}')), findsOneWidget);
    expect(find.byKey(Key('tab-menu-${sessions[4].id}')), findsNothing);
    expect(_tab(sessions[0]).evaluate(), isEmpty, reason: 'the first tabs are scrolled out; the current one is in view');

    await tester.tap(_tab(sessions[4]));
    await tester.pumpAndSettle();
    expect(store.selectedId, sessions[4].id);
    expect(find.byKey(Key('tab-menu-${sessions[4].id}')), findsOneWidget);
  });

  testWidgets('a tab in the window of a stand-in without windows has no move entries', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    addTearDown(store.dispose);
    final hub = PopoutHub(store, FakeWindows(available: false));
    addTearDown(hub.dispose);
    final sessions = <ChatSession>[];
    for (var i = 0; i < 2; i++) {
      sessions.add(await tester.runAsync(() => ScriptedAgent().start(store)) as ChatSession);
    }
    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.light),
        home: Scaffold(body: HubTabStrip(hub: hub, windowKey: PopoutHub.mainKey)),
      ),
    );
    expect(_tab(sessions[0]), findsOneWidget, reason: 'tabs work where windows do not');
    expect(_menuOf(sessions[0]), findsOneWidget, reason: 'close is in the menu');
    await tester.tap(_menuOf(sessions[0]));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('tab-move-new')), findsNothing);
    expect(find.byKey(const Key('tab-close-item')), findsOneWidget);
  });
}
