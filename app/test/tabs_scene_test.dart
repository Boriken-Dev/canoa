// The tab strip in the whole app: sessions chosen in the list become tabs of the conversation area, the list and the strip agree on
// the one shown, and the strip draws at desktop and phone width (goldens; with CANOA_SAMPLES_DIR set they are written with real fonts
// for `ui/samples/canoa/`).
import 'dart:async';

import 'package:canoa/session_store.dart';
import 'package:canoa/tabs/session_tab_strip.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart' show pumpApp;
import 'fixtures/scripted_agent.dart';

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 80)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

typedef _Scene = (SessionStore, List<ChatSession>, List<ScriptedAgent>);

Future<_Scene> _scene(WidgetTester tester, Size size) async {
  final store = SessionStore();
  addTearDown(store.dispose);
  final sessions = <ChatSession>[];
  final agents = <ScriptedAgent>[];
  for (final title in ['Fix calc', 'Write the release notes', 'Port the parser']) {
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    store.rename(session, title);
    sessions.add(session);
    agents.add(agent);
  }
  await pumpApp(tester, store, size: size);
  await _settle(tester);
  // Only the session shown when the app opened is a tab; choosing the others opens theirs.
  for (final session in sessions) {
    store.select(session.id);
    await tester.pump();
  }
  await _settle(tester);
  return (store, sessions, agents);
}


void main() {
  testWidgets('the sessions that were chosen are tabs of the conversation area, and a tab and a row choose the same session', (tester) async {
    final (store, sessions, _) = await _scene(tester, const Size(1280, 800));
    final strip = tester.widget<SessionTabStrip>(find.byType(SessionTabStrip));
    expect(strip.sessions.map((s) => s.id), [sessions[2].id, sessions[0].id, sessions[1].id], reason: 'the one shown when the app opened, then each in the order it was chosen');
    expect(strip.current, sessions[2].id);

    await tester.tap(find.byKey(Key('tab-open-${sessions[0].id}')));
    await _settle(tester);
    expect(store.selectedId, sessions[0].id, reason: 'a tab selects the session as the list does');

    await tester.tap(find.byKey(ValueKey('row ${sessions[1].id}')));
    await _settle(tester);
    expect(store.selectedId, sessions[1].id);
    expect(tester.widget<SessionTabStrip>(find.byType(SessionTabStrip)).current, sessions[1].id, reason: 'the strip follows the list');
    expect(tester.widget<SessionTabStrip>(find.byType(SessionTabStrip)).sessions, hasLength(3), reason: 'choosing a session that has its tab adds none');
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the strip draws at $label width, with a working session and one that waits for an answer', (tester) async {
      final (store, sessions, agents) = await _scene(tester, size);
      store.select(sessions[0].id);
      unawaited(store.sendText('Fix addNumbers'));
      await _settle(tester);
      store.select(sessions[1].id);
      unawaited(store.sendText('Draft the notes'));
      agents[1].ask(5, 'Edit notes.md');
      await _settle(tester);
      store.select(sessions[2].id);
      await _settle(tester);
      expect(tester.takeException(), isNull);
      expect(find.byKey(const Key('tab-state-working')), findsWidgets);
      expect(find.byKey(const Key('tab-state-needsYou')), findsWidgets);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/tabs_$label.png'));
    });
  }
}
