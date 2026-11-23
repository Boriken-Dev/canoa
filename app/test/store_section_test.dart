// The Store by type: Agents, Overlays, Skills and MCPs behind one toolbar, each with Installed and Discover, a search that
// narrows both, the line that says how old the index is, and details that keep the query, the type and the place in the list.
// Catalogs come from a stand-in index over in-memory repositories; the programs from a stand-in host.
@Timeout(Duration(minutes: 3))
library;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_index.dart';
import 'package:canoa/store/store_section.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'fixtures/store_link.dart';
import 'loadout_test.dart' show catalogRepo, withCatalog, world;

/// What the index adds to [catalog] when the host has read it: how old it is, and its entries' manifests.
LoadedCatalog _aged(LoadedCatalog catalog, {required DateTime checked, bool stale = false, String? because}) => LoadedCatalog(
  label: catalog.label,
  source: catalog.source,
  commit: catalog.commit,
  catalog: catalog.catalog,
  problem: catalog.problem,
  icons: catalog.icons,
  checked: checked,
  stale: stale,
  staleBecause: because,
  facts: {
    'overlay:review': const EntryFacts(
      id: 'main:overlay:review',
      hasContents: true,
      skills: ['reviewing'],
      hooks: ['PreToolUse'],
      envScripts: ['setup.sh'],
      mcpServers: ['tracker'],
      requirements: ['git'],
    ),
    'skill:pdf': const EntryFacts(id: 'main:skill:pdf', hasContents: false),
  },
);

FakeStoreHost _programs() => FakeStoreHost([
  FakeStore(
    'one_store',
    title: 'Program One',
    serves: [
      {'provider': 'one', 'when': <String, Object?>{}},
    ],
    copies: [copyOf('2.1.322')],
    latest: '2.1.330',
  ),
  FakeStore(
    'two_store',
    title: 'Program Two',
    serves: [
      {'provider': 'stdio', 'when': {'harness': 'two'}},
    ],
    latest: '0.167.1',
  ),
]);

void main() {
  late MemoryOverlays overlays;
  final clock = DateTime(2026, 11, 16, 12);
  var asks = <bool>[];

  Future<void> open(
    WidgetTester tester, {
    FakeStoreHost? programs,
    DateTime? checked,
    bool stale = false,
    String? because,
    String text = withCatalog,
    CatalogIndex? index,
    Size size = const Size(1280, 1600),
  }) async {
    await tester.binding.setSurfaceSize(size);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final config = CanoaConfig.parse(text);
    await tester.pumpWidget(MaterialApp(
      home: StoreSection(
        readAhead: false,
        config: config,
        home: MemoryHome(overlays),
        now: () => clock,
        programs: programs?.programs(),
        index: index ??
            ({bool refresh = false}) async {
              asks.add(refresh);
              return [
                for (final catalog in await standInCatalogs(overlays, config))
                  _aged(catalog, checked: checked ?? clock.subtract(const Duration(minutes: 3)), stale: stale, because: because),
              ];
            },
        agents: const ['Claude'],
        catalogTypes: standInCatalogTypes,
        onSave: (changed) async => changed,
        readCatalog: standInCatalogRead(overlays, () => config, registry: (uri) async => '{"servers": []}'),
      ),
    ));
    await tester.pumpAndSettle();
  }

  Future<void> tapKey(WidgetTester tester, String key) async {
    await tester.ensureVisible(find.byKey(Key(key)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key(key)));
    await tester.pumpAndSettle();
  }

  String statusText(WidgetTester tester) => tester.widget<Text>(find.byKey(const Key('store-status'))).data!;

  setUp(() {
    overlays = world();
    asks = [];
  });

  testWidgets('the types are Overlays, Skills, Agents and MCP servers, and Agents only where the host has programs', (tester) async {
    await open(tester, programs: _programs());
    for (final id in const ['agents', 'overlays', 'skills', 'mcps']) {
      expect(find.byKey(Key('store-tab-$id')), findsOneWidget, reason: id);
    }
    expect(tester.takeException(), isNull);
    await open(tester);
    expect(find.byKey(const Key('store-tab-agents')), findsNothing);
    for (final id in const ['overlays', 'skills', 'mcps']) {
      expect(find.byKey(Key('store-tab-$id')), findsOneWidget, reason: id);
    }
  });

  testWidgets('Agents: the programs found are Installed, the others Discover, a search narrows both, updates are a badge and a filter',
      (tester) async {
    await open(tester, programs: _programs());
    await tapKey(tester, 'store-tab-agents');
    expect(find.byKey(const Key('program-one_store')), findsOneWidget);
    expect(find.byKey(const Key('program-two_store')), findsNothing);
    expect(find.byKey(const Key('store-updates-badge')), findsOneWidget);
    expect(find.byKey(const Key('agents-updates-filter')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('store-search')), 'nothing like it');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('program-one_store')), findsNothing);
    expect(find.byKey(const Key('agents-nothing')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('store-search')), 'one');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('program-one_store')), findsOneWidget);
    await tapKey(tester, 'agents-updates-filter');
    expect(find.byKey(const Key('program-one_store')), findsOneWidget, reason: 'it is the one with a newer version');
    await tapKey(tester, 'agents-discover');
    expect(find.byKey(const Key('program-two_store')), findsNothing, reason: 'the query "one" is kept for Discover');
    await tester.enterText(find.byKey(const Key('store-search')), 'two');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('program-two_store')), findsOneWidget);
    expect(find.byKey(const Key('program-install-two_store')), findsOneWidget);
  });

  testWidgets('Overlays: Discover lists the catalogs\' entries as tiles that say what they need, and a search narrows them', (tester) async {
    await open(tester);
    await tapKey(tester, 'overlays-discover');
    final card = find.byKey(const Key('store-entry-main-review'));
    expect(card, findsOneWidget);
    expect(find.descendant(of: card, matching: find.text('Needs git')), findsOneWidget, reason: 'compatibility, from the manifest');
    expect(find.descendant(of: card, matching: find.byKey(const Key('store-entry-install-main-review'))), findsOneWidget, reason: 'the +');
    await tester.enterText(find.byKey(const Key('store-search')), 'zzz');
    await tester.pumpAndSettle();
    expect(card, findsNothing);
    expect(find.byKey(const Key('overlays-nothing')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('store-search')), 'conventions');
    await tester.pumpAndSettle();
    expect(card, findsOneWidget);
  });

  testWidgets('Skills: Installed says none yet, and Discover offers no skill an overlay catalog names', (tester) async {
    await open(tester);
    await tapKey(tester, 'store-tab-skills');
    expect(find.byKey(const Key('skills-none')), findsOneWidget);
    await tapKey(tester, 'skills-discover');
    expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing);
    expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
    expect(find.byKey(const Key('skills-source-anthropics')), findsOneWidget);
  });

  testWidgets('MCPs: Installed says none yet, and Discover asks the registry', (tester) async {
    await open(tester);
    await tapKey(tester, 'store-tab-mcps');
    expect(find.byKey(const Key('mcps-none')), findsOneWidget);
    await tapKey(tester, 'mcps-discover');
    expect(find.byKey(const Key('mcps-nothing')), findsOneWidget, reason: 'the registry listed nothing');
  });

  testWidgets('a card opens its details, and coming back keeps the type, the query and the switch', (tester) async {
    await open(tester);
    await tapKey(tester, 'overlays-discover');
    await tester.enterText(find.byKey(const Key('store-search')), 'review');
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('store-entry-main-review')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('details-technical')), findsOneWidget);
    expect(find.byKey(const Key('store-skill-review')), findsNothing, reason: 'looking is not installing');
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-entry-main-review')), findsOneWidget);
    expect(tester.widget<TextField>(find.byKey(const Key('store-search'))).controller!.text, 'review');
    expect(find.byKey(const Key('overlays-discover')), findsOneWidget);
    expect(tester.widget<SegmentedButton<bool>>(find.byKey(const Key('overlays-switch'))).selected, {true});
  });

  testWidgets('an overlay\'s page shows what the index read of its manifest before anything is fetched', (tester) async {
    await open(tester);
    await tapKey(tester, 'overlays-discover');
    await tester.tap(find.byKey(const Key('store-entry-main-review')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('details-skills')), findsOneWidget);
    expect(find.byKey(const Key('details-mcp-servers')), findsOneWidget);
    expect(find.byKey(const Key('details-requirement')), findsOneWidget);
    expect(find.byKey(const Key('details-hooks')), findsOneWidget);
    expect(find.byKey(const Key('details-env-scripts')), findsOneWidget);
    expect(find.byKey(const Key('details-carried-off')), findsOneWidget, reason: 'they say what stays off');
    expect(find.byKey(const Key('store-fetch')), findsOneWidget, reason: 'nothing was fetched to show this');
    expect(overlays.records, isEmpty);
  });

  testWidgets('the line under the toolbar says how old the index is', (tester) async {
    await open(tester);
    expect(statusText(tester), 'Cached · updated 3 min ago');
  });

  testWidgets('a stale index says so, is still listed, and the source says why it could not be refreshed', (tester) async {
    await open(tester, checked: clock.subtract(const Duration(hours: 2)), stale: true, because: 'it cannot be fetched: offline');
    expect(statusText(tester), 'Stale · updated 2 h ago');
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(const Key('store-entry-main-review')), findsOneWidget, reason: 'the last copy is shown');
    expect(find.textContaining('showing the last copy'), findsOneWidget);
    expect(find.textContaining('offline'), findsOneWidget);
  });

  testWidgets('a catalog the host could not read has its reason shown and lists nothing', (tester) async {
    await open(
      tester,
      index: ({bool refresh = false}) async => [
        LoadedCatalog(label: 'main', source: OverlaySource.parseGit('$catalogRepo@main#catalog.toml'), problem: 'It cannot be fetched: gone'),
      ],
    );
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(const Key('store-source-note-main')), findsOneWidget);
    expect(find.textContaining('main: It cannot be fetched: gone'), findsOneWidget);
    expect(find.byKey(const Key('overlays-nothing')), findsOneWidget);
  });

  testWidgets('an index that fails leaves the Store standing and says why', (tester) async {
    await open(tester, index: ({bool refresh = false}) async => throw StateError('the host went away'));
    expect(find.byKey(const Key('store-tabs')), findsOneWidget);
    expect(find.textContaining('the host went away'), findsOneWidget);
  });

  testWidgets('Refresh asks the host to look again', (tester) async {
    await open(tester);
    expect(asks, [false]);
    await tapKey(tester, 'store-refresh');
    expect(asks, [false, true]);
  });

  testWidgets('Store settings holds the catalogs: listed, with what each said, and removable', (tester) async {
    await open(tester);
    await tapKey(tester, 'store-settings');
    expect(find.byKey(const Key('store-settings-page')), findsOneWidget);
    expect(find.byKey(const Key('store-catalog-main')), findsOneWidget);
    expect(find.byKey(const Key('store-remove-catalog-main')), findsOneWidget);
    expect(find.byKey(const Key('store-add-catalog')), findsOneWidget);
  });
}
