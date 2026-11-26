// An installed overlay whose source holds something newer: the badge on its card, the Update button in place of Check, the update
// icon where a Discover tile shows a check, the count in the Store's index and tabs, the button that asks every source, and the
// plan an update shows. With CANOA_SAMPLES_DIR set the scenes are written there with real fonts instead of compared.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart' show OverlayAvailability;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/store/store_index_view.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/store/store_selection.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show teamRepo, withCatalog, world;

const _newer = OverlayAvailability(state: 'available', installed: 'a1b2c3d4e5f6', latest: '0f9e8d7c6b5a', follows: 'branch');

void main() {
  late MemoryOverlays overlays;
  late List<CanoaConfig> saved;
  late CanoaConfig config;
  var shown = const Size(1280, 800);

  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/overlay_updates_test.dart')));

  /// The Store as a section of the shell with its index beside it, or on its own with tabs.
  Future<void> open(WidgetTester tester, Size size, {bool index = false, CanoaConfig? using}) async {
    shown = size;
    await tester.binding.setSurfaceSize(size);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    config = using ?? CanoaConfig.parse(withCatalog);
    final layout = ShellLayout()..section = 'store';
    final selection = index ? StoreSelection(layout: layout) : null;
    final section = StoreSection(
      readAhead: false,
      embedded: index,
      selection: selection,
      config: config,
      home: MemoryHome(overlays),
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
      home: index
          ? AppShell(
              layout: layout,
              narrow: size.width < 730,
              sections: [
                AppSection(
                  id: 'store',
                  label: 'Store',
                  icon: Icons.storefront_outlined,
                  index: StoreIndex(selection: selection!),
                  ownsLayoutControls: true,
                  main: section,
                ),
              ],
            )
          : section,
    ));
    await tester.pumpAndSettle();
  }

  Future<void> tapKey(WidgetTester tester, String key) async {
    await tester.ensureVisible(find.byKey(Key(key)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key(key)));
    await tester.pumpAndSettle();
  }

  /// Installs the catalog's review overlay through its page, then shows the installed list.
  Future<void> installReview(WidgetTester tester) async {
    // A page lists only what it can reach, so the install button is found on a tall window.
    await tester.binding.setSurfaceSize(Size(shown.width, 2400));
    await tapKey(tester, 'overlays-discover');
    await tapKey(tester, 'store-entry-main-review');
    await tapKey(tester, 'store-fetch');
    await tapKey(tester, 'store-install');
    await tester.pageBack();
    await tester.pumpAndSettle();
    await tester.binding.setSurfaceSize(shown);
    await tapKey(tester, 'overlays-installed');
  }

  /// The review overlay installed, then the Store opened again while the host says [found] of its source: what a person sees on
  /// coming back to a Store whose host has looked at the sources.
  Future<void> installedWith(WidgetTester tester, Size size, OverlayAvailability found, {bool index = false}) async {
    await open(tester, size, index: index);
    await installReview(tester);
    overlays.availability['review'] = found;
    // What the pages of the install drew is not what these tests are about; the Store opened again is.
    while (tester.takeException() != null) {}
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();
    await open(tester, size, index: index, using: saved.last);
    await tapKey(tester, 'overlays-installed');
  }

  setUp(() {
    overlays = world();
    saved = [];
  });

  for (final (label, size) in [('phone', const Size(390, 900)), ('desktop', const Size(1280, 800))]) {
    testWidgets('an installed overlay with an update shows the badge, what is newer and an Update button at $label width', (tester) async {
      await installedWith(tester, size, _newer, index: label == 'desktop');
      expect(find.text('1 overlay has an update.'), findsOneWidget);
      expect(find.byKey(const Key('overlay-update-badge-review')), findsNothing, reason: 'the card in the list holds no badge');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/overlay_update_installed_$label.png'));
      await tapKey(tester, 'overlay-installed-review');
      final card = find.byKey(const Key('overlay-detail-review'));
      expect(find.descendant(of: card, matching: find.byKey(const Key('overlay-update-badge-review'))), findsOneWidget);
      expect(find.descendant(of: card, matching: find.text('Update available')), findsOneWidget);
      expect(find.descendant(of: card, matching: find.textContaining('Newer commit 0f9e8d7c')), findsOneWidget);
      final update = find.descendant(of: card, matching: find.byKey(const Key('overlay-update-review')));
      expect(find.descendant(of: update, matching: find.text('Update')), findsOneWidget);
      expect(find.descendant(of: update, matching: find.byIcon(Icons.arrow_circle_up)), findsOneWidget);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/overlay_update_detail_$label.png'));
    });

    testWidgets('a Discover tile of an installed overlay with an update shows the update icon in place of the check at $label width',
        (tester) async {
      await installedWith(tester, size, _newer, index: label == 'desktop');
      await tapKey(tester, 'overlays-discover');
      expect(find.byKey(const Key('store-entry-install-main-review')), findsNothing);
      final button = find.byKey(const Key('store-entry-update-main-review'));
      expect(button, findsOneWidget);
      expect(find.descendant(of: button, matching: find.byIcon(Icons.arrow_circle_up)), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('store-entry-main-review')), matching: find.byKey(const Key('update-available'))),
          findsOneWidget);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/overlay_update_discover_$label.png'));
    });
  }

  testWidgets('an overlay with nothing newer shows its Check button, a check on its tile and no badge anywhere', (tester) async {
    await installedWith(tester, const Size(1280, 900), const OverlayAvailability(state: 'current', installed: 'a1b2c3d4e5f6'), index: true);
    expect(find.text('Every overlay is up to date.'), findsOneWidget);
    expect(find.byKey(const Key('store-index-overlays-updates')), findsNothing);
    expect(find.byKey(const Key('overlay-update-badge-review')), findsNothing);
    await tapKey(tester, 'overlay-installed-review');
    final card = find.byKey(const Key('overlay-detail-review'));
    expect(find.descendant(of: card, matching: find.byKey(const Key('overlay-update-badge-review'))), findsNothing);
    expect(find.descendant(of: card, matching: find.text('Check for updates…')), findsOneWidget);
    await tester.pageBack();
    await tester.pumpAndSettle();
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(const Key('store-entry-update-main-review')), findsNothing);
    expect(find.descendant(of: find.byKey(const Key('store-entry-main-review')), matching: find.byIcon(Icons.check)), findsOneWidget);
  });

  testWidgets('the Store index counts the overlays that have an update, and the tabs of a Store on its own do too', (tester) async {
    await installedWith(tester, const Size(1280, 800), _newer, index: true);
    final count = find.byKey(const Key('store-index-overlays-updates'));
    expect(count, findsOneWidget);
    expect(find.descendant(of: count, matching: find.text('1')), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();

    await open(tester, const Size(1280, 800), using: saved.last);
    expect(find.descendant(of: find.byKey(const Key('overlays-updates-badge')), matching: find.text('1')), findsOneWidget);
  });

  testWidgets('Check for updates asks every source now, and what the host then says is shown', (tester) async {
    await installedWith(tester, const Size(1280, 900), const OverlayAvailability(state: 'current', installed: 'a1b2c3d4e5f6'));
    expect(find.text('1 overlay has an update.'), findsNothing);
    final before = overlays.forced;
    overlays.availability['review'] = _newer;
    await tapKey(tester, 'overlays-check-updates');
    expect(overlays.forced, before + 1, reason: 'the button asks with refresh');
    expect(find.text('1 overlay has an update.'), findsOneWidget);
    expect(find.byKey(const Key('overlays-updates-badge')), findsOneWidget);
  });

  testWidgets('an overlay whose source could not be asked says why and offers no update', (tester) async {
    await installedWith(
      tester,
      const Size(1280, 900),
      const OverlayAvailability(state: 'unreachable', installed: 'a1b2c3d4e5f6', problem: 'the host is down'),
    );
    await tapKey(tester, 'overlay-installed-review');
    final card = find.byKey(const Key('overlay-detail-review'));
    expect(find.descendant(of: card, matching: find.textContaining('Its source could not be asked: the host is down')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.byKey(const Key('overlay-update-badge-review'))), findsNothing);
  });

  testWidgets('Update shows the plan an install shows, from and to, and applying it clears the badge', (tester) async {
    await installedWith(tester, const Size(1280, 900), _newer);
    final installed = overlays.records['review']!;
    overlays.put(teamRepo, {
      'overlay.cfg': 'name = "review"\n[about]\nsummary = "Review conventions, newer."\n',
      'skills/review/SKILL.md': '# Review\n',
    });
    await tapKey(tester, 'overlay-installed-review');
    await tapKey(tester, 'overlay-update-review');
    expect(find.text('Sync review?'), findsOneWidget);
    expect(find.textContaining('From ${installed.commit}'), findsOneWidget);
    expect(find.byKey(const Key('overlay-apply')), findsOneWidget);
    await tester.tap(find.byKey(const Key('overlay-apply')));
    await tester.pumpAndSettle();
    expect(overlays.records['review']!.commit, isNot(installed.commit), reason: 'the newer tree was installed');
    expect(find.byKey(const Key('overlay-update-badge-review')), findsNothing, reason: 'the host no longer lists an update');
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('overlays-updates-badge')), findsNothing);
    expect(saved.last.overlays['review'], isNotNull);
  });
}
