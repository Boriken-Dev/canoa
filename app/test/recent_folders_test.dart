// The new-session view offers the folders sessions worked in, newest first, one click each; the ledger is the only source,
// and a folder that is gone is left out.
import 'dart:io' show Directory;

import 'package:canoa/chat/new_session_view.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A ledger that has worked in [folders], the first one longest ago.
SessionLedger _ledgerOf(List<String> folders) {
  var minute = 0;
  final ledger = SessionLedger(MemoryLedgerBackend(), clock: () => DateTime.utc(2026, 11, 1, 9, minute++));
  for (final (index, folder) in folders.indexed) {
    ledger.record(peerId: 'local:Scripted', sessionId: 's$index', entry: 'scripted-agent', place: SessionPlace.local, cwd: folder);
  }
  return ledger;
}

Finder _row(String folder) => find.byKey(ValueKey('recent-folder $folder'));

List<String> _shown(WidgetTester tester) => [
  for (final element in find.byWidgetPredicate((widget) => widget.key is ValueKey<String> &&
      (widget.key! as ValueKey<String>).value.startsWith('recent-folder ')).evaluate())
    (element.widget.key! as ValueKey<String>).value.substring('recent-folder '.length),
];

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/recent_folders_test.dart')));

  testWidgets('recent folders are listed newest first, with no more than the view shows', (tester) async {
    final folders = ['/home/ana/a', '/home/ana/b', '/home/ana/c', '/home/ana/d', '/home/ana/e', '/home/ana/f', '/home/ana/g'];
    final store = SessionStore(ledger: _ledgerOf(folders));
    addTearDown(store.dispose);
    await pumpApp(tester, store);
    await newSession(tester);
    expect(_shown(tester), folders.reversed.take(NewSessionView.recentShown).toList());
  });

  testWidgets('a click puts the new session in that folder and keeps its source', (tester) async {
    final store = SessionStore(ledger: _ledgerOf(['/home/ana/old', '/home/ana/new']));
    addTearDown(store.dispose);
    await pumpApp(tester, store);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    final session = store.selected!;
    expect(session.cwd, isNull);
    await tester.tap(_row('/home/ana/old'));
    await tester.pumpAndSettle();
    expect(store.selected, session, reason: 'the same new session, not another');
    expect(session.cwd, '/home/ana/old');
    expect(session.source?.label, 'Scripted');
    expect(session.status, SessionStatus.draft, reason: 'nothing starts before the first message');
  });

  testWidgets('a folder that no longer exists is not listed', (tester) async {
    final store = SessionStore(ledger: _ledgerOf(['/home/ana/kept', '/home/ana/deleted']));
    addTearDown(store.dispose);
    await pumpApp(tester, store, folderExists: (folder) => folder != '/home/ana/deleted');
    await newSession(tester);
    expect(_shown(tester), ['/home/ana/kept']);
  });

  testWidgets('Open a folder opens the dialog and Sources opens Settings on the sources', (tester) async {
    final store = SessionStore(ledger: _ledgerOf(['/home/ana/a']));
    addTearDown(store.dispose);
    var asked = 0;
    await pumpApp(tester, store, pickFolder: () async {
      asked++;
      return '/home/ana/picked';
    });
    await newSession(tester);
    await tester.tap(find.byKey(const Key('quick-choose-folder')));
    await tester.pumpAndSettle();
    expect(asked, 1);
    expect(store.selected!.cwd, '/home/ana/picked');
    await tester.tap(find.byKey(const Key('quick-sources')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('settings-sources')), findsWidgets);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the new session draws recent folders at $label width with nothing overflowing', (tester) async {
      final store = SessionStore(ledger: _ledgerOf(['/home/ana/notes', '/home/ana/projects/canoa', '/home/ana/projects/a-folder-with-a-rather-long-name-for-a-project']));
      addTearDown(store.dispose);
      await pumpApp(tester, store, size: size);
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await tester.tap(_row('/home/ana/projects/canoa'));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/recent_folders_$label.png'));
    });
  }
}
