// The app's shell: Conversations, Loadouts and Settings are sections of one selector, a message being written survives a visit to
// another section, Ctrl+B hides only the conversations index, and the layout comes back after a restart.
import 'dart:io' show Directory;

import 'package:canoa/chat/new_session_view.dart';
import 'package:canoa/rail/session_rail.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/session_setup.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

Finder get _composer => find.byWidgetPredicate((widget) => widget is TextField && (widget.decoration?.hintText ?? '').startsWith('Message'));

Future<void> _ctrl(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await frames(tester);
}

/// A store with a few conversations in different states, the first of them shown.
SessionStore _busy() {
  final store = SessionStore();
  final now = DateTime.now();
  ChatSession open(String title, String cwd, {Duration ago = const Duration(minutes: 5)}) {
    final session = store.addProviderSession(title: title, agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-$title', cwd: cwd)
      ..status = SessionStatus.connected;
    session.chat.addMessage(MessageRole.user, 'Why does the rail test fail on CI?', at: now.subtract(ago));
    session.chat.addMessage(MessageRole.agent, 'It reads the clock. I pinned it in the fixture, so the age no longer drifts.', at: now.subtract(ago));
    return session;
  }
  open('Fix the failing test', '/home/ana/devel/canoa', ago: const Duration(minutes: 12));
  open('Release notes', '/home/ana/devel/canoa', ago: const Duration(hours: 20)).pinned = true;
  open('Review the diff', '/home/ana/devel/coqui-acp', ago: const Duration(minutes: 1)).asking();
  store.select(store.sessions.last.id);
  return store;
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/shell_app_test.dart')));

  testWidgets('the selector offers Conversations, Loadouts and Settings, and Settings is no longer a page over the app', (tester) async {
    await pumpApp(tester, SessionStore());
    for (final name in const ['Conversations', 'Loadouts', 'Canoa settings']) {
      expect(find.byTooltip(name), findsOneWidget);
    }
    expect(find.text('Settings'), findsNothing);
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    expect(find.byKey(const Key('settings-search')), findsOneWidget);
    expect(find.byType(SectionRail), findsOneWidget, reason: 'the selector stays beside it');
    expect(find.byType(SessionRail), findsNothing, reason: 'Settings has no conversations index');
    await tester.tap(find.byTooltip('Loadouts'));
    await frames(tester);
    expect(find.byKey(const Key('loadout-tabs')), findsOneWidget);
    await tester.tap(find.byTooltip('Conversations'));
    await frames(tester);
    expect(find.byType(SessionRail), findsOneWidget);
  });

  testWidgets('a message being written is still there after a visit to Settings and to Loadouts', (tester) async {
    await pumpApp(tester, SessionStore());
    await newSession(tester);
    await tester.enterText(_composer, 'fix the build');
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    await tester.tap(find.byTooltip('Loadouts'));
    await frames(tester);
    await tester.tap(find.byTooltip('Conversations'));
    await frames(tester);
    expect(tester.widget<TextField>(_composer).controller?.text, 'fix the build');
  });

  testWidgets('Ctrl+B hides the conversations index and gives the room to the conversation, and brings it back', (tester) async {
    await pumpApp(tester, SessionStore());
    await newSession(tester);
    final width = tester.getSize(find.byType(NewSessionView)).width;
    expect(find.byType(SessionRail), findsOneWidget);
    await _ctrl(tester, LogicalKeyboardKey.keyB);
    expect(find.byType(SessionRail), findsNothing);
    expect(find.byType(SectionRail), findsOneWidget);
    expect(tester.getSize(find.byType(NewSessionView)).width, greaterThan(width));
    await _ctrl(tester, LogicalKeyboardKey.keyB);
    expect(find.byType(SessionRail), findsOneWidget);
  });

  testWidgets('the section, the chosen conversation and the width come back in a new run', (tester) async {
    final backend = MemoryLayoutBackend();
    final first = ShellLayout(backend: backend);
    final store = SessionStore();
    final one = store.createDraft(title: 'One');
    store.createDraft(title: 'Two');
    store.select(one.id);
    await pumpApp(tester, store, layout: first);
    first.setIndexWidth(ShellLayout.conversations, 330, keep: true);
    await tester.pump();
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    await first.flush();

    await tester.pumpWidget(const SizedBox());
    final second = ShellLayout(backend: backend);
    await second.load();
    final again = SessionStore();
    final one2 = again.createDraft(title: 'One');
    again.createDraft(title: 'Two');
    again.select(again.sessions.first.id);
    await pumpApp(tester, again, layout: second);
    expect(second.section, 'settings');
    expect(find.byKey(const Key('settings-search')), findsOneWidget);
    expect(second.indexWidth(ShellLayout.conversations, 260), 330);
    expect(again.selectedId, one2.id, reason: 'the conversation shown last time');
  });

  testWidgets('a narrow window has the selector and the index in the drawer', (tester) async {
    await pumpApp(tester, SessionStore(), size: const Size(500, 800));
    expect(find.byType(SectionRail), findsNothing);
    await tester.tap(find.byTooltip('Sessions'));
    await frames(tester);
    expect(find.byType(SectionRail), findsOneWidget);
    expect(find.byType(SessionRail), findsOneWidget);
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    expect(find.byKey(const Key('settings-search')), findsOneWidget);
    expect(find.byType(SectionRail), findsNothing, reason: 'the drawer closed');
    await tester.tap(find.byKey(const ValueKey('open-navigation')));
    await frames(tester);
    expect(find.byType(SectionRail), findsOneWidget);
  });

  group('goldens', () {
    testWidgets('the shell at desktop width: the selector, the conversations index and the conversation', (tester) async {
      final store = _busy();
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: const Size(1280, 800));
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/shell_desktop.png'));
    });

    testWidgets('the shell at desktop width with the index hidden', (tester) async {
      final store = _busy();
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: const Size(1280, 800));
      await _ctrl(tester, LogicalKeyboardKey.keyB);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/shell_index_hidden_desktop.png'));
    });

    testWidgets('Settings as a section at desktop width', (tester) async {
      final store = _busy();
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: const Size(1280, 800));
      await tester.tap(find.byTooltip('Canoa settings'));
      await frames(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/shell_settings_desktop.png'));
    });

    testWidgets('the shell at phone width with the drawer open: the selector beside the index', (tester) async {
      final store = _busy();
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: const Size(390, 844));
      await tester.tap(find.byTooltip('Sessions'));
      await frames(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/shell_phone.png'));
    });
  });
}
