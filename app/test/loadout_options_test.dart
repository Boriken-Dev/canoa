// Loadout: the form for the options an overlay declares, written to `[overlays.NAME] options`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

const catalogRepo = 'https://example.invalid/catalog.git';
const teamRepo = 'https://example.invalid/review.git';

const optionsCfg = '''
name = "review"
[about]
summary = "Review conventions."

[options.REVIEW_STYLE]
type = "string"
description = "How the reviewer words its notes."
default = "terse"

[options.REVIEW_DEPTH]
type = "integer"
description = "How many files deep to look."
default = 3

[options.REVIEW_STRICT]
type = "boolean"
description = "Refuse a change with any open note."
default = false

[options.REVIEW_TOKEN]
type = "string"
sensitive = true
description = "The tracker token."
''';

const catalog = '''
name = "Team"

[overlays.review]
source = "$teamRepo@main"
about = { summary = "Review conventions.", tags = ["review"] }
''';

MemoryOverlays world() => MemoryOverlays()
  ..put(catalogRepo, {'catalog.toml': catalog})
  ..put(teamRepo, {'overlay.cfg': optionsCfg});

const withCatalog = '[catalogs]\nmain = "$catalogRepo@main#catalog.toml"\n';

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/loadout_options_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 900)), ('phone', const Size(390, 1100))]) {
    group('at $label width', () {
      late List<CanoaConfig> saved;

      Future<void> tap(WidgetTester tester, String key) async {
        await tester.ensureVisible(find.byKey(Key(key)));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(Key(key)));
        await tester.pumpAndSettle();
      }

      Future<void> open(WidgetTester tester) async {
        saved = [];
        await tester.binding.setSurfaceSize(Size(size.width, 3000));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        var current = CanoaConfig.parse(withCatalog);
        await tester.pumpWidget(MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: ThemeData(brightness: Brightness.dark, useMaterial3: true),
          home: LoadoutPage(
            config: current,
            home: MemoryHome(world()),
            agents: const ['Claude', 'Codex'],
            onSave: (changed) async {
              saved.add(changed);
              return current = changed;
            },
            fetchRegistry: (uri) async => '',
            findProgram: (program) async => '/usr/bin/$program',
          ),
        ));
        await tester.pumpAndSettle();
        for (final key in const ['overlays-discover', 'store-entry-main-review', 'store-fetch', 'store-install']) {
          await tap(tester, key);
        }
        await tester.pageBack();
        await tester.pumpAndSettle();
        await tap(tester, 'overlays-installed');
        await tester.binding.setSurfaceSize(size);
        await tester.pumpAndSettle();
        await tap(tester, 'overlay-options-review');
      }

      testWidgets('a string, an integer and a boolean are written to the overlay, a sensitive option has no field', (tester) async {
        await open(tester);
        expect(find.byKey(const Key('overlay-option-review-REVIEW_STYLE')), findsOneWidget);
        expect(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), findsOneWidget);
        expect(find.byKey(const Key('overlay-option-review-REVIEW_STRICT')), findsOneWidget);
        expect(find.byKey(const Key('overlay-option-review-REVIEW_TOKEN')), findsNothing);
        expect(find.textContaining('The tracker token'), findsNothing);
        expect(find.textContaining('How many files deep to look. · Default: 3'), findsOneWidget);
        expect(tester.widget<FilledButton>(find.byKey(const Key('overlay-options-save-review'))).onPressed, isNull,
            reason: 'nothing changed yet');

        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_STYLE')), 'verbose');
        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), '5');
        final on = find.descendant(of: find.byKey(const Key('overlay-option-review-REVIEW_STRICT')), matching: find.text('On'));
        await tester.ensureVisible(on);
        await tester.pumpAndSettle();
        await tester.tap(on);
        await tester.pumpAndSettle();
        await tap(tester, 'overlay-options-save-review');

        expect(saved.last.overlayOptions('review'), {'REVIEW_STYLE': 'verbose', 'REVIEW_DEPTH': 5, 'REVIEW_STRICT': true});
        final encoded = CanoaConfig.parse(saved.last.encode());
        expect(encoded.overlayOptions('review'), {'REVIEW_STYLE': 'verbose', 'REVIEW_DEPTH': 5, 'REVIEW_STRICT': true});
        expect(encoded.overlays.keys, ['review'], reason: 'the overlay keeps its source');
        expect(saved.last.encode(), isNot(contains('REVIEW_TOKEN')));
        await golden(tester, 'set', label);
      });

      testWidgets('an empty field is left out, so the default applies; a wrong-typed integer cannot be saved', (tester) async {
        await open(tester);
        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_STYLE')), 'verbose');
        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), '5');
        await tap(tester, 'overlay-options-save-review');
        expect(saved.last.overlayOptions('review'), {'REVIEW_STYLE': 'verbose', 'REVIEW_DEPTH': 5});

        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), '');
        await tester.pumpAndSettle();
        await tap(tester, 'overlay-options-save-review');
        expect(saved.last.overlayOptions('review'), {'REVIEW_STYLE': 'verbose'});

        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), '1-2');
        await tester.pumpAndSettle();
        expect(find.text('A whole number'), findsOneWidget);
        expect(tester.widget<FilledButton>(find.byKey(const Key('overlay-options-save-review'))).onPressed, isNull);

        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_STYLE')), '');
        await tester.enterText(find.byKey(const Key('overlay-option-review-REVIEW_DEPTH')), '');
        await tap(tester, 'overlay-options-save-review');
        expect(saved.last.overlayOptions('review'), isEmpty);
        expect(saved.last.encode(), isNot(contains('options')));
      });

      testWidgets('the form with nothing set shows each option with its default', (tester) async {
        await open(tester);
        await golden(tester, 'unset', label);
      });
    });
  }
}

Future<void> golden(WidgetTester tester, String name, String label) async {
  expect(tester.takeException(), isNull);
  await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/loadout_options_${name}_$label.png'));
}
