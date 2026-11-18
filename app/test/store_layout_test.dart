// The Store section: its index lists the store types (the built-in ones, then those an extension adds), the content shows the
// chosen type across every catalog as a banner and tiles with a +, a catalog filter narrows it, the index toggle lives at the
// top right only, and the choice survives a restart.

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/store/store_extension.dart';
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

[skills]
reviewing = "https://example.invalid/skills/reviewing.git"
''';

const _other = '''
[overlays.lint]
source = "./overlays/lint"
[overlays.lint.about]
summary = "Lint rules."

[skills]
linting = "https://example.invalid/skills/linting.git"
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
    StoreTypeRow row(String id) => StoreTypeRow(id: id, label: id, icon: Icons.apps, installed: 0);

    test('is kept in the layout, and a type no longer listed reads as the first', () async {
      final backend = MemoryLayoutBackend();
      final layout = ShellLayout(backend: backend);
      StoreSelection(layout: layout).select('skills');
      await layout.flush();
      final again = ShellLayout(backend: backend);
      await again.load();
      final restored = StoreSelection(layout: again);
      expect(restored.selected, 'skills', reason: 'kept until the types are listed');
      restored.publish([row('overlays'), row('skills')]);
      expect(restored.selected, 'skills');
      restored.publish([row('overlays'), row('mcps')]);
      expect(restored.selected, 'overlays', reason: 'the type is gone');
    });
  });

  group('the Store section', () {
    final config = CanoaConfig.parse('[store.overlays.catalogs]\nboriken = false\nteam = "$_teamRepo@main#catalog.toml"\nother = "$_otherRepo@main#catalog.toml"\n');
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    late ShellLayout layout;
    var installs = <String>[];

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
      installs = [];
    });

    ExtensionStoreType themes() => ExtensionStoreType(
      id: 'themes',
      label: 'Themes',
      items: () => [
        ExtensionStoreItem(
          catalog: 'team',
          name: 'dusk',
          by: 'by Ana',
          summary: 'A dark theme.',
          installed: installs.contains('dusk'),
          install: (_) async => installs.add('dusk'),
        ),
        const ExtensionStoreItem(catalog: 'other', name: 'dawn', by: 'by Bo', summary: 'A light theme.'),
      ],
    );

    Future<void> open(WidgetTester tester,
        {Size size = const Size(1200, 800), ShellLayout? using, List<ExtensionStoreType> extensions = const []}) async {
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
                extensionTypes: extensions,
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

    testWidgets('the index lists the store types, the built-in ones first, then the extension\'s, each with its count', (tester) async {
      await open(tester, extensions: [themes()]);
      final ids = ['overlays', 'skills', 'mcps', 'themes'];
      for (final id in ids) {
        expect(find.byKey(Key('store-index-$id')), findsOneWidget, reason: id);
      }
      expect(find.byKey(const Key('store-index-agents')), findsNothing, reason: 'the host grants no programs');
      expect(find.byKey(const Key('store-index-team')), findsNothing, reason: 'a catalog is not a type');
      final tops = [for (final id in ids) tester.getTopLeft(find.byKey(Key('store-index-$id'))).dy];
      expect(tops, [...tops]..sort(), reason: 'in order');
      expect(find.text('0 installed · 3 to discover'), findsOneWidget, reason: 'overlays across both catalogs');
      expect(find.text('0 installed · 2 to discover'), findsOneWidget, reason: 'themes: a skill an overlay catalog names comes with its overlay, so Skills counts none to discover');
      expect(find.byKey(const Key('store-tabs')), findsNothing, reason: 'the index chooses the type');
    });

    testWidgets('an extension\'s own picture is drawn in the index and on its page in place of an icon', (tester) async {
      final plain = themes();
      final pictured = ExtensionStoreType(
        id: 'pictured',
        label: 'Pictured',
        image: const StoreMarkImage(path: 'icon.png', bytes: [1, 2, 3]),
        items: () => const [],
      );
      await open(tester, extensions: [plain, pictured]);
      expect(find.byKey(const Key('store-index-pictured-image')), findsOneWidget);
      expect(find.byKey(const Key('store-index-themes-image')), findsNothing, reason: 'a type with no picture keeps its icon');
      await tester.tap(find.byKey(const Key('store-index-pictured')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-banner-image')), findsOneWidget);
    });

    testWidgets('a type shows its items across every catalog, and the catalog filter narrows them', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('store-banner')), findsOneWidget);
      expect(find.text('Top overlays'), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-docs-tools')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
      expect(find.text('by Ana · team'), findsOneWidget);
      expect(find.byKey(const Key('store-count')), findsOneWidget);
      expect(find.byKey(const Key('catalog-filter-all')), findsOneWidget);

      await tester.tap(find.byKey(const Key('catalog-filter-team')));
      await tester.pumpAndSettle();
      expect(find.text('Team overlays'), findsWidgets, reason: 'the catalog\'s own banner');
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-other-lint')), findsNothing);
      expect(find.byKey(const Key('store-tag-featured')), findsOneWidget, reason: 'the catalog\'s own tags');

      await tester.tap(find.byKey(const Key('catalog-filter-other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing);
      expect(find.byKey(const Key('store-tag-featured')), findsNothing, reason: 'another catalog\'s tag');

      await tester.tap(find.byKey(const Key('catalog-filter-all')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
    });

    testWidgets('removing the catalog the filter chose shows every catalog again, not an empty Store', (tester) async {
      await open(tester);
      await tester.tap(find.byKey(const Key('catalog-filter-other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing);

      final state = tester.state<StoreSectionState>(find.byType(StoreSection));
      await tester.runAsync(() => state.controller.removeCatalog('other'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget, reason: 'the gone catalog no longer filters');
      expect(find.byKey(const Key('store-entry-other-lint')), findsNothing);
      expect(state.view.catalogs, isEmpty);
    });

    testWidgets('choosing a type shows only that type, and a search stays within it', (tester) async {
      await open(tester);
      await tester.tap(find.byKey(const Key('store-index-skills')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
      expect(find.byKey(const Key('store-offered-skill-team-reviewing')), findsNothing, reason: 'a skill an overlay names comes with its overlay');
      expect(find.byKey(const Key('store-offered-skill-other-linting')), findsNothing);
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing, reason: 'an overlay is not a skill');

      await tester.enterText(find.byKey(const Key('store-search')), 'lint');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-other-lint')), findsNothing, reason: 'the overlay lint is another type\'s');

      await tester.tap(find.byKey(const Key('store-index-overlays')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget, reason: 'the search of Skills is Skills\' alone');
    });

    testWidgets('an extension\'s type has the same page, its catalog filter and a + that installs', (tester) async {
      await open(tester, extensions: [themes()]);
      await tester.tap(find.byKey(const Key('store-index-themes')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-banner')), findsOneWidget);
      expect(find.text('Top themes'), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-dusk')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-other-dawn')), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-entry-install-team-dusk')));
      await tester.pumpAndSettle();
      expect(installs, ['dusk']);
      await tester.tap(find.byKey(const Key('catalog-filter-other')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-team-dusk')), findsNothing);
      expect(find.byKey(const Key('store-entry-other-dawn')), findsOneWidget);
    });

    testWidgets('the banner pages through the catalog and its tags, and the + installs from the tile', (tester) async {
      await open(tester);
      await tester.tap(find.byKey(const Key('catalog-filter-team')));
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
      testWidgets('a type\'s page as a banner and tiles, $label', (tester) async {
        await open(tester, size: size);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_page_$label.png'));
      });
      testWidgets('the index of the store types, $label', (tester) async {
        await open(tester, size: size, extensions: [themes()]);
        if (label == 'phone') {
          await tester.tap(find.byKey(const Key('open-navigation')));
          await tester.pumpAndSettle();
        }
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_types_$label.png'));
      });
    }

    testWidgets('the choice comes back after a restart', (tester) async {
      final backend = MemoryLayoutBackend();
      final first = ShellLayout(backend: backend);
      await open(tester, using: first);
      await tester.tap(find.byKey(const Key('store-index-skills')));
      await tester.pumpAndSettle();
      await first.flush();
      await tester.pumpWidget(const SizedBox());
      final again = ShellLayout(backend: backend);
      await again.load();
      await open(tester, using: again);
      expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsNothing);
    });

    testWidgets('a phone has the types in the drawer, which closes once one is chosen', (tester) async {
      await open(tester, size: const Size(420, 800));
      expect(find.byKey(const Key('store-index-skills')), findsNothing);
      await tester.tap(find.byKey(const Key('open-navigation')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-index-skills')), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-index-skills')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-index-skills')), findsNothing, reason: 'the drawer closed');
      expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
    });

    testWidgets('a catalog that could not be read says so in the page', (tester) async {
      overlays.repos.remove(_otherRepo);
      await open(tester);
      expect(find.byKey(const Key('store-source-note-other')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
    });
  });
}
