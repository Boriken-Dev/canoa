// The welcome page on its own: the offer to import earlier sessions reads nothing until the person says yes.
import 'dart:io' show Directory;

import 'package:canoa/sources/sources.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/welcome/welcome_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
const _codex = AgentSource.hosted('Codex', harness: 'codex', local: true);

Future<void> _show(WidgetTester tester, WelcomeView view, {double width = 1000}) async {
  await tester.binding.setSurfaceSize(Size(width, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false, theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: view),
  ));
  await tester.pump();
}

WelcomeView _view({HistoryImport? historyImport, Map<AgentSource, SourceCheck>? checks, List<AgentSource>? sources}) => WelcomeView(
  sources: sources ?? [_claude, _codex],
  checks: checks ?? {_claude: const SourceCheck(), _codex: const SourceCheck()},
  onNewSession: () {},
  historyImport: historyImport,
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/welcome_view_test.dart')));

  for (final (label, width) in [('phone', 390.0), ('desktop', 1000.0)]) {
    testWidgets('the page with the import offer draws at $label width', (tester) async {
      await _show(
        tester,
        _view(historyImport: HistoryImport(from: const ['Claude', 'Codex'], run: () async => 0)),
        width: width,
      );
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/welcome_offer_$label.png'));
    });
  }

  testWidgets('the import offer reads nothing before a yes, then once, and says how many', (tester) async {
    var reads = 0;
    await _show(tester, _view(historyImport: HistoryImport(from: const ['Claude', 'Codex'], run: () async => ++reads + 2)));
    expect(find.text('Import earlier sessions'), findsOneWidget);
    expect(find.textContaining('Claude and Codex'), findsOneWidget);
    await tester.pump(const Duration(seconds: 2));
    expect(reads, 0, reason: 'showing the offer, even for a while, reads nothing');
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(reads, 1);
    expect(find.text('Imported 3 sessions.'), findsOneWidget);
    expect(find.byKey(const Key('welcome-import')), findsNothing);
  });

  testWidgets('a failed import says why and can be tried again', (tester) async {
    var tries = 0;
    await _show(tester, _view(historyImport: HistoryImport(from: const ['Claude'], run: () async {
      if (++tries == 1) throw StateError('no history folder');
      return 1;
    })));
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Could not import: Bad state: no history folder'), findsOneWidget);
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(find.text('Imported 1 session.'), findsOneWidget);
  });

  testWidgets('with no import to offer the page makes no offer', (tester) async {
    await _show(tester, _view());
    expect(find.text('Import earlier sessions'), findsNothing);
    expect(find.byKey(const Key('welcome-import')), findsNothing);
  });

  testWidgets('a source still being looked at says so, and a missing one says what it needs', (tester) async {
    await _show(tester, _view(checks: {_codex: const SourceCheck(missing: ['codex'])}));
    expect(find.descendant(of: find.byKey(const Key('welcome-source Claude')), matching: find.text('Looking…')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('welcome-source Codex')), matching: find.textContaining('codex was not found')), findsOneWidget);
  });

  testWidgets('with no source at all the page says so', (tester) async {
    await _show(tester, _view(sources: const [], checks: const {}), width: 360);
    expect(find.byKey(const Key('welcome-no-sources')), findsOneWidget);
    expect(find.textContaining('verified ACP peer'), findsNothing);
  });
}
