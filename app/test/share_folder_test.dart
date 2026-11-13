// A folder on a Windows share (\\server\share\folder) is an ordinary folder for a session: the folder checks and the list of
// recent folders take it. The real checks run against this machine's own administrative share, so nothing leaves it.
import 'dart:io';

import 'package:canoa/acp/local_agent_io.dart';
import 'package:canoa/acp/share_path.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/foundation.dart' show ValueKey;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';

/// [path], a drive path, as the same place named through the administrative share.
String _onShare(String path) => '\\\\localhost\\${path[0]}\$${path.substring(2)}';

void main() {
  final notWindows = !Platform.isWindows; // a share is a Windows thing

  test('a whole folder path is a root, a drive or a share, never a device or a bare server', () {
    for (final whole in ['/home/ana/src', r'C:\work', 'c:/work', r'\\fileserver\projects\app', '//fileserver/projects', r'\\?\UNC\fileserver\projects\app']) {
      expect(isWholeFolderPath(whole), isTrue, reason: whole);
    }
    for (final not in ['', 'relative/dir', r'C:work', r'\\fileserver', r'\\fileserver\', r'\\.\pipe\x', r'\\?\Volume{1}\x', r'\work', r'\\\share\x', '//?/x']) {
      expect(isWholeFolderPath(not), isFalse, reason: not);
    }
  });

  group('on this machine\'s administrative share', () {
    late Directory made;
    late String shared;

    setUp(() {
      made = Directory.systemTemp.createTempSync('canoa-share-');
      shared = _onShare(made.path);
      Directory('$shared\\project\\src').createSync(recursive: true);
    });
    tearDown(() => made.deleteSync(recursive: true));

    test('the folder checks take a folder on a share', () {
      expect(folderExists('$shared\\project'), isTrue);
      expect(folderExists('$shared\\gone'), isFalse);
      expect(inGitRepository('$shared\\project\\src'), isFalse);
      Directory('$shared\\project\\.git').createSync();
      expect(inGitRepository('$shared\\project\\src'), isTrue);
    }, skip: notWindows);

    testWidgets('a recent folder on a share is listed, and one that is gone is not', (tester) async {
      var minute = 0;
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: () => DateTime.utc(2026, 11, 1, 9, minute++));
      for (final (index, folder) in ['$shared\\gone', '$shared\\project'].indexed) {
        ledger.record(peerId: 'local:Scripted', sessionId: 's$index', entry: 'scripted-agent', place: SessionPlace.local, cwd: folder);
      }
      final store = SessionStore(ledger: ledger);
      addTearDown(store.dispose);
      await pumpApp(tester, store, folderExists: folderExists);
      await newSession(tester);
      expect(find.byKey(ValueKey('recent-folder $shared\\project')), findsOneWidget);
      expect(find.byKey(ValueKey('recent-folder $shared\\gone')), findsNothing);
      await tester.tap(find.byKey(ValueKey('recent-folder $shared\\project')));
      await tester.pumpAndSettle();
      expect(store.selected!.cwd, '$shared\\project');
    }, skip: notWindows);
  });
}
