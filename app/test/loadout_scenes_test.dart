// Loadout drawn at desktop and phone width: what is installed, what is offered, the empty state with no catalog, a skill, an
// MCP server and the review of one from a registry. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same
// scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show registryAnswer, world, withCatalog;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/loadout_scenes_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      late MemoryOverlays overlays;
      setUp(() => overlays = world());

      Future<void> open(WidgetTester tester, {String text = withCatalog}) async {
        await tester.binding.setSurfaceSize(size);
        addTearDown(() => tester.binding.setSurfaceSize(null));
        await tester.pumpWidget(MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: ThemeData(brightness: Brightness.dark, useMaterial3: true),
          home: LoadoutPage(
            config: CanoaConfig.parse(text),
            home: MemoryHome(overlays),
            agents: const ['Claude', 'Codex'],
            onSave: (changed) async => changed,
            fetchRegistry: (uri) async => registryAnswer,
            findProgram: (program) async => '/usr/bin/$program',
          ),
        ));
        await tester.pumpAndSettle();
      }

      Future<void> tap(WidgetTester tester, String key) async {
        await tester.ensureVisible(find.byKey(Key(key)));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(Key(key)));
        await tester.pumpAndSettle();
      }

      Future<void> installReview(WidgetTester tester) async {
        // The overlay's page is a long list that builds what is near the screen: install on a tall one, draw on the real one.
        await tester.binding.setSurfaceSize(Size(size.width, 3000));
        for (final key in const ['overlays-discover', 'store-entry-main-review', 'store-fetch', 'store-install']) {
          await tap(tester, key);
        }
        await tester.pageBack();
        await tester.pumpAndSettle();
        await tester.binding.setSurfaceSize(size);
        await tester.pumpAndSettle();
      }

      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/loadout_${name}_$label.png'));
      }

      testWidgets('an installed overlay, with where it came from, what it applies and its default', (tester) async {
        await open(tester);
        await installReview(tester);
        await tap(tester, 'overlays-installed');
        await golden(tester, 'overlays');
      });

      testWidgets('the overlays a catalog offers, each with the catalog it is from', (tester) async {
        await open(tester);
        await tap(tester, 'overlays-discover');
        await golden(tester, 'discover');
      });

      testWidgets('no catalog yet: a catalog to start from and a spec to paste', (tester) async {
        await open(tester, text: '');
        await tap(tester, 'overlays-discover');
        await golden(tester, 'empty');
      });

      testWidgets('what the catalog offers as skills, each with its source and pin', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('loadout-tab-skills')));
        await tester.pumpAndSettle();
        await tap(tester, 'skills-discover');
        await golden(tester, 'skills_discover');
      });

      testWidgets('a skill chosen, fetched at its pin and on for new sessions', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('loadout-tab-skills')));
        await tester.pumpAndSettle();
        await tap(tester, 'skills-discover');
        await tap(tester, 'store-skill-install-main-pdf');
        await tap(tester, 'store-skill-install-confirm');
        await golden(tester, 'skills');
      });

      testWidgets('MCP servers: what the registry lists', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('loadout-tab-mcps')));
        await tester.pumpAndSettle();
        await tap(tester, 'mcps-discover');
        await golden(tester, 'mcps_discover');
      });

      testWidgets('the review of an MCP server before anything is written', (tester) async {
        await open(tester);
        await tester.tap(find.byKey(const Key('loadout-tab-mcps')));
        await tester.pumpAndSettle();
        await tap(tester, 'mcps-discover');
        await tap(tester, 'mcp-entry-io.github.ana/files');
        await golden(tester, 'mcp_review');
      });
    });
  }
}
