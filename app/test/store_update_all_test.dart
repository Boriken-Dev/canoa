// Update all in the Store: the button where updates are, the one combined plan before anything runs, the items applied one after
// another with the count, a failure that does not stop the rest and is named at the end, Retry that runs only the failed, and Stop
// that ends after the item being applied. With CANOA_SAMPLES_DIR set the scenes are written there with real fonts instead of compared.
@Timeout(Duration(minutes: 3))
library;

import 'dart:async';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/base_text.dart' show baseStrings;
import 'package:canoa/profile/overlays.dart' show OverlayAvailability;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/store/store_index_view.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/store/store_selection.dart';
import 'package:canoa/store/store_update_all.dart';
import 'package:canoa/store/store_update_all_view.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'fixtures/store_link.dart';
import 'loadout_test.dart' show teamRepo, withCatalog, world;

/// A candidate whose plan and run are written down in [log]; [fail] makes its run throw, [gate] holds it until completed.
UpdateCandidate _candidate(
  String id,
  List<String> log, {
  String? fail,
  Completer<void>? gate,
  String? cannotPlan,
  List<String> attention = const [],
  void Function(String id, bool start)? around,
}) => UpdateCandidate(
  id: id,
  title: 'Item $id',
  from: '1.0',
  to: '2.0',
  plan: () async {
    log.add('plan $id');
    if (cannotPlan != null) throw StateError(cannotPlan);
    return UpdateItem(
      from: '1.0',
      to: '2.0',
      changes: ['Download: 1.0 MB'],
      attention: attention,
      run: (run) async {
        log.add('run $id');
        around?.call(id, true);
        run.say('Working on $id');
        if (gate != null) await gate.future;
        around?.call(id, false);
        if (fail != null) throw StateError(fail);
      },
    );
  },
);

Future<void> _show(WidgetTester tester, List<UpdateCandidate> candidates, {Size size = const Size(900, 900)}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Builder(
      builder: (context) => Scaffold(
        body: TextButton(key: const Key('open'), onPressed: () => unawaited(showUpdateAll(context, candidates)), child: const Text('open')),
      ),
    ),
  ));
  await tester.tap(find.byKey(const Key('open')));
  await tester.pumpAndSettle();
}

String? _text(WidgetTester tester, String key) => tester.widget<Text>(find.byKey(Key(key))).data;

Future<void> _tap(WidgetTester tester, String key) async {
  await tester.ensureVisible(find.byKey(Key(key)));
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(Key(key)));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/store_update_all_test.dart')));

  group('the plan and the run', () {
    testWidgets('the plan lists every item with its versions, what it changes and what needs attention, and nothing runs before the yes', (tester) async {
      final log = <String>[];
      await _show(tester, [
        _candidate('a', log),
        _candidate('b', log, attention: ['Not verified: https://example.test/b.zip']),
        _candidate('c', log),
      ]);
      for (final id in ['a', 'b', 'c']) {
        expect(find.byKey(Key('update-all-title-$id')), findsOneWidget);
        expect(_text(tester, 'update-all-versions-$id'), '1.0 → 2.0');
        expect(find.byKey(Key('update-all-change-$id-0')), findsOneWidget);
      }
      expect(find.byKey(const Key('update-all-attention-b-0')), findsOneWidget);
      expect(find.byKey(const Key('update-all-attention-a-0')), findsNothing);
      expect(find.text('Update all (3)'), findsOneWidget);
      expect(log, ['plan a', 'plan b', 'plan c'], reason: 'planned, not run');
      await _tap(tester, 'update-all-cancel');
      expect(find.byKey(const Key('update-all')), findsNothing);
      expect(log.where((line) => line.startsWith('run')), isEmpty, reason: 'Cancel in the plan runs nothing');
    });

    testWidgets('the items run one after another, never two at once, and the summary says all are updated', (tester) async {
      final log = <String>[];
      var running = 0, most = 0;
      void around(String id, bool start) {
        running += start ? 1 : -1;
        if (running > most) most = running;
      }
      await _show(tester, [_candidate('a', log, around: around), _candidate('b', log, around: around), _candidate('c', log, around: around)]);
      await _tap(tester, 'update-all-confirm');
      expect(log.where((line) => line.startsWith('run')), ['run a', 'run b', 'run c']);
      expect(most, 1);
      expect(find.byKey(const Key('update-all-summary')), findsOneWidget);
      expect(find.text('All 3 are updated.'), findsOneWidget);
      expect(find.byKey(const Key('update-all-retry')), findsNothing);
    });

    testWidgets('one failure does not stop the rest, and the summary names it with its reason', (tester) async {
      final log = <String>[];
      await _show(tester, [_candidate('a', log), _candidate('b', log, fail: 'the disk is full'), _candidate('c', log)]);
      await _tap(tester, 'update-all-confirm');
      expect(log.where((line) => line.startsWith('run')), ['run a', 'run b', 'run c'], reason: 'c ran after b failed');
      expect(find.text('2 updated, 1 failed, 0 not run.'), findsOneWidget);
      expect(_text(tester, 'update-all-reason-b'), 'the disk is full');
      expect(find.byKey(const Key('update-all-reason-a')), findsNothing);
      expect(find.byKey(const Key('update-all-reason-c')), findsNothing);
    });

    testWidgets('Retry runs only the failed ones, and the plan the person confirmed is the one that is run again', (tester) async {
      final log = <String>[];
      var broken = true;
      final candidates = [
        _candidate('a', log),
        UpdateCandidate(
          id: 'b',
          title: 'Item b',
          plan: () async {
            log.add('plan b');
            return UpdateItem(run: (run) async {
              log.add('run b');
              if (broken) throw StateError('the publisher did not answer');
            });
          },
        ),
        _candidate('c', log),
      ];
      await _show(tester, candidates);
      await _tap(tester, 'update-all-confirm');
      expect(find.text('Retry the failed (1)'), findsOneWidget);
      broken = false;
      await _tap(tester, 'update-all-retry');
      expect(log, ['plan a', 'plan b', 'plan c', 'run a', 'run b', 'run c', 'run b'], reason: 'a and c were not run again, and b was not planned again');
      expect(find.text('All 3 are updated.'), findsOneWidget);
      expect(find.byKey(const Key('update-all-retry')), findsNothing);
    });

    testWidgets('an item that cannot be planned is listed with the reason, the others are updated, and Retry plans it again', (tester) async {
      final log = <String>[];
      var unreachable = true;
      final candidates = [
        _candidate('a', log),
        UpdateCandidate(
          id: 'b',
          title: 'Item b',
          plan: () async {
            log.add('plan b');
            if (unreachable) throw StateError('the source could not be asked');
            return UpdateItem(run: (run) async => log.add('run b'));
          },
        ),
      ];
      await _show(tester, candidates);
      expect(find.text('Cannot be updated: the source could not be asked'), findsOneWidget);
      expect(find.text('Update all (1)'), findsOneWidget);
      await _tap(tester, 'update-all-confirm');
      expect(log, ['plan a', 'plan b', 'run a']);
      expect(find.text('1 updated, 1 failed, 0 not run.'), findsOneWidget);
      unreachable = false;
      await _tap(tester, 'update-all-retry');
      expect(log, ['plan a', 'plan b', 'run a', 'plan b', 'run b']);
      expect(find.text('All 2 are updated.'), findsOneWidget);
    });

    testWidgets('Stop ends the run after the item being applied: that one finishes, the others are not run, and Close ends it', (tester) async {
      final log = <String>[];
      final gate = Completer<void>();
      await _show(tester, [_candidate('a', log), _candidate('b', log, gate: gate), _candidate('c', log), _candidate('d', log)]);
      await tester.tap(find.byKey(const Key('update-all-confirm')));
      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 50));
      expect(log.where((line) => line.startsWith('run')), ['run a', 'run b'], reason: 'b is the one being applied');
      expect(find.text('Updating 2 of 4'), findsOneWidget);
      expect(find.text('Working on b'), findsOneWidget);
      await tester.tap(find.byKey(const Key('update-all-stop')));
      await tester.pump();
      expect(find.text('Stopping after this one…'), findsOneWidget);
      expect(log.where((line) => line.startsWith('run')), ['run a', 'run b'], reason: 'stopping does not end the item being applied');
      gate.complete();
      await tester.pumpAndSettle();
      expect(log.where((line) => line.startsWith('run')), ['run a', 'run b'], reason: 'c and d were not started');
      expect(find.text('2 updated, 0 failed, 2 not run.'), findsOneWidget);
      expect(find.byKey(const Key('update-all-not-run-c')), findsOneWidget);
      expect(find.byKey(const Key('update-all-retry')), findsNothing, reason: 'nothing failed: what was not run is asked for again by Update all');
      await _tap(tester, 'update-all-close');
      expect(find.byKey(const Key('update-all')), findsNothing);
    });

    test('every word of Update all has a Spanish text, with the same placeholders', () {
      final english = baseStrings['en']!, spanish = baseStrings['es']!;
      final keys = english.keys.where((key) => key.startsWith('store.update_all'));
      expect(keys, isNotEmpty);
      for (final key in keys) {
        expect(spanish[key], isNotNull, reason: key);
        final names = RegExp(r'\{(\w+)\}');
        expect({for (final m in names.allMatches(spanish[key]!)) m.group(1)}, {for (final m in names.allMatches(english[key]!)) m.group(1)}, reason: key);
      }
      expect(const AppText(language: 'es')('store.update_all_count', {'count': 2}), 'Actualizar todo (2)');
    });
  });

  group('in the Store', () {
    late FakeStoreHost host;

    FakeStoreHost fakeHost({bool updates = true}) => FakeStoreHost([
      FakeStore(
        'one_store',
        title: 'Program One',
        serves: [{'provider': 'one', 'when': <String, Object?>{}}],
        copies: [copyOf('2.1.322')],
        latest: updates ? '2.1.330' : '2.1.322',
        plan: {'update': planOf('update', '2.1.330', from: '2.1.322', bytes: 61 * 1024 * 1024)},
      ),
      FakeStore(
        'two_store',
        title: 'Program Two',
        serves: [{'provider': 'stdio', 'when': {'harness': 'two'}}],
        copies: [copyOf('0.160.0')],
        latest: updates ? '0.167.1' : '0.160.0',
        plan: {
          'update': planOf(
            'update',
            '0.167.1',
            from: '0.160.0',
            bytes: 38 * 1024 * 1024,
            downloads: [
              {'url': 'https://downloads.example.test/two/two.zip', 'bytes': 38 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'c0' * 32}},
              {'url': 'https://downloads.example.test/two/two-helper.bin', 'bytes': 1024, 'check': {'kind': 'unverified', 'why': 'no checksum is published'}},
            ],
            changes: [
              {'kind': 'path', 'target': 'PATH', 'detail': 'Adds its folder.', 'undone_by_uninstall': true},
            ],
            notices: ['Program Two is running: it restarts when the update is applied.'],
          ),
        },
      ),
      FakeStore(
        'three_store',
        title: 'Program Three',
        serves: [{'provider': 'stdio', 'when': {'harness': 'three'}}],
        copies: [copyOf('1.5.2', managed: false, path: '/usr/local/bin/three', update: {'kind': 'elsewhere', 'how': 'three self-update'})],
        latest: '1.6.0',
      ),
    ]);

    Future<void> openAgents(WidgetTester tester, FakeStoreHost with_, {Size size = const Size(1280, 900)}) async {
      host = with_;
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: StoreSection(config: const CanoaConfig(), home: MemoryHome(MemoryOverlays()), onSave: (changed) async => changed, programs: host.programs()),
      ));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-tab-agents')));
      await tester.pumpAndSettle();
    }

    testWidgets('the button appears only where an update the Store can apply is: none when every program is current', (tester) async {
      await openAgents(tester, fakeHost(updates: false));
      expect(find.byKey(const Key('store-update-all')), findsNothing);
    });

    testWidgets('a program found elsewhere, which the Store cannot update, is not counted, and the button names the two it can', (tester) async {
      await openAgents(tester, fakeHost());
      expect(find.byKey(const Key('program-update-three_store')), findsNothing);
      expect(find.descendant(of: find.byKey(const Key('store-update-all')), matching: find.text('Update all (2)')), findsOneWidget);
    });

    testWidgets('the combined plan asks the host for each plan and applies none until the yes, then applies in order', (tester) async {
      await openAgents(tester, fakeHost());
      await _tap(tester, 'store-update-all');
      expect(find.byKey(const Key('update-all')), findsOneWidget);
      expect(_text(tester, 'update-all-versions-one_store'), '2.1.322 → 2.1.330');
      expect(_text(tester, 'update-all-versions-two_store'), '0.160.0 → 0.167.1');
      expect(find.text('Download: 38 MB'), findsOneWidget);
      expect(find.text('path: PATH'), findsOneWidget);
      expect(find.byKey(const Key('update-all-attention-two_store-0')), findsOneWidget, reason: 'the download nothing checks');
      expect(find.text('Program Two is running: it restarts when the update is applied.'), findsOneWidget);
      expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
      expect(host.calls.where((call) => call.startsWith('store/plan')), ['store/plan one_store', 'store/plan two_store']);
      await _tap(tester, 'update-all-confirm');
      expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store', 'store/apply two_store']);
      expect(find.text('All 2 are updated.'), findsOneWidget);
    });

    testWidgets('a program that fails to apply is named with the host\'s words, the other is updated, and Retry applies only the failed', (tester) async {
      await openAgents(tester, fakeHost());
      var broken = true;
      host.onApply = (apply) async {
        if (apply.params['provider'] == 'one_store' && broken) throw ControlFailed(-32002, 'another install of this program is running');
        return {'applied': true, 'version': (apply.params['plan'] as Map)['version'], 'dir': '/programs/x'};
      };
      await _tap(tester, 'store-update-all');
      await _tap(tester, 'update-all-confirm');
      expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store', 'store/apply two_store']);
      expect(find.text('1 updated, 1 failed, 0 not run.'), findsOneWidget);
      expect(_text(tester, 'update-all-reason-one_store'), 'another install of this program is running');
      broken = false;
      await _tap(tester, 'update-all-retry');
      expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store', 'store/apply two_store', 'store/apply one_store']);
      expect(host.calls.where((call) => call.startsWith('store/plan')).length, 2, reason: 'the confirmed plan was applied again, not made again');
      expect(find.text('All 2 are updated.'), findsOneWidget);
    });

    testWidgets('Stop after the first program leaves the second where it was', (tester) async {
      await openAgents(tester, fakeHost());
      final gate = Completer<void>();
      host.onApply = (apply) async {
        apply.say('Fetching ${apply.params['provider']}');
        await gate.future;
        return {'applied': true, 'version': (apply.params['plan'] as Map)['version'], 'dir': '/programs/x'};
      };
      await _tap(tester, 'store-update-all');
      await tester.tap(find.byKey(const Key('update-all-confirm')));
      for (var i = 0; i < 10; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(find.text('Updating 1 of 2'), findsOneWidget);
      expect(find.text('Fetching one_store'), findsOneWidget);
      await tester.tap(find.byKey(const Key('update-all-stop')));
      await tester.pump();
      gate.complete();
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store']);
      expect(find.text('1 updated, 0 failed, 1 not run.'), findsOneWidget);
    });

    for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
      testWidgets('the Agents page with its Update all button, and the combined plan, draw at $label width', (tester) async {
        await openAgents(tester, fakeHost(), size: size);
        expect(find.byKey(const Key('store-update-all')), findsOneWidget);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_button_$label.png'));
        await _tap(tester, 'store-update-all');
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_plan_$label.png'));
      });

      testWidgets('Update all in progress, and its summary with a failure, draw at $label width', (tester) async {
        await openAgents(tester, fakeHost(), size: size);
        final gate = Completer<void>();
        host.onApply = (apply) async {
          apply.progress('Downloading ${apply.params['provider']}', 0.42);
          await gate.future;
          if (apply.params['provider'] == 'two_store') throw ControlFailed(-32002, 'the download did not match its checksum');
          return {'applied': true, 'version': (apply.params['plan'] as Map)['version'], 'dir': '/programs/x'};
        };
        await _tap(tester, 'store-update-all');
        await tester.tap(find.byKey(const Key('update-all-confirm')));
        for (var i = 0; i < 10; i++) {
          await tester.pump(const Duration(milliseconds: 50));
        }
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_progress_$label.png'));
        gate.complete();
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('update-all-retry')), findsOneWidget);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_summary_$label.png'));
      });
    }
  });

  group('across the Store', () {
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    late CanoaConfig config;
    late FakeStoreHost host;

    Future<void> open(WidgetTester tester, Size size, {CanoaConfig? using}) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      config = using ?? CanoaConfig.parse(withCatalog);
      final layout = ShellLayout()..section = 'store';
      final selection = StoreSelection(layout: layout);
      final section = StoreSection(
        readAhead: false,
        embedded: true,
        selection: selection,
        config: config,
        home: MemoryHome(overlays),
        programs: host.programs(),
        index: standInIndex(overlays, () => saved.isEmpty ? config : saved.last),
        catalogTypes: standInCatalogTypes,
        agents: const ['Claude'],
        onSave: (changed) async {
          saved.add(changed);
          return changed;
        },
        readCatalog: standInCatalogRead(overlays, () => saved.isEmpty ? config : saved.last, registry: (uri) async => '{"servers": []}'),
      );
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: AppShell(
          layout: layout,
          narrow: size.width < 730,
          sections: [
            AppSection(
              id: 'store',
              label: 'Store',
              icon: Icons.storefront_outlined,
              index: StoreIndex(selection: selection),
              ownsLayoutControls: true,
              main: section,
            ),
          ],
        ),
      ));
      await tester.pumpAndSettle();
    }

    /// The review overlay installed through its page, then the Store opened again while its source holds something newer.
    Future<void> withOverlayUpdate(WidgetTester tester, Size size) async {
      await open(tester, size);
      await tester.binding.setSurfaceSize(Size(size.width, 2400));
      await _tap(tester, 'overlays-discover');
      await _tap(tester, 'store-entry-main-review');
      await _tap(tester, 'store-fetch');
      await _tap(tester, 'store-install');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.binding.setSurfaceSize(size);
      overlays.availability['review'] = const OverlayAvailability(state: 'available', installed: 'a1b2c3d4e5f6', latest: '0f9e8d7c6b5a', follows: 'branch');
      while (tester.takeException() != null) {}
      await tester.pumpWidget(const SizedBox());
      await tester.pumpAndSettle();
      await open(tester, size, using: saved.last);
      overlays.put(teamRepo, {
        'overlay.cfg': 'name = "review"\n[about]\nsummary = "Review conventions, newer."\n',
        'skills/review/SKILL.md': '# Review\n',
      });
    }

    setUp(() {
      overlays = world();
      saved = [];
      host = FakeStoreHost([
        FakeStore(
          'one_store',
          title: 'Program One',
          serves: [{'provider': 'one', 'when': <String, Object?>{}}],
          copies: [copyOf('2.1.322')],
          latest: '2.1.330',
          plan: {'update': planOf('update', '2.1.330', from: '2.1.322', bytes: 61 * 1024 * 1024)},
        ),
      ]);
    });

    for (final (label, size) in [('phone', const Size(390, 900)), ('desktop', const Size(1280, 800))]) {
      testWidgets('the index counts every update and its Update all plans the overlays and the programs together at $label width', (tester) async {
        await withOverlayUpdate(tester, size);
        final button = find.byKey(const Key('store-index-update-all'));
        if (label == 'phone') {
          // On a narrow window the index is the drawer.
          await tester.tap(find.byKey(const Key('open-navigation')));
          await tester.pumpAndSettle();
        }
        expect(find.descendant(of: button, matching: find.text('Update all (2)')), findsOneWidget);
        expect(find.byKey(const Key('store-index-overlays-updates')), findsOneWidget);
        expect(find.byKey(const Key('store-index-agents-updates')), findsOneWidget);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_index_$label.png'));
        await tester.tap(button);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('update-all')), findsOneWidget);
        expect(find.text('Overlays'), findsWidgets, reason: 'the plan groups the items by the type they are in');
        expect(find.byKey(const Key('update-all-title-Overlays/review')), findsOneWidget);
        expect(find.byKey(const Key('update-all-title-Agents/one_store')), findsOneWidget);
        expect(find.byKey(const Key('update-all-details-Overlays/review')), findsOneWidget);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/update_all_mixed_plan_$label.png'));
        final before = overlays.records['review']!.commit;
        await tester.tap(find.byKey(const Key('update-all-confirm')));
        await tester.pumpAndSettle();
        expect(find.text('All 2 are updated.'), findsOneWidget);
        expect(overlays.records['review']!.commit, isNot(before), reason: 'the newer overlay was installed');
        expect(saved.last.overlays['review'], isNotNull);
        expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store']);
        await tester.tap(find.byKey(const Key('update-all-close')));
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('store-index-overlays-updates')), findsNothing, reason: 'the host no longer lists an update');
      });
    }

    testWidgets('the Overlays page has its own Update all, for its overlays only', (tester) async {
      await withOverlayUpdate(tester, const Size(1280, 900));
      await _tap(tester, 'store-index-overlays');
      await _tap(tester, 'overlays-installed');
      final button = find.byKey(const Key('store-update-all'));
      expect(find.descendant(of: button, matching: find.text('Update all (1)')), findsOneWidget);
      await tester.tap(button);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('update-all-title-review')), findsOneWidget);
      expect(find.byKey(const Key('update-all-title-one_store')), findsNothing);
      expect(find.byKey(const Key('update-all-attention-review-0')), findsOneWidget, reason: 'its manifest changes');
    });

    testWidgets('an overlay whose update fails is named, and the program after it is still updated', (tester) async {
      await withOverlayUpdate(tester, const Size(1280, 900));
      overlays.repos.remove(teamRepo);
      await tester.tap(find.byKey(const Key('store-index-update-all')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('update-all-reason-Overlays/review')), findsOneWidget, reason: 'its source cannot be fetched now');
      await tester.tap(find.byKey(const Key('update-all-confirm')));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.startsWith('store/apply')), ['store/apply one_store']);
      expect(find.text('1 updated, 1 failed, 0 not run.'), findsOneWidget);
    });
  });
}
