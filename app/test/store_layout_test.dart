// The Store section: its index lists the stores, the content shows the chosen store as a banner and tiles with a +, the index
// toggle lives at the top right only, and the choice survives a restart.

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/store/store_index_view.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/store/store_selection.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';

const _teamRepo = 'https://example.invalid/team.git';
const _otherRepo = 'https://example.invalid/other.git';

const _team = '''
name = "Team overlays"
description = "What the team shares."

[overlays.review]
source = "./overlays/review"
[overlays.review.about]
summary = "Review conventions."
authors = ["Ana"]
tags = ["docs", "featured"]

[overlays.docs-tools]
source = "./overlays/docs-tools"
[overlays.docs-tools.about]
summary = "Docs tools."
tags = ["docs"]
''';

const _other = '''
[overlays.lint]
source = "./overlays/lint"
[overlays.lint.about]
summary = "Lint rules."
''';

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/store_layout_test.dart')));

  group('the section rail', () {
    testWidgets('has no index toggle; the layout control at the top right is the one', (tester) async {
      final layout = ShellLayout();
      await tester.binding.setSurfaceSize(const Size(1100, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: AppShell(
          layout: layout,
          narrow: false,
          sections: const [
            AppSection(id: 'a', label: 'A', icon: Icons.chat, index: Material(child: Text('a index')), main: Material(child: Text('a main'))),
          ],
        ),
      ));
      await tester.pump();
      expect(find.descendant(of: find.byType(SectionRail), matching: find.byType(IconButton)), findsNothing);
      expect(find.byKey(const ValueKey('toggle-index')), findsNothing);
      expect(find.byKey(const ValueKey('layout-left')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('layout-left')));
      await tester.pump();
      expect(find.text('a index'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('layout-left')));
      await tester.pump();
      expect(find.text('a index'), findsOneWidget);
    });
  });

  group('the selection', () {
    test('is kept in the layout, and a store no longer listed reads as every store', () async {
      final backend = MemoryLayoutBackend();
      final layout = ShellLayout(backend: backend);
      StoreSelection(layout: layout).select('main');
      await layout.flush();
      final again = ShellLayout(backend: backend);
      await again.load();
      final restored = StoreSelection(layout: again);
      expect(restored.selected, 'main', reason: 'kept until the stores are read');
      restored.publish(const [], loading: false);
      expect(restored.selected, isNull, reason: 'the store is gone');
    });
  });

  group('the Store section', () {
    final config = CanoaConfig.parse('[catalogs]\nteam = "$_teamRepo@main#catalog.toml"\nother = "$_otherRepo@main#catalog.toml"\n');
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    late ShellLayout layout;

    setUp(() {
      overlays = MemoryOverlays()
        ..put(_teamRepo, {
          'catalog.toml': _team,
          'overlays/review/overlay.cfg': '[env]\nREVIEW = "1"\n',
          'overlays/docs-tools/overlay.cfg': '[env]\nDOCS = "1"\n',
        })
        ..put(_otherRepo, {'catalog.toml': _other, 'overlays/lint/overlay.cfg': '[env]\nLINT = "1"\n'});
      saved = [];
      layout = ShellLayout();
    });

    Future<void> open(WidgetTester tester, {Size size = const Size(1200, 800), ShellLayout? using}) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final shell = using ?? layout;
      shell.section = 'store';
      final selection = StoreSelection(layout: shell);
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: AppShell(
          layout: shell,
          narrow: size.width < 730,
          sections: [
            AppSection(
              id: 'store',
              label: 'Store',
              icon: Icons.storefront_outlined,
              index: StoreIndex(selection: selection),
              ownsLayoutControls: true,
              main: StoreSection(
                embedded: true,
                selection: selection,
                config: config,
                home: MemoryHome(overlays),
                index: standInIndex(overlays, () => saved.isEmpty ? config : saved.last),
                onSave: (changed) async {
                  saved.add(changed);
                  return changed;
                },
              ),
            ),
          ],
        ),
      ));
      await tester.pumpAndSettle();
    }

    testWidgets('the index lists the stores with what each offers, and a store shows only its own items', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('store-index-all')), findsOneWidget);
      expect(find.text('2 overlays · 0 skills'), findsOneWidget);
      expect(find.text('1 overlays · 0 skills'), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-index-team')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-banner')), findsOneWidget);
      expect(find.text('Team overlays'), findsWidgets);
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-docs-tools')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-other-lint')), findsNothing);
      expect(find.text('by Ana'), findsOneWidget);
      expect(find.byKey(const Key('store-tag-featured')), findsOneWidget, reason: 'the store\'s own tags');

      await tester.enterText(find.byKey(const Key('store-search')), 'lint');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-other-lint')), findsNothing, reason: 'search stays within the store');
      expect(find.byKey(const Key('overlays-nothing')), findsOneWidget);
      await tester.enterText(find.byKey(const Key('store-search')), '');
      await tester.tap(find.byKey(const Key('store-index-other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing);
      expect(find.byKey(const Key('store-tag-featured')), findsNothing, reason: 'another store\'s tag');
    });

    testWidgets('the banner pages through the store and its tags, and the + installs from the tile', (tester) async {
      await open(tester);
      await tester.tap(find.byKey(const Key('store-index-team')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-banner-dot-1')), findsOneWidget, reason: 'a second banner for the tag docs');
      await tester.tap(find.byKey(const Key('store-banner-next')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-banner-explore-docs')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-show-all')), findsOneWidget);

      await tester.tap(find.byKey(const Key('store-entry-install-team-docs-tools')));
      await tester.pumpAndSettle();
      expect(saved, hasLength(1));
      expect(saved.single.overlays.keys, contains('docs-tools'));
      expect(overlays.records['docs-tools']?.catalog, 'team');
      expect(find.descendant(of: find.byKey(const Key('store-entry-team-docs-tools')), matching: find.byIcon(Icons.check)), findsOneWidget);
    });

    for (final (label, size) in const [('desktop', Size(1200, 800)), ('phone', Size(420, 800))]) {
      testWidgets('an opened store as a banner and tiles, $label', (tester) async {
        await open(tester, size: size);
        if (label == 'phone') {
          await tester.tap(find.byKey(const Key('open-navigation')));
          await tester.pumpAndSettle();
        }
        await tester.tap(find.byKey(const Key('store-index-team')));
        await tester.pumpAndSettle();
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_page_$label.png'));
      });
    }

    testWidgets('the choice comes back after a restart', (tester) async {
      final backend = MemoryLayoutBackend();
      final first = ShellLayout(backend: backend);
      await open(tester, using: first);
      await tester.tap(find.byKey(const Key('store-index-other')));
      await tester.pumpAndSettle();
      await first.flush();
      await tester.pumpWidget(const SizedBox());
      final again = ShellLayout(backend: backend);
      await again.load();
      await open(tester, using: again);
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing);
    });

    testWidgets('a phone has the stores in the drawer, which closes once one is chosen, and the banner swipes', (tester) async {
      await open(tester, size: const Size(420, 800));
      expect(find.byKey(const Key('store-index-team')), findsNothing);
      await tester.tap(find.byKey(const Key('open-navigation')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-index-team')), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-index-team')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-index-team')), findsNothing, reason: 'the drawer closed');
      expect(find.byKey(const Key('store-banner')), findsOneWidget);
      expect(find.byKey(const Key('store-banner-next')), findsNothing, reason: 'arrows become swipes');
      await tester.drag(find.byKey(const Key('store-banner')), const Offset(-300, 0));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-banner-explore-docs')), findsOneWidget);
    });

    testWidgets('a store that could not be read says so in the index and in its content', (tester) async {
      overlays.repos.remove(_otherRepo);
      await open(tester);
      expect(find.text('Could not be read'), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-index-other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-source-note-other')), findsOneWidget);
    });
  });

}
