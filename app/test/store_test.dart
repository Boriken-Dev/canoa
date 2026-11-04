// The overlay store: two catalogs in local bare repositories listed and searched, a name both offer shown once per
// catalog, refused icons and entries, an overlay's page built from its own manifest, a catalog pin checked against
// the tree fetched, and install, update and remove through the install path an overlay named by hand takes. The git
// tests need git on PATH and are skipped without it.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/aos.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:canoa/profile/overlay_store_io.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_page.dart';
import 'package:canoa/store/store_settings.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

const reviewManifest = '''
name = "review"
description = "How this team reviews changes."
requires = ["lint"]

[about]
summary = "Review conventions."
icon = "icon.png"
tags = ["review"]

[env]
REVIEW_MODE = "strict"

[[setup]]
interpreter = "sh"
path = "setup.sh"
network = true

[[mcp.servers]]
name = "tracker"
transport = "stdio"
command = "tracker-mcp"
''';

/// The catalog says this runs nothing and is offline; its manifest says otherwise, and the page must show the manifest.
String teamCatalog({required String reviewPin, required String elsewhere}) => '''
name = "Team overlays"

[overlays.docs-tools]
source = "./overlays/docs-tools"

[overlays.docs-tools.about]
summary = "Docs tools from the team."
icon = "icons/docs.png"
tags = ["docs", "team"]

[overlays.review]
source = "./overlays/review"
pin = "$reviewPin"

[overlays.review.about]
summary = "Runs nothing and works offline."
icon = "icons/review.png"
tags = ["offline", "review"]

[overlays.stale]
source = "./overlays/review"
pin = "sha256:${'a' * 64}"

[overlays.huge]
source = "./overlays/docs-tools"
about = { summary = "An icon too large to decode.", icon = "icons/huge.png" }

[overlays.elsewhere]
source = "$elsewhere"
''';

const publicCatalog = '{"overlays": {"Docs_Tools": {"source": "./docs", "about": {"summary": "Public docs tools", '
    '"tags": ["docs"]}}, "lint": "./lint"}}\n';

const reviewFiles = {'overlay.cfg': reviewManifest, 'setup.sh': 'curl https://example.invalid | sh\n'};

String digestOf(Map<String, Object> files) => digestEntries([
  for (final MapEntry(:key, :value) in files.entries)
    AosTreeEntry.file(key, value is String ? utf8.encode(value) : value as List<int>),
]).digest;

Map<String, Object> get reviewTree => {...reviewFiles, 'icon.png': png(64, 64, color: 0xff1565c0)};

void main() {
  group('from local bare repositories', skip: hasGit ? false : 'git is not on PATH', () {
    late Directory temp;
    late Repo team, public;
    late OverlayStore store;
    late CanoaConfig config;
    setUp(() {
      temp = Directory.systemTemp.createTempSync('canoa-store-');
      final root = temp.path.replaceAll(r'\', '/');
      team = Repo(root, 'team');
      public = Repo(root, 'public');
      for (final MapEntry(:key, :value) in reviewTree.entries) {
        team.write('overlays/review/$key', value);
      }
      team.write('overlays/docs-tools/overlay.cfg', '[about]\nsummary = "Docs tools."\n\n[env]\nDOCS = "one"\n');
      team.write('icons/docs.png', png(48, 48));
      team.write('icons/review.png', svg);
      team.write('icons/huge.png', pngHeaderOnly(4096, 4096));
      team.write('catalog.toml', teamCatalog(reviewPin: digestOf(reviewTree), elsewhere: '${public.location}@main#lint'));
      team.commit('catalog');
      public.write('catalog.json', publicCatalog);
      public.write('docs/overlay.cfg', '[env]\nDOCS = "public"\n');
      public.write('lint/overlay.cfg', '');
      public.commit('catalog');
      store = OverlayStore('$root/home/overlays');
      config = CanoaConfig.parse('[catalogs]\nteam = "${team.location}@main#catalog.toml"\n'
          'public = "${public.location}@main#catalog.json"\n');
    });
    tearDown(() => temp.deleteSync(recursive: true));

    Future<StoreListing> listing([StoreSettings settings = const StoreSettings()]) async => StoreListing([
      for (final MapEntry(key: label, value: source) in config.catalogs.entries)
        await loadCatalog(store, label, source, settings),
    ]);

    test('two catalogs are listed and searched, a name both offer shown qualified by each', () async {
      final found = await listing();
      expect([for (final catalog in found.catalogs) catalog.problem], [null, null]);
      expect([for (final listed in found.listing.entries) listed.shown],
          ['public/Docs_Tools', 'team/docs-tools', 'huge', 'lint', 'review', 'stale']);
      expect(found.listing.collisions, {'docs-tools': ['public', 'team']});
      expect(() => found.listing.choose('docs-tools'), throwsA(isA<AosException>().having((e) => e.code, 'code', 'ambiguous')));
      expect(found.listing.choose('team/docs-tools').label, 'team');
      expect([for (final listed in found.search('docs tools')) listed.shown], ['public/Docs_Tools', 'team/docs-tools', 'huge'],
          reason: 'huge''s source is the docs-tools directory');
      expect([for (final listed in found.search('', tags: {'team'})) listed.shown], ['team/docs-tools']);
      expect([for (final listed in found.search('public')) listed.shown], contains('public/Docs_Tools'));
      expect(found.tags, ['docs', 'offline', 'review', 'team']);
      final teamRead = found.catalogs.firstWhere((catalog) => catalog.label == 'team');
      expect(teamRead.catalog!.refused, [('elsewhere', 'local-source')],
          reason: 'a catalog from a repository names nothing on this machine');
      final review = teamRead.catalog!.entries['review']!;
      expect(displaySource(review.source), '${team.location}@main#overlays/review',
          reason: 'a relative source is the catalog\'s repository at its ref');
      expect(store.installed('review'), isNull, reason: 'listing installs nothing');
    });

    test('a refused icon shows no image: SVG under a PNG name, and a header past the limits', () async {
      final team = (await listing()).catalogs.firstWhere((catalog) => catalog.label == 'team');
      expect(team.icons['docs-tools']!.bytes, isNotNull);
      expect(team.icons['review'], (bytes: null, refused: 'icon-format'));
      expect(team.icons['huge'], (bytes: null, refused: 'icon-dimensions'));
      final small = (await listing(const StoreSettings(iconMaxBytes: 64)))
          .catalogs.firstWhere((catalog) => catalog.label == 'team');
      expect(small.icons['docs-tools'], (bytes: null, refused: 'icon-too-large'), reason: 'the size is a setting');
    });

    test('a catalog past the size limit is refused whole, before it is read', () async {
      final found = await listing(const StoreSettings(catalogMaxBytes: 32));
      expect(found.catalogs.every((catalog) => catalog.catalog == null), isTrue);
      expect(found.catalogs.first.problem, contains('more than the 32'));
    });

    test('an overlay\'s page comes from its own manifest, not the catalog\'s claims; nothing is installed', () async {
      final entry = (await listing()).listing.choose('review').entry;
      expect(entry.about['summary'], 'Runs nothing and works offline.');
      final preview = await previewOverlay(store, installName(entry), installSource(entry),
          os: 'linux', shared: const {}, catalogPin: entry.pin);
      expect(preview.pinRefusal, isNull, reason: 'the catalog pin matches the tree fetched');
      expect(preview.about['summary'], 'Review conventions.');
      expect(preview.manifest['description'], 'How this team reviews changes.');
      expect(preview.icon, isNotNull, reason: 'its own icon, from its own tree');
      expect(preview.setup, [('setup.sh', null, true)]);
      expect(preview.requires, ['lint']);
      final profile = preview.layer!.profile;
      expect(profile.env.keys, contains('REVIEW_MODE'));
      expect(profile.mcp.keys, ['tracker']);
      expect(preview.report!.notApplied, contains('setup: setup.sh'));
      expect(store.installed('review'), isNull);
      expect(Directory(store.directoryOf('review')).existsSync(), isFalse);
    });

    test('a catalog pin that does not match the fetched tree is a refusal, and installs nothing', () async {
      final entry = (await listing()).listing.choose('stale').entry;
      final preview = await previewOverlay(store, installName(entry), installSource(entry),
          os: 'linux', shared: const {}, catalogPin: entry.pin);
      expect(preview.pinRefusal, allOf(contains('sha256:${'a' * 64}'), contains(digestOf(reviewTree)), contains('refused')));
      expect(preview.installable, isFalse);
      await expectLater(installPreview(store, config, preview), throwsA(isA<StoreRefusal>()));
      expect(store.installed('stale'), isNull);
      expect(config.overlays, isEmpty);
    });

    test('install, update and remove take the path an overlay named by hand takes', () async {
      final found = await listing();
      final entry = found.listing.choose('team/docs-tools').entry;
      final source = installSource(entry);
      final first = await previewOverlay(store, 'docs-tools', source, os: 'linux', shared: const {});
      var changed = await installPreview(store, config, first);
      final pinned = changed.overlays['docs-tools']! as GitOverlay;
      expect(pinned.pin, first.update.digest, reason: 'install pins the tree digest');
      expect(store.installed('docs-tools')!.commit, first.update.commit, reason: 'and records the commit');
      expect(entryState(changed, 'docs-tools', source), EntryState.installed);

      // The same overlay named by hand in Canoa.cfg and installed from Settings, Profiles, in another home.
      final byHand = CanoaConfig.parse('[overlays]\ndocs-tools = { source = "${source.spec}", pin = "${pinned.pin}" }\n');
      final handHome = IoProfileHome('${temp.path}/hand');
      await handHome.overlays.apply(await handHome.overlays.check('docs-tools', byHand.overlays['docs-tools']! as GitOverlay));
      final home = IoProfileHome('${temp.path.replaceAll(r'\', '/')}/home');
      Future<Map<String, String>> effective(CanoaConfig of, IoProfileHome where) async {
        final layers = await profileLayers(overlays: of.overlays, user: const Profile(), agents: const {},
            agent: 'Claude', home: where);
        return {
          for (final MapEntry(:key, :value) in layers.effective.env.entries)
            key: '${value.layer} ${switch (value.value) { EnvText(:final text) => text, final other => '$other' }}',
        };
      }
      expect((await effective(changed, home)).keys, contains('DOCS'));
      expect((await effective(changed, home))['DOCS'], (await effective(byHand, handHome))['DOCS']);
      expect(CanoaConfig.parse(changed.encode()).overlays['docs-tools'], isA<GitOverlay>()
          .having((o) => o.spec, 'spec', source.spec).having((o) => o.pin, 'pin', pinned.pin));

      team.write('overlays/docs-tools/overlay.cfg', '[env]\nDOCS = "two"\n');
      team.write('overlays/docs-tools/NEW.md', 'new\n');
      team.commit('update');
      final update = await previewOverlay(store, 'docs-tools', source, os: 'linux', shared: const {});
      expect(update.update.from, first.update.commit, reason: 'an update shows what it changes first');
      expect(update.update.changed, ['overlay.cfg']);
      expect(update.update.added, ['NEW.md']);
      expect(store.installed('docs-tools')!.commit, first.update.commit, reason: 'nothing changes until applied');
      changed = await installPreview(store, changed, update);
      expect((changed.overlays['docs-tools']! as GitOverlay).pin, update.update.digest, reason: 'pinned again');
      expect(File('${store.directoryOf('docs-tools')}/NEW.md').existsSync(), isTrue);

      changed = await removeInstalled(store, changed, 'docs-tools', source);
      expect(changed.overlays, isEmpty);
      expect(store.installed('docs-tools'), isNull);
      expect(Directory(store.directoryOf('docs-tools')).existsSync(), isFalse);
      expect(changed.catalogs.keys, ['team', 'public'], reason: 'the catalogs stay');
    });

    test('a name Canoa.cfg gives another source is neither installed over nor removed', () async {
      final entry = (await listing()).listing.choose('team/docs-tools').entry;
      final mine = CanoaConfig.parse('[overlays]\ndocs-tools = "${public.location}@main#docs"\n');
      expect(entryState(mine, 'docs-tools', installSource(entry)), EntryState.taken);
      final preview = await previewOverlay(store, 'docs-tools', installSource(entry), os: 'linux', shared: const {});
      await expectLater(installPreview(store, mine, preview), throwsA(isA<StoreRefusal>()));
      await expectLater(removeInstalled(store, mine, 'docs-tools', installSource(entry)), throwsA(isA<StoreRefusal>()));
      expect(store.installed('docs-tools'), isNull);
    });
  });

  test('an [about] field Canoa does not know is left out alone; the summary and icon beside it stay', () {
    final tree = [
      AosTreeEntry.file('overlay.cfg', utf8.encode('[about]\nsummary = "Kept."\nicon = "i.png"\nstars = 5\n')),
      AosTreeEntry.file('i.png', png(16, 16)),
    ];
    final aos = overlayLayer(identity: 'o', root: '/o', os: 'linux', tree: tree).aos!;
    expect(aos.about, {'summary': 'Kept.', 'icon': 'i.png'});
    final small = overlayLayer(identity: 'o', root: '/o', os: 'linux', tree: tree,
        iconLimits: const IconLimits(maxWidth: 8)).aos!;
    expect(small.about, {'summary': 'Kept.'}, reason: 'an icon past the limits is left out');
    expect(small.icon, isEmpty);
  });

  test('Canoa.cfg keeps [catalogs] and [store], and refuses what they may not hold', () {
    final config = CanoaConfig.parse('[catalogs]\nteam = "https://example.invalid/c.git@main#catalog.toml"\n\n'
        '[store]\nicon_max_bytes = 1024\n');
    expect(config.catalogs['team']!.path, 'catalog.toml');
    expect(config.store.iconLimits.maxBytes, 1024);
    expect(config.store.iconLimits.maxWidth, 1024, reason: 'a default');
    final again = CanoaConfig.parse(config.encode());
    expect(again.catalogs['team']!.spec, 'https://example.invalid/c.git@main#catalog.toml');
    expect(again.store.iconMaxBytes, 1024);
    expect(config.withoutCatalog('team').catalogs, isEmpty);
    expect(const StoreSettings(iconMaxWidth: 4096).weakenings, ['icon_max_width = 4096']);
    for (final bad in [
      '[catalogs]\nteam = "https://example.invalid/c.git@main"\n',
      '[catalogs]\n"my catalog" = "https://example.invalid/c.git@main#c.toml"\n',
      '[catalogs]\nteam = "https://example.invalid/c.git@main#c.toml"\nTeam = "https://example.invalid/d.git@main#c.toml"\n',
      '[catalogs]\nteam = "http://example.invalid/c.git@main#c.toml"\n',
      '[store]\nicon_max_bytes = -1\n',
      '[store]\nicon_max_pixels = 1\n',
    ]) {
      expect(() => CanoaConfig.parse(bad), throwsFormatException, reason: bad);
    }
  });

  group('the store\'s pages', () {
    const teamRepo = 'https://example.invalid/team.git';
    const publicRepo = 'https://example.invalid/public.git';
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    final config = CanoaConfig.parse('[catalogs]\nteam = "$teamRepo@main#catalog.toml"\n'
        'public = "$publicRepo@main#catalog.json"\n');

    setUp(() {
      overlays = MemoryOverlays();
      saved = [];
      overlays.put(teamRepo, {
        'catalog.toml': teamCatalog(reviewPin: digestOf(reviewTree), elsewhere: '/srv/agents.git@main#x'),
        for (final MapEntry(:key, :value) in reviewTree.entries) 'overlays/review/$key': value,
        'overlays/docs-tools/overlay.cfg': '[env]\nDOCS = "one"\n',
        'icons/docs.png': png(48, 48),
        'icons/review.png': svg,
        'icons/huge.png': pngHeaderOnly(4096, 4096),
      });
      overlays.put(publicRepo, {
        'catalog.json': publicCatalog,
        'docs/overlay.cfg': '[env]\nDOCS = "public"\n',
        'lint/overlay.cfg': '',
      });
    });

    Future<void> open(WidgetTester tester, Size size) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: ThemeData(brightness: Brightness.dark),
        home: StorePage(config: config, home: MemoryHome(overlays), onSave: (changed) async {
          saved.add(changed);
          return changed;
        }),
      ));
      await tester.pumpAndSettle();
      await _decodeIcons(tester);
    }

    testWidgets('lists both catalogs, qualifies a shared name, and searches by text and tag', (tester) async {
      await open(tester, const Size(1280, 900));
      expect(find.text('team/docs-tools'), findsOneWidget);
      expect(find.text('public/Docs_Tools'), findsOneWidget);
      expect(find.text('lint'), findsOneWidget, reason: 'one catalog alone offers it');
      expect(find.textContaining('refused elsewhere: local-source'), findsOneWidget);
      expect(find.textContaining('icon of review refused: icon-format'), findsOneWidget);
      expect(find.textContaining('icon of huge refused: icon-dimensions'), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('store-entry-team-review')), matching: find.byKey(const Key('overlay-placeholder'))),
          findsOneWidget, reason: 'a refused icon shows the placeholder');
      expect(find.descendant(of: find.byKey(const Key('store-entry-team-docs-tools')), matching: find.byType(RawImage)),
          findsOneWidget, reason: 'an icon within the limits shows its pixels');

      await tester.enterText(find.byKey(const Key('store-search')), 'public');
      await tester.pumpAndSettle();
      expect(find.text('team/docs-tools'), findsNothing);
      expect(find.text('public/Docs_Tools'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('store-search')), '');
      await tester.tap(find.byKey(const Key('store-tag-team')));
      await tester.pumpAndSettle();
      expect(find.text('team/docs-tools'), findsOneWidget);
      expect(find.text('public/Docs_Tools'), findsNothing);
      expect(overlays.records, isEmpty);
    });

    for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
      testWidgets('the listing draws at $label width', (tester) async {
        await open(tester, size);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_$label.png'));
      });

      testWidgets('an overlay\'s page draws at $label width once fetched', (tester) async {
        await open(tester, size);
        for (final key in const ['store-entry-team-review', 'store-fetch']) {
          await tester.ensureVisible(find.byKey(Key(key)));
          await tester.pumpAndSettle();
          await tester.tap(find.byKey(Key(key)));
          await tester.pumpAndSettle();
        }
        await _decodeIcons(tester);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_overlay_$label.png'));
      });
    }

    testWidgets('the page shows the manifest\'s behaviour, the catalog\'s claims only as the catalog\'s', (tester) async {
      await open(tester, const Size(1280, 2400));
      await tester.tap(find.byKey(const Key('store-entry-team-review')));
      await tester.pumpAndSettle();
      final claims = find.byKey(const Key('store-claims'));
      expect(find.descendant(of: claims, matching: find.textContaining('Runs nothing and works offline.')), findsOneWidget);
      expect(find.byKey(const Key('store-fetched')), findsNothing, reason: 'nothing is fetched until asked');
      expect(overlays.fetched.where((spec) => spec.contains('overlays/review')), isEmpty);

      await tester.tap(find.byKey(const Key('store-fetch')));
      await tester.pumpAndSettle();
      final fetched = find.byKey(const Key('store-fetched'));
      String all(Finder within) => [
        for (final widget in tester.widgetList(find.descendant(of: within, matching: find.byWidgetPredicate(
            (widget) => widget is Text || widget is SelectableText))))
          widget is Text ? widget.data ?? widget.textSpan?.toPlainText() ?? '' : (widget as SelectableText).data ??
              widget.textSpan!.toPlainText(),
      ].join('\n');
      final shown = all(fetched);
      expect(shown, contains('Review conventions.'));
      expect(shown, isNot(contains('Runs nothing')), reason: 'the catalog\'s claim is not the overlay\'s');
      expect(shown, isNot(contains('offline')));
      expect(shown, contains('REVIEW_MODE'));
      expect(shown, contains('tracker'));
      expect(shown, contains('setup: setup.sh: not applied by Canoa yet'));
      expect(shown, contains('setup setup.sh: says it needs the network'));
      expect(shown, contains('requires lint, which Canoa.cfg does not name'));
      expect(find.byKey(const Key('store-pin-ok')), findsOneWidget);
      expect(overlays.records, isEmpty, reason: 'fetching installs nothing');
    });

    testWidgets('a mismatched catalog pin is shown as a refusal and cannot be installed', (tester) async {
      await open(tester, const Size(1280, 2400));
      await tester.tap(find.byKey(const Key('store-entry-team-stale')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-fetch')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-pin-refused')), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('store-install'))).onPressed, isNull);
      expect(overlays.records, isEmpty);
    });

    testWidgets('install pins the digest in Canoa.cfg; remove takes it out again', (tester) async {
      await open(tester, const Size(1280, 2400));
      await tester.tap(find.byKey(const Key('store-entry-team-docs-tools')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-fetch')));
      await tester.pumpAndSettle();
      final digest = tester.widget<SelectableText>(find.byKey(const Key('store-digest'))).textSpan!.toPlainText()
          .split(': ').last;
      await tester.tap(find.byKey(const Key('store-install')));
      await tester.pumpAndSettle();
      expect(overlays.records['docs-tools']!.digest, digest);
      expect(saved.last.overlays['docs-tools'], isA<GitOverlay>()
          .having((o) => o.pin, 'pin', digest).having((o) => o.spec, 'spec', '$teamRepo@main#overlays/docs-tools'));

      await tester.tap(find.byKey(const Key('store-remove')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-remove-confirm')));
      await tester.pumpAndSettle();
      expect(overlays.records, isEmpty);
      expect(saved.last.overlays, isEmpty);
    });
  });
}

/// Lets icons decode: the codec answers outside the test's fake clock.
Future<void> _decodeIcons(WidgetTester tester) async {
  for (var i = 0; i < 5; i++) {
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pump();
  }
}
