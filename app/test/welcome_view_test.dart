// The welcome page on its own: the offer to import earlier sessions reads nothing until the person says yes.
import 'dart:io' show Directory;

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/history/session_import.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/welcome/welcome_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
const _codex = AgentSource.hosted('Codex', harness: 'codex', local: true);

const _a = ImportChoice(source: 'Claude', entry: 'Claude', harness: 'claude', id: 'a1', title: 'Fix the build', cwd: '/work/app');
const _b = ImportChoice(source: 'Claude', entry: 'Claude', harness: 'claude', id: 'b2', title: 'Rename the module');
const _bAlready = ImportChoice(source: 'Claude', entry: 'Claude', harness: 'claude', id: 'b2', title: 'Rename the module', imported: true);
const _c = ImportChoice(source: 'Codex', entry: 'Codex', harness: 'codex', id: 'c3', title: 'Add an endpoint', cwd: '/work/api');

HistoryImport _offer({
  Future<List<ImportChoice>> Function()? list,
  Future<ImportOutcome> Function(List<ImportChoice>)? bring,
}) => HistoryImport(
  from: const ['Claude', 'Codex'],
  list: list ?? () async => const [],
  bring: bring ?? (chosen) async => ImportOutcome(imported: chosen.length),
);

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
      await _show(tester, _view(historyImport: _offer()), width: width);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/welcome_offer_$label.png'));
    });
  }

  testWidgets('the import offer reads nothing before a yes, then lists once and brings in what is ticked', (tester) async {
    var looks = 0;
    List<ImportChoice>? brought;
    await _show(tester, _view(historyImport: _offer(
      list: () async {
        looks++;
        return [_a, _b, _c];
      },
      bring: (chosen) async {
        brought = chosen;
        return ImportOutcome(imported: chosen.length, notes: const ['Fix the build: 2 lines of its file could not be read and did not carry.']);
      },
    )));
    expect(find.text('Import earlier sessions'), findsOneWidget);
    expect(find.textContaining('Claude and Codex'), findsOneWidget);
    await tester.pump(const Duration(seconds: 2));
    expect(looks, 0, reason: 'showing the offer, even for a while, reads nothing');
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(looks, 1);
    expect(brought, isNull, reason: 'listing brings nothing in');
    expect(find.byKey(const Key('welcome-import-source Claude')), findsOneWidget);
    expect(find.text('Claude: 2 sessions'), findsOneWidget);
    expect(find.text('Codex: 1 sessions'), findsOneWidget);
    expect(find.text('Fix the build'), findsOneWidget);
    expect(find.text('Add an endpoint'), findsOneWidget);
    expect(find.text('Import 3'), findsOneWidget);
    await tester.tap(find.byKey(Key('welcome-import-pick ${_b.id}')));
    await tester.pump();
    expect(find.text('Import 2'), findsOneWidget);
    await tester.tap(find.byKey(const Key('welcome-import-run')));
    await tester.pump();
    await tester.pump();
    expect(looks, 1);
    expect(brought, [_a, _c]);
    expect(find.text('Imported 2 sessions.'), findsOneWidget);
    expect(find.textContaining('2 lines of its file could not be read'), findsOneWidget);
    expect(find.byKey(const Key('welcome-import')), findsNothing);
  });

  testWidgets('a session already imported is shown as such and is not brought in again', (tester) async {
    List<ImportChoice>? brought;
    await _show(tester, _view(historyImport: _offer(
      list: () async => [_a, _bAlready],
      bring: (chosen) async {
        brought = chosen;
        return ImportOutcome(imported: chosen.length);
      },
    )));
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Already imported'), findsOneWidget);
    expect(find.text('Import 1'), findsOneWidget);
    await tester.tap(find.byKey(const Key('welcome-import-run')));
    await tester.pump();
    await tester.pump();
    expect(brought, [_a]);
    expect(find.text('Imported 1 session.'), findsOneWidget);
  });

  testWidgets('nothing found says so, and a failed look says why and can be tried again', (tester) async {
    var tries = 0;
    await _show(tester, _view(historyImport: _offer(list: () async {
      if (++tries == 1) throw StateError('no history folder');
      return const [];
    })));
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Could not look: Bad state: no history folder'), findsOneWidget);
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    expect(find.byKey(const Key('welcome-import-none')), findsOneWidget);
  });

  testWidgets('an import the host refuses says why and keeps the choice to try again', (tester) async {
    var tries = 0;
    await _show(tester, _view(historyImport: _offer(
      list: () async => [_a],
      bring: (chosen) async {
        if (++tries == 1) throw const ControlFailed(-32002, 'this host keeps no session histories');
        return ImportOutcome(imported: 1);
      },
    )));
    await tester.tap(find.byKey(const Key('welcome-import')));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('welcome-import-run')));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Could not import: this host keeps no session histories'), findsOneWidget);
    await tester.tap(find.byKey(const Key('welcome-import-run')));
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
