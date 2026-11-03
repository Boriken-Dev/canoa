// The session ledger in a real browser's IndexedDB, as the web build keeps it.
//
//   flutter test --platform chrome test/ledger_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'package:canoa/ledger/ledger_backend.dart';
import 'package:canoa/ledger/ledger_backend_web.dart' show IndexedDbLedgerBackend;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the web build keeps the list in IndexedDB', () {
    expect(platformLedgerBackend(configPath: '/ignored/Canoa.cfg'), isA<IndexedDbLedgerBackend>());
  });

  test('a ledger written by one page is read by the next, and nothing written reads as none', () async {
    final database = 'canoa-sessions-test-${DateTime.now().microsecondsSinceEpoch}';
    expect(await IndexedDbLedgerBackend(database: database).read(), isNull);

    final first = SessionLedger(IndexedDbLedgerBackend(database: database), settings: const LedgerSettings(keepText: true));
    first.record(peerId: 'local:Pi', sessionId: 's1', entry: 'Pi', place: SessionPlace.local, reportedName: 'pi-acp',
        version: '0.0.34', reopenBy: 'session/load', cwd: '/w', title: 'From the web');
    first.keepText('local:Pi', 's1', [KeptMessage('user', 'kept here', DateTime.utc(2026))]);
    await first.flush();

    final second = SessionLedger(IndexedDbLedgerBackend(database: database), settings: const LedgerSettings(keepText: true));
    await second.load();
    final entry = second.entries.single;
    expect((entry.harness, entry.title, entry.messages.single.text), ('Pi', 'From the web', 'kept here'));

    second.clearText();
    await second.flush();
    expect(await IndexedDbLedgerBackend(database: database).read(), isNot(contains('kept here')));
  });
}
