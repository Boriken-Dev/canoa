// The Store page with its programs, the plan an install shows before it runs, and an install in progress, drawn at phone and desktop
// width. With CANOA_SAMPLES_DIR set the scenes are written there with real fonts instead of compared (test/flutter_test_config.dart).
import 'dart:async';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show MemoryHome, MemoryOverlays, PlatformTolerant;
import 'fixtures/store_link.dart';

FakeStoreHost _host() {
  final host = FakeStoreHost([
    FakeStore(
      'one_store',
      title: 'Program One',
      serves: [
        {'provider': 'one', 'when': <String, Object?>{}},
      ],
      copies: [copyOf('2.1.322')],
      latest: '2.1.330',
      plan: {'update': planOf('update', '2.1.330', from: '2.1.322', bytes: 61 * 1024 * 1024)},
    ),
    FakeStore(
      'two_store',
      title: 'Program Two',
      serves: [
        {'provider': 'stdio', 'when': {'harness': 'two'}},
      ],
      latest: '0.167.1',
      plan: {
        'install': planOf(
          'install',
          '0.167.1',
          bytes: 38 * 1024 * 1024,
          downloads: [
            {'url': 'https://downloads.example.test/two/0.167.1/two-windows-arm64.zip', 'bytes': 38 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'c0' * 32}},
            {'url': 'https://downloads.example.test/two/0.167.1/two-helper.bin', 'bytes': 1024 * 1024, 'check': {'kind': 'unverified', 'why': 'the publisher lists no checksum for this file'}},
          ],
          steps: ['unpack two-windows-arm64.zip into the program\'s folder', 'run the helper once to register its plugins'],
          changes: [],
          notices: ['Canoa starts this copy by its full path. Nothing is added to PATH.'],
        ),
      },
    ),
    FakeStore(
      'three_store',
      title: 'Program Three',
      serves: [
        {'provider': 'stdio', 'when': {'harness': 'three'}},
      ],
      copies: [copyOf('1.5.2', managed: false, path: '/usr/local/bin/three', update: {'kind': 'elsewhere', 'how': 'three self-update'})],
      latest: '1.5.2',
    ),
  ]);
  return host;
}

Future<FakeStoreHost> _open(WidgetTester tester, Size size) async {
  final host = _host();
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    home: LoadoutPage(config: const CanoaConfig(), home: MemoryHome(MemoryOverlays()), onSave: (changed) async => changed, programs: host.programs()),
  ));
  await tester.pumpAndSettle();
  // The programs are on a page of their own, reached from Loadout's menu.
  await tester.tap(find.byKey(const Key('loadout-programs')));
  await tester.pumpAndSettle();
  return host;
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/program_store_scenes_test.dart')));

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the Store page lists the programs at $label width', (tester) async {
      await _open(tester, size);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_programs_$label.png'));
    });

    testWidgets('an install\'s plan draws at $label width', (tester) async {
      await _open(tester, size);
      await tester.ensureVisible(find.byKey(const Key('program-install-two_store')));
      await tester.tap(find.byKey(const Key('program-install-two_store')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/program_plan_$label.png'));
    });

    testWidgets('an install in progress draws at $label width', (tester) async {
      final host = await _open(tester, size);
      final release = Completer<void>();
      host.onApply = (apply) async {
        apply.say('Fetching two-windows-arm64.zip');
        apply.say('Checking it against the publisher\'s SHA-256');
        apply.progress('Downloading two-windows-arm64.zip', 0.42);
        await release.future;
        return {'applied': true, 'version': '0.167.1', 'dir': '/programs/two'};
      };
      await tester.ensureVisible(find.byKey(const Key('program-install-two_store')));
      await tester.tap(find.byKey(const Key('program-install-two_store')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('plan-confirm')));
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(find.text('Downloading two-windows-arm64.zip'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/program_progress_$label.png'));
      release.complete();
      for (var i = 0; i < 20 && find.byKey(const Key('program-close')).evaluate().isEmpty; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      await tester.tap(find.byKey(const Key('program-close')));
      await tester.pumpAndSettle();
    });
  }
}
