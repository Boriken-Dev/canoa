// The Store's readiness reaching the app: Settings' sources and a new session's source menu say what the host's Store extension
// reports (the version found, an update, Install), the app asks the host nothing but the copies when it starts, and an install
// from Settings changes the source's readiness at once.
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_link.dart';

FakeStoreHost _host({List<Map<String, Object?>> copies = const [], String? latest}) => FakeStoreHost([
  FakeStore(
    'claude_like_store',
    title: 'Program One',
    serves: [
      {'provider': 'claude', 'when': <String, Object?>{}},
    ],
    copies: copies,
    latest: latest,
    plan: {'install': planOf('install', '1.0.0', bytes: 1024 * 1024), 'update': planOf('update', '1.1.0', from: '1.0.0')},
  ),
]);

Future<FakeStoreHost> _app(WidgetTester tester, FakeStoreHost host, {Set<String> missing = const {}}) async {
  await pumpApp(
    tester,
    SessionStore()..createDraft(title: 'Work'), // a session open, so the welcome page does not repeat what Settings shows
    missing: missing,
    size: const Size(1280, 1400),
    linked: (name) => name == '@control' ? host.open() : throw StateError('nothing starts in this test: $name'),
  );
  return host;
}

Future<void> _openSources(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Canoa settings'));
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(const Key('settings-nav-sources')));
  await tester.pumpAndSettle();
  // A source's version, readiness and buttons are in its detail.
  for (final name in const ['Claude', 'Codex']) {
    final row = find.byKey(Key('source-row $name'));
    if (row.evaluate().isEmpty) continue;
    await tester.tap(row);
    await tester.pumpAndSettle();
  }
}

void main() {
  testWidgets('starting the app asks the host only for the copies: no publisher, no plan, no install', (tester) async {
    final host = await _app(tester, _host(latest: '1.0.0'));
    expect(host.calls.toSet().difference({'store/list', 'store/report claude_like_store'}), isEmpty, reason: '${host.calls}');
    expect((host.params['store/report'] ?? []).every((each) => !each.containsKey('probe')), isTrue);
  });

  testWidgets('Settings shows the version the extension found and that an update exists, and Update asks first', (tester) async {
    final host = await _app(tester, _host(copies: [copyOf('1.0.0')], latest: '1.1.0'));
    await _openSources(tester);
    final claude = find.byKey(const Key('source-detail Claude'));
    expect(find.descendant(of: claude, matching: find.textContaining('Version 1.0.0 · installed by Canoa')), findsOneWidget);
    expect(find.descendant(of: claude, matching: find.textContaining('Update available: 1.1.0')), findsOneWidget);
    expect(find.descendant(of: claude, matching: find.byKey(const Key('program-update-claude_like_store'))), findsOneWidget);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
    await tester.tap(find.byKey(const Key('program-update-claude_like_store')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('plan-version'))).data, 'Version 1.0.0 to 1.1.0');
  });

  testWidgets('a program the extension does not find shows Not installed in Settings, and Install makes the source ready',
      (tester) async {
    final host = await _app(tester, _host(latest: '1.0.0'), missing: {'claude'});
    host.afterApply = (store, plan) => store.copies = [copyOf('1.0.0')];
    await _openSources(tester);
    expect(find.text('Program One is not installed. Install it in Settings, under Sources, or in the Store.'), findsNothing,
        reason: 'Settings shows the state and the button, not the sentence');
    final claude = find.byKey(const Key('source-detail Claude'));
    expect(find.descendant(of: claude, matching: find.textContaining('Not installed')), findsOneWidget);
    await tester.tap(find.byKey(const Key('program-install-claude_like_store')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(find.byKey(const Key('program-done')), findsOneWidget);
    await tester.tap(find.byKey(const Key('program-close')));
    await tester.pumpAndSettle();
    expect(find.descendant(of: claude, matching: find.textContaining('Version 1.0.0 · installed by Canoa')), findsOneWidget);
    expect(find.byKey(const Key('program-install-claude_like_store')), findsNothing);
  });

  testWidgets('a source no extension serves keeps the host\'s own list of what it is missing', (tester) async {
    await _app(tester, _host(copies: [copyOf('1.0.0')]), missing: {'codex'});
    await _openSources(tester);
    expect(find.text('Codex needs codex on PATH: codex was not found. Install what is missing, or start Canoa where PATH finds it.'),
        findsOneWidget);
  });

  testWidgets('a host whose control channel cannot be opened is asked once, not again for ever', (tester) async {
    var opened = 0;
    await pumpApp(
      tester,
      SessionStore(),
      size: const Size(1280, 1400),
      linked: (name) {
        if (name == '@control') opened++;
        throw StateError('no control channel in this test');
      },
    );
    await _openSources(tester);
    final after = opened;
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(opened, after, reason: 'nothing asks again by itself');
    expect(opened, lessThan(10));
  });
}
