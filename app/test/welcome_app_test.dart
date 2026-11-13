// The first screen: with no session yet the window shows each source with its readiness, not two offline preview sessions.
import 'dart:io' show Directory;

import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;
import 'fixtures/store_link.dart';

Finder _row(String label) => find.byKey(Key('welcome-source $label'));

FakeStoreHost _storeHost({List<Map<String, Object?>> copies = const []}) => FakeStoreHost([
  FakeStore(
    'claude_like_store',
    title: 'Program One',
    serves: [
      {'provider': 'claude', 'when': <String, Object?>{}},
    ],
    copies: copies,
    plan: {'install': planOf('install', '1.0.0', bytes: 1024 * 1024)},
  ),
]);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/welcome_app_test.dart')));

  testWidgets('a first run with a ready source lists it, with no preview session and no peer to connect', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store);
    expect(store.sessions, isEmpty);
    expect(find.byKey(const Key('welcome')), findsOneWidget);
    expect(_row('Claude'), findsOneWidget);
    expect(find.descendant(of: _row('Claude'), matching: find.text('Ready')), findsOneWidget);
    expect(find.descendant(of: _row('Codex'), matching: find.text('Ready')), findsOneWidget);
    expect(find.text('Getting started'), findsNothing);
    expect(find.text('Another conversation'), findsNothing);
    expect(find.textContaining('Preview'), findsNothing);
    expect(find.textContaining('verified ACP peer'), findsNothing);
    expect(find.text('Create a conversation to begin.'), findsNothing);
  });

  testWidgets('a source whose program is missing says what it needs', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store, missing: {'codex'});
    expect(find.descendant(of: _row('Claude'), matching: find.text('Ready')), findsOneWidget);
    expect(find.descendant(of: _row('Codex'), matching: find.textContaining('needs codex on PATH')), findsOneWidget);
    expect(find.descendant(of: _row('Codex'), matching: find.text('Ready')), findsNothing);
  });

  testWidgets('a source the Store serves shows its version, or Install when the program is not found', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final host = _storeHost(copies: [copyOf('1.0.0')]);
    await pumpApp(tester, store, linked: (name) => name == '@control' ? host.open() : throw StateError('nothing starts: $name'));
    expect(find.descendant(of: _row('Claude'), matching: find.textContaining('1.0.0')), findsOneWidget);
    expect(find.byKey(const Key('program-install-claude_like_store')), findsNothing);
  });

  testWidgets('Install from the welcome page asks for the plan first, and makes the source ready', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final host = _storeHost();
    host.afterApply = (store, plan) => store.copies = [copyOf('1.0.0')];
    await pumpApp(
      tester, store, missing: {'claude'}, size: const Size(1280, 1000),
      linked: (name) => name == '@control' ? host.open() : throw StateError('nothing starts: $name'),
    );
    expect(find.descendant(of: _row('Claude'), matching: find.textContaining('Not installed')), findsOneWidget);
    await tester.tap(find.byKey(const Key('program-install-claude_like_store')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.tap(find.byKey(const Key('program-close')));
    await tester.pumpAndSettle();
    expect(find.descendant(of: _row('Claude'), matching: find.textContaining('1.0.0')), findsOneWidget);
  });

  testWidgets('New session on the welcome page opens a draft, and the page goes', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await pumpApp(tester, store);
    await tester.tap(find.byKey(const Key('welcome-new-session')));
    await frames(tester);
    expect(store.selected?.isDraft, isTrue);
    expect(find.byKey(const Key('welcome')), findsNothing);
  });

  testWidgets('with a session open there is no welcome page', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    store.createDraft(title: 'Work');
    await pumpApp(tester, store);
    expect(find.byKey(const Key('welcome')), findsNothing);
  });

  // One source found by the Store with its version, one missing, the rest the host's own and ready.
  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the welcome page draws at $label width with nothing overflowing', (tester) async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final host = _storeHost(copies: [copyOf('1.0.0')]);
      await pumpApp(
        tester, store, missing: {'codex'}, size: size,
        linked: (name) => name == '@control' ? host.open() : throw StateError('nothing starts: $name'),
      );
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/welcome_$label.png'));
    });
  }
}
