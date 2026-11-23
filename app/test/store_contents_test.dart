// What every entry of a catalog provides, read ahead (by a stand-in for the host, whose own reading canoa-host tests): kept by catalog under the cache with a time to live; read again on a forced refresh; one entry failing leaves the rest.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/store/contents_storage.dart';
import 'package:canoa/store/contents_store.dart';
import 'package:canoa/store/store.dart' show LoadedCatalog;
import 'package:canoa/store/store_section.dart';
import 'package:canoa/theme/look.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/contents_reader.dart';
import 'fixtures/store_fixtures.dart';

const _repo = 'https://example.invalid/team.git';

const _catalog = '''
name = "Team"

[overlays.full]
source = "./overlays/full"
[overlays.full.about]
summary = "Everything."

[overlays.bare]
source = "./overlays/bare"

[overlays.broken]
source = "https://example.invalid/gone.git@main"

[skills.pdf]
source = "https://example.invalid/skills.git@main#pdf"
''';

const _manifest = '''
name = "full"
requires = ["lint"]

[knowledge]
serve = true

[[mcp.servers]]
name = "tracker"
transport = "stdio"
command = "tracker-mcp"

[[env_scripts]]
interpreter = "sh"
path = "scripts/env.sh"
''';

const _skill = '---\nname: review\ndescription: Reviews a diff against the team rules.\n---\nBody.\n';

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/store_contents_test.dart')));

  group('ContentsStore', () {
    final config = CanoaConfig.parse('[store.overlays.catalogs]\nteam = "$_repo@main#catalog.toml"\n');
    late MemoryOverlays overlays;
    late MemoryContentsStorage storage;
    late DateTime clock;

    setUp(() {
      overlays = MemoryOverlays()
        ..put(_repo, {
          'catalog.toml': _catalog,
          'overlays/full/overlay.cfg': _manifest,
          'overlays/full/skills/review/SKILL.md': _skill,
          'overlays/bare/pdf/SKILL.md': _skill,
        })
        ..putSkill('https://example.invalid/skills.git@main#pdf', {'SKILL.md': _skill});
      storage = MemoryContentsStorage();
      clock = DateTime(2026, 11, 16, 12);
    });

    ContentsStore make() => ContentsStore(reader: StandInContents(overlays), storage: storage, now: () => clock);
    const ttl = Duration(minutes: 5);

    test('a refresh fills every entry, a generated overlay.cfg for a source without one, and a failing entry stops no other', () async {
      final catalogs = await standInCatalogs(overlays, config);
      final store = make();
      await store.readAhead(catalogs, ttl: ttl);
      final full = store.of(contentsId('team', 'overlay', 'full'))!;
      expect(full.contents!.skills.single.name, 'review');
      expect(full.contents!.mcpServers, ['tracker (stdio)']);
      expect(store.of(contentsId('team', 'overlay', 'bare'))!.contents!.generated, isTrue);
      expect(store.of(contentsId('team', 'overlay', 'broken'))!.problem, isNotNull);
      expect(store.of(contentsId('team', 'overlay', 'broken'))!.contents, isNull);
      expect(store.of(contentsId('team', 'skill', 'pdf'))!.contents!.skills.single.description, startsWith('Reviews'));
      expect(overlays.records, isEmpty, reason: 'reading installs nothing');
      expect(overlays.discarded, containsAll(['full', 'bare']), reason: 'what was fetched to be read is removed');
    });

    /// The catalogs, as the host's index gives them: at a commit.
    Future<List<LoadedCatalog>> atCommit(String? commit) async => [
      for (final each in await standInCatalogs(overlays, config)) LoadedCatalog(label: each.label, source: each.source, commit: commit, catalog: each.catalog),
    ];

    test('a catalog at the commit it was read at reads nothing again, whatever the time; a new commit and a forced refresh read again', () async {
      final catalogs = await atCommit('a' * 40);
      final store = make();
      await store.readAhead(catalogs, ttl: ttl);
      final first = store.fetches;
      expect(first, greaterThan(0));
      await store.readAhead(catalogs, ttl: ttl);
      expect(store.fetches, first, reason: 'every reading is current');
      clock = clock.add(const Duration(hours: 3));
      await store.readAhead(catalogs, ttl: ttl);
      expect(store.fetches, first, reason: 'the same commit has the same entries, so time alone reads nothing');
      await store.readAhead(await atCommit('b' * 40), ttl: ttl);
      expect(store.fetches, greaterThan(first), reason: 'the catalog moved, so what it lists may have');
      final before = store.fetches;
      await store.readAhead(catalogs, ttl: ttl, force: true);
      expect(store.fetches - before, first, reason: 'the refresh button reads everything');
    });

    test('what was read is kept under the cache by catalog, so a new run reads nothing', () async {
      final catalogs = await standInCatalogs(overlays, config);
      await make().readAhead(catalogs, ttl: ttl);
      expect(storage.kept.keys, ['team']);
      overlays.fetched.clear();
      final again = make();
      await again.readAhead(catalogs, ttl: ttl);
      expect(again.fetches, 0);
      expect(again.of(contentsId('team', 'overlay', 'full'))!.contents!.skills.single.name, 'review');
    });

    test('a catalog with no commit has the time to live', () async {
      final catalogs = await atCommit(null);
      final store = make();
      await store.readAhead(catalogs, ttl: ttl);
      final first = store.fetches;
      clock = clock.add(const Duration(minutes: 4));
      await store.readAhead(catalogs, ttl: ttl);
      expect(store.fetches, first);
      clock = clock.add(const Duration(minutes: 2));
      await store.readAhead(catalogs, ttl: ttl);
      expect(store.fetches, greaterThan(first));
    });

    test('only the entries asked for are read, and the others stay unread', () async {
      final catalogs = await atCommit('c' * 40);
      final store = make();
      final full = contentsId('team', 'overlay', 'full');
      await store.readAhead(catalogs, ttl: ttl, only: {full});
      expect(store.fetches, 1);
      expect(store.of(full)!.contents!.skills.single.name, 'review');
      expect(store.of(contentsId('team', 'overlay', 'bare')), isNull);
      await store.readAhead(catalogs, ttl: ttl, only: {full});
      expect(store.fetches, 1, reason: 'an entry read at this commit is not read again');
      await store.readAhead(catalogs, ttl: ttl, only: {contentsId('team', 'overlay', 'bare')});
      expect(store.fetches, 2);
    });

    test('a zero time to live reads again every time', () async {
      final catalogs = await standInCatalogs(overlays, config);
      final store = make();
      await store.readAhead(catalogs, ttl: Duration.zero);
      final first = store.fetches;
      await store.readAhead(catalogs, ttl: Duration.zero);
      expect(store.fetches, first * 2);
    });
  });

  group('an entry page', () {
    final config = CanoaConfig.parse('[store.overlays.catalogs]\nteam = "$_repo@main#catalog.toml"\n');

    testWidgets('lists what the overlay provides once the catalog is read, and never asks for a fetch', (tester) async {
      final overlays = MemoryOverlays()
        ..put(_repo, {
          'catalog.toml': _catalog,
          'overlays/full/overlay.cfg': _manifest,
          'overlays/full/rules/style.md': 'Use tabs.',
          'overlays/full/skills/review/SKILL.md': _skill,
          'overlays/bare/pdf/SKILL.md': _skill,
        });
      final reader = StandInContents(overlays);
      await tester.binding.setSurfaceSize(const Size(1200, 1600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: StoreSection(config: config, home: MemoryHome(overlays), contentsReader: reader, index: standInIndex(overlays, () => config), onSave: (changed) async => changed),
      ));
      await tester.pumpAndSettle();
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('overlays-discover')));
      await tester.pumpAndSettle();
      expect(reader.batches, isEmpty, reason: 'listing the catalogs reads no entry');
      await tester.tap(find.byKey(const Key('store-entry-team-full')));
      // Opening the page reads that entry, and no other.
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
      await tester.pumpAndSettle();
      expect(reader.batches, [[contentsId('team', 'overlay', 'full')]], reason: 'the page read the entry opened and no other');
      expect(find.byKey(const Key('store-fetch')), findsOneWidget, reason: 'installing is reviewed, which is not reading');
      expect(find.byKey(const Key('details-contents-unread')), findsNothing);
      expect(find.byKey(const Key('details-skill-review')), findsOneWidget);
      expect(find.textContaining('Reviews a diff against the team rules.'), findsOneWidget);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_provides_desktop.png'));
      expect(find.byKey(const Key('details-mcp-servers')), findsOneWidget);
      expect(find.byKey(const Key('details-env-scripts')), findsOneWidget);
      expect(find.byKey(const Key('details-knowledge')), findsOneWidget);
      expect(find.textContaining('rules/style.md: served on request'), findsOneWidget);
    });
  });
}
