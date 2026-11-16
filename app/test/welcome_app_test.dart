// The first screen: the window lands on the page of choices for a new session, which exists only as that page until Send. With no
// agent installed it says so and links to the Store, which is where an agent is installed.
import 'dart:io' show Directory;

import 'package:canoa/chat/new_session_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/rail/session_rail.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

Finder _row(String label) => find.byKey(Key('welcome-source $label'));

/// Every program the host's own sources need, so that no agent can start.
const _noAgents = {'claude', 'codex', 'agy', 'grok', 'pi'};

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/welcome_app_test.dart')));

  testWidgets('launch lands on the page of choices for a new session, with nothing in the side list and nothing started', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = OptionsAgent();
    await pumpApp(tester, store, linked: agent.openLink);
    expect(find.byType(NewSessionView), findsOneWidget);
    expect(find.byKey(const Key('welcome')), findsNothing);
    expect(store.selected?.isDraft, isTrue);
    expect(store.sessions, hasLength(1));
    expect(store.listed, isEmpty, reason: 'the page is not a session until Send');
    expect(find.descendant(of: find.byType(SessionRail), matching: find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key! as ValueKey<String>).value.startsWith('row '))), findsNothing);
    expect(agent.starts, isEmpty);
    expect(find.textContaining('Preview'), findsNothing);
    await finish(tester);
  });

  testWidgets('choosing the agent, the place and the folder makes no session and starts nothing', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = OptionsAgent();
    await pumpApp(tester, store, linked: agent.openLink);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    expect(store.sessions, hasLength(1));
    expect(store.listed, isEmpty);
    expect(agent.starts, isEmpty);
    await finish(tester);
  });

  testWidgets('New session beside the page of choices shows the same page, not a second one', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store, linked: OptionsAgent().openLink);
    await newSession(tester);
    await newSession(tester);
    expect(store.sessions, hasLength(1));
    expect(store.listed, isEmpty);
    await finish(tester);
  });

  testWidgets('Send makes the session, its row appears in the side list, and the view becomes the conversation', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = OptionsAgent();
    await pumpApp(tester, store, linked: agent.openLink);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    expect(store.listed, isEmpty);
    await send(tester, 'Say hello');
    await until(tester, () => store.selected != null && !store.selected!.isDraft, 'the session to start');
    expect(agent.starts, hasLength(1));
    expect(store.listed, hasLength(1));
    expect(find.byKey(ValueKey('row ${store.selected!.id}')), findsOneWidget);
    expect(find.byType(NewSessionView), findsNothing);
    expect(find.text('Say hello'), findsWidgets);
    await finish(tester);
  });

  testWidgets('closing the last session lands on the page of choices again', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store, linked: OptionsAgent().openLink);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await send(tester, 'Say hello');
    await until(tester, () => store.selected != null && !store.selected!.isDraft, 'the session to start');
    await closeSession(tester, store, store.selected!);
    await frames(tester);
    expect(store.selected?.isDraft, isTrue);
    expect(store.listed, isEmpty);
    expect(find.byType(NewSessionView), findsOneWidget);
    await finish(tester);
  });

  testWidgets('with no agent installed the page says so and links to the Store, which it is not', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store, missing: _noAgents, config: const CanoaConfig(), linked: OptionsAgent().openLink);
    expect(find.byKey(const Key('welcome')), findsOneWidget);
    expect(find.byKey(const Key('welcome-title')), findsOneWidget);
    expect(find.byType(NewSessionView), findsNothing);
    expect(find.byKey(const Key('welcome-new-session')), findsNothing);
    expect(find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key! as ValueKey<String>).value.startsWith('program-install')), findsNothing);
    expect(_row('Claude'), findsOneWidget);
    await tester.ensureVisible(find.byKey(const Key('welcome-store')));
    await tester.tap(find.byKey(const Key('welcome-store')));
    await frames(tester);
    expect(find.byKey(const Key('store-tabs')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a source whose program is missing says what it needs, while another is ready the page of choices shows', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store, missing: {'codex'}, linked: OptionsAgent().openLink);
    expect(find.byType(NewSessionView), findsOneWidget);
    expect(find.byKey(const Key('welcome')), findsNothing);
    await finish(tester);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the no-agent page draws at $label width with nothing overflowing', (tester) async {
      final store = SessionStore();
      addTearDown(store.dispose);
      await pumpApp(tester, store, missing: _noAgents, config: const CanoaConfig(), size: size, linked: OptionsAgent().openLink);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/welcome_$label.png'));
    });

    testWidgets('the page of choices a launch lands on draws at $label width', (tester) async {
      final store = SessionStore();
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: size, linked: OptionsAgent().openLink);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/landing_$label.png'));
    });
  }
}
