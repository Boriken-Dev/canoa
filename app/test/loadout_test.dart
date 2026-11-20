// Loadout: the page with Overlays, Skills and MCPs, each with what is installed and a place to discover more. Overlays and skills
// come from catalogs in memory, MCP servers from a registry's answer written here; nothing touches the person's files.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/store/overlay_needs.dart';
import 'package:canoa/store/programs.dart' show ProgramStores;
import 'package:canoa/store/store_section.dart';
import 'package:canoa/loadout/mcp_registry.dart' show RegistryRefusal;
import 'package:canoa/loadout/loadout_controller.dart' show knownCatalogs;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'fixtures/store_link.dart';
import 'mcp_registry_test.dart' show npmServer, page;
import 'skills_from_source_test.dart' show pdfFiles, pinOf, skillMd, skillsHost;

const catalogRepo = 'https://example.invalid/catalog.git';
const teamRepo = 'https://example.invalid/review.git';

const reviewCfg = '''
name = "review"
[about]
summary = "Review conventions."
tags = ["review"]

[env]
REVIEW_MODE = "strict"

[[mcp.servers]]
name = "tracker"
transport = "stdio"
command = "tracker-mcp"
''';

final catalog = '''
name = "Team"

[overlays.review]
source = "$teamRepo@main"
about = { summary = "Review conventions.", tags = ["review"] }

[skills]
pdf = { source = "$skillsHost/pdf.git", pin = "${pinOf('pdf', pdfFiles)}" }
''';

MemoryOverlays world() => MemoryOverlays()
  ..put(catalogRepo, {'catalog.toml': catalog})
  ..put(teamRepo, {'overlay.cfg': reviewCfg})
  ..putSkill('$skillsHost/pdf.git', pdfFiles);

const withCatalog = '[store.overlays.catalogs]\nboriken = false\nmain = "$catalogRepo@main#catalog.toml"\n';

final registryAnswer = page([
  npmServer(),
  {
    'name': 'io.github.ana/bundle',
    'description': 'A bundle.',
    'version': '1.0.0',
    'packages': [
      {'registryType': 'mcpb', 'identifier': 'https://x.test/a.mcpb'},
    ],
  },
]);

void main() {
  late MemoryOverlays overlays;
  late List<CanoaConfig> saved;
  late CanoaConfig config;

  Future<void> open(WidgetTester tester, {String text = withCatalog, Size size = const Size(1280, 2000),
      List<String> agents = const ['Claude', 'Codex'], Future<String?> Function(String)? findProgram,
      Future<String> Function(Uri)? fetch, OverlayNeeds? overlayNeeds, ProgramStores? runtimes}) async {
    await tester.binding.setSurfaceSize(size);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    config = CanoaConfig.parse(text);
    await tester.pumpWidget(MaterialApp(
      home: StoreSection(
        readAhead: false,
        config: config,
        home: MemoryHome(overlays),
        index: standInIndex(overlays, () => saved.isEmpty ? config : saved.last),
        agents: agents,
        onSave: (changed) async {
          saved.add(changed);
          return changed;
        },
        readCatalog: standInCatalogRead(overlays, () => saved.isEmpty ? config : saved.last, registry: fetch ?? (uri) async => registryAnswer),
        findProgram: findProgram ?? (program) async => '/usr/bin/$program',
        overlayNeeds: overlayNeeds,
        runtimes: runtimes,
      ),
    ));
    await tester.pumpAndSettle();
  }

  setUp(() {
    overlays = world();
    saved = [];
  });

  String tabText(WidgetTester tester, String key) => (tester.widget<Tab>(find.byKey(Key(key))).text)!;

  Future<void> tapKey(WidgetTester tester, String key) async {
    await tester.ensureVisible(find.byKey(Key(key)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key(key)));
    await tester.pumpAndSettle();
  }

  Future<void> openTab(WidgetTester tester, String name) async {
    await tester.tap(find.byKey(Key('store-tab-$name')));
    await tester.pumpAndSettle();
  }

  Future<void> installReview(WidgetTester tester) async {
    await tapKey(tester, 'overlays-discover');
    await tapKey(tester, 'store-entry-main-review');
    await tapKey(tester, 'store-fetch');
    await tapKey(tester, 'store-install');
    await tester.pageBack();
    await tester.pumpAndSettle();
  }

  testWidgets('each tab counts what is installed, and Discover counts what the catalogs offer', (tester) async {
    await open(tester);
    expect(tabText(tester, 'store-tab-overlays'), 'Overlays 0');
    expect(tabText(tester, 'store-tab-skills'), 'Skills 0');
    expect(tabText(tester, 'store-tab-mcps'), 'MCP servers 0');
    expect(find.text('Discover 1'), findsOneWidget, reason: 'the catalog offers one overlay');

    await installReview(tester);
    expect(tabText(tester, 'store-tab-overlays'), 'Overlays 1');
    expect(find.text('Installed 1'), findsOneWidget);
    expect(tabText(tester, 'store-tab-mcps'), 'MCP servers 1', reason: 'the overlay brings one MCP server');
    expect(saved.last.overlays.keys, ['review']);
  });

  testWidgets('an installed overlay names where it came from, what it applies, and is on for new sessions until turned off', (tester) async {
    await open(tester);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    final card = find.byKey(const Key('overlay-installed-review'));
    expect(card, findsOneWidget);
    String within(String key) => tester.widget<Text>(find.descendant(of: card, matching: find.byKey(Key(key)))).data!;
    expect(within('provenance'), allOf(contains('from the catalog main'), contains('$teamRepo@main'), contains('commit '), contains('pinned sha256:')));
    expect(find.descendant(of: card, matching: find.text('Review conventions.')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.textContaining('Applies: 1 environment variable · 1 MCP server (off until allowed)')), findsOneWidget);
    expect(tester.widget<Switch>(find.byKey(const Key('overlay-default-review'))).value, isTrue);

    await tapKey(tester, 'overlay-default-review');
    expect(saved.last.loadouts['Default']!.overlays, isEmpty);
    expect(saved.last.defaultOverlays, isEmpty, reason: 'a new session starts without it');
    expect(saved.last.overlays.keys, ['review'], reason: 'it stays installed');
    expect(find.text('Off for new sessions'), findsOneWidget);
    await tapKey(tester, 'overlay-default-review');
    expect(saved.last.loadouts['Default']!.overlays, ['review']);
  });

  testWidgets('an installed overlay says what it needs and this computer lacks, with the Store runtime that installs it', (tester) async {
    await open(tester, overlayNeeds: (name) async => name == 'review'
        ? const [
            NeedLine(
              key: 'runtime.python',
              met: false,
              how: "Python 3.11 or later is needed and is not on the PATH; the Store's Python installs it",
              offer: 'python_store',
            ),
            NeedLine(key: 'runtime.commands: git', met: true, how: 'found /usr/bin/git'),
            NeedLine(key: 'runtime.commands: canoa-absent', met: false, how: 'canoa-absent is needed and is not on the PATH'),
          ]
        : const []);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    final card = find.byKey(const Key('overlay-installed-review'));
    expect(find.descendant(of: card, matching: find.textContaining('Needs python: Python 3.11 or later is needed')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.textContaining('Needs commands: canoa-absent: canoa-absent is needed')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.text('Found on this computer: commands: git')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.byKey(const Key('overlay-needs-install-review-python_store'))), findsOneWidget,
        reason: 'only the program a Store runtime provides is offered');
    expect(find.byKey(const Key('overlay-needs-install-review-null')), findsNothing);
    // Not applied by Canoa no longer lists what the host checks.
    expect(find.descendant(of: card, matching: find.textContaining('runtime')), findsNothing);
  });

  testWidgets("Install from the Store shows the host's plan for the runtime, installs it on the yes, and the card's lines refresh", (tester) async {
    final host = FakeStoreHost([
      FakeStore(
        'python_store',
        title: 'Python',
        runtime: true,
        commands: ['python', 'python3'],
        latest: '3.13.1',
        plan: {
          'install': planOf('install', '3.13.1', bytes: 40 * 1024 * 1024, downloads: [
            {'url': 'https://example.invalid/cpython-3.13.1.tar.gz', 'bytes': 40 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'c0' * 32}},
          ]),
        },
      ),
    ]);
    var found = false;
    host.afterApply = (store, plan) {
      store.copies = [copyOf('3.13.1')];
      found = true;
    };
    final runtimes = host.programs(kind: 'runtimes');
    addTearDown(runtimes.dispose);
    await tester.runAsync(runtimes.refresh);
    await open(tester, runtimes: runtimes, overlayNeeds: (name) async => name != 'review'
        ? const []
        : [
            found
                ? const NeedLine(key: 'runtime.python', met: true, how: 'found /programs/python/bin/python')
                : const NeedLine(key: 'runtime.python', met: false, how: "Python 3.11 or later is needed; the Store's Python installs it", offer: 'python_store'),
          ]);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    final card = find.byKey(const Key('overlay-installed-review'));
    expect(find.descendant(of: card, matching: find.textContaining('Needs python:')), findsOneWidget);

    // The button asks the host for its plan and shows it; a no installs nothing and the card is as it was.
    await tapKey(tester, 'overlay-needs-install-review-python_store');
    expect(host.params['store/plan']!.last, containsPair('action', 'install'));
    expect(host.params['store/plan']!.last, containsPair('provider', 'python_store'));
    expect(find.byKey(const Key('plan-confirm')), findsOneWidget);
    await tester.tap(find.byKey(const Key('plan-cancel')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
    expect(find.descendant(of: card, matching: find.textContaining('Needs python:')), findsOneWidget);

    // A yes runs the plan, and the host is asked again what the overlay needs.
    await tapKey(tester, 'overlay-needs-install-review-python_store');
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.tap(find.byKey(const Key('program-close')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call == 'store/apply python_store'), hasLength(1));
    expect(find.descendant(of: card, matching: find.textContaining('Needs python:')), findsNothing);
    expect(find.descendant(of: card, matching: find.text('Found on this computer: python')), findsOneWidget);
    expect(find.byKey(const Key('overlay-needs-install-review-python_store')), findsNothing);
  });

  testWidgets('an overlay whose needs are all met, or that the host cannot ask about, offers nothing to install', (tester) async {
    await open(tester, overlayNeeds: (name) async => const [NeedLine(key: 'runtime.node', met: true, how: 'found /usr/bin/node')]);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    final card = find.byKey(const Key('overlay-installed-review'));
    expect(find.descendant(of: card, matching: find.text('Found on this computer: node')), findsOneWidget);
    expect(find.descendant(of: card, matching: find.textContaining('Needs ')), findsNothing);
    expect(find.descendant(of: card, matching: find.text('Install from the Store')), findsNothing);
  });

  testWidgets('an overlay that has dependencies and a setup lists what will run, and is prepared on the yes that installs it', (tester) async {
    overlays.put(teamRepo, {
      'overlay.cfg': reviewCfg,
      'lib/python/requirements.txt': 'requests==2.0\n',
      'setup.py': 'pass\n',
    });
    await open(tester);
    await tapKey(tester, 'overlays-discover');
    await tapKey(tester, 'store-entry-main-review');
    await tapKey(tester, 'store-fetch');
    expect(find.byKey(const Key('store-prepares')), findsOneWidget);
    expect(find.textContaining('dependencies: lib/python/requirements.txt (pip install, not pinned'), findsOneWidget);
    expect(find.textContaining('setup: setup.py under python (up to 300 s; says it needs the network)'), findsOneWidget);
    expect(overlays.prepared, isEmpty, reason: 'listing what runs runs nothing');
    await tapKey(tester, 'store-install');
    expect(overlays.records.keys, ['review']);
    expect(overlays.prepared, ['review']);
    expect(find.byKey(const Key('store-problem')), findsNothing);
  });

  testWidgets('an overlay with nothing to run is not prepared, and one whose preparation fails stays installed and says so', (tester) async {
    await open(tester);
    await installReview(tester);
    expect(overlays.prepared, isEmpty, reason: 'nothing to prepare, so nothing was asked of the host');

    overlays.put(teamRepo, {
      'overlay.cfg': reviewCfg,
      'lib/python/requirements.txt': 'requests==2.0\n',
    });
    overlays.prepareFails = 'dependencies (python) failed: it ended with exit code: 1:\nno matching distribution';
    await tapKey(tester, 'overlays-discover');
    await tapKey(tester, 'store-entry-main-review');
    await tapKey(tester, 'store-fetch');
    await tapKey(tester, 'store-install');
    await tapKey(tester, 'store-move-pin');
    expect(overlays.records.keys, ['review'], reason: 'the files are installed whatever the preparation did');
    expect(saved.last.overlays.keys, ['review']);
    expect(find.byKey(const Key('store-problem')), findsOneWidget);
    expect(find.textContaining('Installed, but preparing it failed: dependencies (python) failed'), findsOneWidget);
  });

  testWidgets('what a card says is not prepared is run only after the person is shown it and says yes', (tester) async {
    await open(tester, overlayNeeds: (name) async => const [
      NeedLine(
        key: 'dependencies: lib/python/requirements.txt',
        met: false,
        how: 'not done for this version: install or sync the overlay in Canoa',
        runs: 'dependencies: lib/python/requirements.txt (pip install, not pinned; fetches from the network)',
      ),
      NeedLine(key: 'setup: setup.py', met: false, how: 'not done for this version', runs: 'setup: setup.py under python (up to 300 s)'),
    ]);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    final card = find.byKey(const Key('overlay-installed-review'));
    expect(find.descendant(of: card, matching: find.textContaining('Needs dependencies: lib/python/requirements.txt')), findsOneWidget);
    await tapKey(tester, 'overlay-prepare-review');
    expect(find.byKey(const Key('overlay-prepare-line-0')), findsOneWidget);
    expect(find.byKey(const Key('overlay-prepare-line-1')), findsOneWidget);
    expect(overlays.prepared, isEmpty, reason: 'the list is shown first');
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(overlays.prepared, isEmpty);
    await tapKey(tester, 'overlay-prepare-review');
    await tester.tap(find.byKey(const Key('overlay-prepare-confirm')));
    await tester.pumpAndSettle();
    expect(overlays.prepared, ['review']);
  });

  testWidgets('removing what the store installed takes its entry and its default with it', (tester) async {
    await open(tester);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    await tapKey(tester, 'overlay-default-review');
    await tapKey(tester, 'overlay-remove-review');
    await tapKey(tester, 'overlay-remove-confirm');
    expect(saved.last.overlays, isEmpty);
    expect(saved.last.loadouts.values.expand((loadout) => loadout.overlays), isEmpty);
    expect(overlays.records, isEmpty);
    expect(find.byKey(const Key('overlays-none')), findsOneWidget);
  });

  testWidgets('what an installed overlay provides is listed as installed under its type, marked with the overlay, with no Remove, and goes with it', (tester) async {
    overlays.put(teamRepo, {'overlay.cfg': reviewCfg, 'skills/helper/SKILL.md': skillMd('helper')});
    await open(tester);
    await openTab(tester, 'skills');
    expect(find.byKey(const Key('skills-none')), findsOneWidget, reason: 'nothing is installed yet');
    await openTab(tester, 'overlays');
    await installReview(tester);

    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-installed');
    final skill = find.byKey(const Key('skill-installed-helper'));
    expect(skill, findsOneWidget, reason: 'the overlay\'s skill is installed with it');
    expect(find.descendant(of: skill, matching: find.text('Comes with review')), findsOneWidget);
    expect(find.byKey(const Key('skill-remove-helper')), findsNothing, reason: 'it goes only with its overlay');
    expect(find.byKey(const Key('skills-none')), findsNothing);

    await openTab(tester, 'mcps');
    await tapKey(tester, 'mcps-installed');
    final server = find.byKey(const Key('mcp-installed-tracker'));
    expect(server, findsOneWidget);
    expect(find.descendant(of: server, matching: find.textContaining('comes with the overlay review')), findsOneWidget);
    expect(find.byKey(const Key('mcp-remove-tracker')), findsNothing, reason: 'it goes only with its overlay');

    await openTab(tester, 'overlays');
    await tapKey(tester, 'overlays-installed');
    await tapKey(tester, 'overlay-remove-review');
    await tapKey(tester, 'overlay-remove-confirm');
    await openTab(tester, 'skills');
    expect(find.byKey(const Key('skill-installed-helper')), findsNothing, reason: 'removing the overlay removes its skill');
    expect(find.byKey(const Key('skills-none')), findsOneWidget);
    await openTab(tester, 'mcps');
    expect(find.byKey(const Key('mcp-installed-tracker')), findsNothing, reason: 'and its MCP server');
  });

  testWidgets('a Discover card says which catalog it is from and its repository, whatever store offers it', (tester) async {
    overlays.put('https://example.invalid/other.git', {'catalog.toml': '[overlays.lint]\nsource = "https://example.invalid/lint.git@main"\n'});
    await open(tester, text: '${withCatalog}other = "https://example.invalid/other.git@main#catalog.toml"\n');
    await tapKey(tester, 'overlays-discover');
    final card = find.byKey(const Key('store-entry-main-review'));
    expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data,
        'from https://example.invalid/review.git@main · main');
    expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
  });

  testWidgets('with no catalog, Discover offers a known catalog and a spec to paste, and no bare repository field', (tester) async {
    await open(tester, text: '[store.overlays.catalogs]\nboriken = false\n');
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(const Key('catalogs-empty')), findsOneWidget);
    expect(find.byKey(Key('known-catalog-$defaultCatalogLabel')), findsOneWidget);
    expect(find.textContaining('https://github.com/ana/overlays.git@main#catalog.toml'), findsWidgets, reason: 'an example to paste');
    expect(find.textContaining('<repository>'), findsNothing);
    expect(find.textContaining('add one to [overlays]'), findsNothing);

    await tester.enterText(find.byKey(const Key('store-catalog-source')), 'https://example.invalid/team-things.git@main#catalog.toml');
    await tapKey(tester, 'store-add-catalog');
    expect(saved.last.catalogs.keys, ['team-things'], reason: 'the label is the repository\'s name when none is given');
  });

  testWidgets('Boriken Dev\'s catalog is the only one offered to start from', (tester) async {
    await open(tester, text: '[store.overlays.catalogs]\nboriken = false\n');
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(Key('known-catalog-$defaultCatalogLabel')), findsOneWidget);
    expect(find.byKey(const Key('known-catalog-aos-examples')), findsNothing);
    expect(knownCatalogs.map((each) => each.label), [defaultCatalogLabel]);
  });

  testWidgets('the default catalog removed comes back from the empty state under its own label', (tester) async {
    await open(tester, text: '[store.overlays.catalogs]\nboriken = false\n');
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(Key('known-catalog-$defaultCatalogLabel')), findsOneWidget);
    await tapKey(tester, 'known-catalog-add-$defaultCatalogLabel');
    expect(saved.last.catalogs.keys, [defaultCatalogLabel]);
    expect(saved.last.catalogs[defaultCatalogLabel]!.spec, defaultCatalogSpec);
    expect(saved.last.catalogTable('overlays'), {defaultCatalogLabel: defaultCatalogSpec});
  });

  testWidgets('Skills Discover lists no skill that only an overlay catalog names', (tester) async {
    await open(tester);
    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-discover');
    expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing, reason: 'the catalog names pdf for its overlay');
    expect(find.byKey(const Key('skills-from-catalogs')), findsNothing);
    expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
    expect(find.byKey(const Key('store-banner')), findsNothing);
  });

  testWidgets('a skill pasted is shown with its source, fetched on a yes, chosen and on for new sessions', (tester) async {
    await open(tester);
    expect(find.text('Skills 0'), findsOneWidget);
    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-discover');
    overlays.putSkill('https://example.invalid/skills.git@main#skills/pdf', pdfFiles);
    await tester.enterText(find.byKey(const Key('skills-paste')), 'https://example.invalid/skills.git@main#skills/pdf');
    await tapKey(tester, 'skills-paste-review');
    expect(find.textContaining('unpinned'), findsWidgets);
    expect(overlays.skillFetches, isEmpty, reason: 'nothing is fetched before the yes');
    await tapKey(tester, 'store-skill-install-confirm');
    expect(overlays.skills.keys, ['pdf']);
    expect(tabText(tester, 'store-tab-skills'), 'Skills 1');
    expect(saved.last.loadoutSkills, {'pdf'});
    expect(saved.last.profile.skills, isEmpty);

    final card = find.byKey(const Key('skill-installed-pdf'));
    expect(find.descendant(of: card, matching: find.text('Chosen by you')), findsOneWidget);
    expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data, contains('unpinned'));
    await tapKey(tester, 'skill-default-pdf');
    expect(saved.last.loadoutSkills, isEmpty, reason: 'off for new sessions, still installed');
    expect(overlays.skills.keys, ['pdf']);
    await tapKey(tester, 'skill-remove-pdf');
    expect(overlays.skills, isEmpty);
    expect(find.byKey(const Key('skills-none')), findsOneWidget);
  });

  testWidgets('a skill pasted as a source is reviewed before it is fetched, and refused when its directory is no name', (tester) async {
    await open(tester, text: '[store.overlays.catalogs]\nboriken = false\n');
    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-discover');
    await tester.enterText(find.byKey(const Key('skills-paste')), 'https://example.invalid/skills.git@main#');
    await tapKey(tester, 'skills-paste-review');
    expect(find.byKey(const Key('skills-problem')), findsOneWidget);
    overlays.putSkill('https://example.invalid/skills.git@main#skills/pdf', pdfFiles);
    await tester.enterText(find.byKey(const Key('skills-paste')), 'https://example.invalid/skills.git@main#skills/pdf');
    await tapKey(tester, 'skills-paste-review');
    expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
    expect(find.textContaining('unpinned'), findsWidgets);
    expect(overlays.skillFetches, isEmpty);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(overlays.skills, isEmpty);
  });

  group('MCP servers', () {
    Future<void> review(WidgetTester tester, String name) async {
      await openTab(tester, 'mcps');
      await tapKey(tester, 'mcps-discover');
      await tapKey(tester, 'mcp-install-$name');
    }

    testWidgets('the registry\'s servers are listed with where they come from, and a server is reviewed in full before it is written', (tester) async {
      await open(tester);
      await openTab(tester, 'mcps');
      await tapKey(tester, 'mcps-discover');
      final card = find.byKey(const Key('mcp-entry-io.github.ana/files'));
      expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data,
          allOf(contains('registry.modelcontextprotocol.io'), contains('io.github.ana/files 1.2.0'), contains('https://github.com/ana/files')));
      await tapKey(tester, 'mcp-entry-io.github.ana/files');
      expect(find.byKey(const Key('mcp-details-io.github.ana/files')), findsOneWidget, reason: 'the card opens its details');
      expect(find.byKey(const Key('details-technical')), findsOneWidget);
      expect(find.textContaining('Registry: registry.modelcontextprotocol.io'), findsOneWidget);
      expect(find.byKey(const Key('mcp-install-dialog')), findsNothing, reason: 'looking is not installing');
      await tapKey(tester, 'entry-details-action');
      expect(find.byKey(const Key('mcp-install-dialog')), findsOneWidget);
      expect(find.textContaining('server.json digest sha256:'), findsOneWidget);
      expect(find.textContaining('Starts npx -y @ana/files-mcp@1.2.0'), findsOneWidget);
      expect(find.textContaining('off for Claude, Codex'), findsOneWidget);
      expect(saved, isEmpty, reason: 'nothing is written before the yes');
    });

    testWidgets('installing writes it to the profile, off for every agent, and it starts only once allowed', (tester) async {
      await open(tester);
      await review(tester, 'io.github.ana/files');
      await tapKey(tester, 'mcp-install-confirm');
      final changed = saved.last;
      expect(changed.profile.mcp.keys, ['files']);
      expect(changed.agentMcp, {'Claude': {'files': false}, 'Codex': {'files': false}});
      expect(changed.storeMcp['files']!.server, 'io.github.ana/files');
      expect(changed.storeMcp['files']!.digest, startsWith('sha256:'));
      expect(tabText(tester, 'store-tab-mcps'), 'MCP servers 1');
      expect(find.textContaining('Off for every agent'), findsOneWidget);
      expect(find.textContaining('from the registry registry.modelcontextprotocol.io'), findsOneWidget);

      await tapKey(tester, 'mcp-allowed-files');
      expect(saved.last.agentMcp, {'Claude': {'files': true}, 'Codex': {'files': true}});
      expect(find.textContaining('Allowed for 2 of 2 agents'), findsOneWidget);
      await tapKey(tester, 'mcp-remove-files');
      expect(find.byKey(const Key('confirm-remove')), findsOneWidget);
      await tapKey(tester, 'confirm-remove-no');
      expect(saved.last.profile.mcp.keys, ['files'], reason: 'Keep it removes nothing');
      expect(find.byKey(const Key('mcp-remove-files')), findsOneWidget);
      await tapKey(tester, 'mcp-remove-files');
      await tapKey(tester, 'confirm-remove-yes');
      expect(saved.last.profile.mcp, isEmpty);
      expect(saved.last.agentMcp, isEmpty);
    });

    testWidgets('a package whose runtime is not on this computer says so and cannot be installed', (tester) async {
      await open(tester, findProgram: (program) async => null);
      await review(tester, 'io.github.ana/files');
      expect(find.textContaining('Needs npx, which is not on this computer'), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('mcp-install-confirm'))).onPressed, isNull);
    });

    testWidgets('a server Canoa refuses names what it does not do, and a name already taken is refused', (tester) async {
      await open(tester);
      await review(tester, 'io.github.ana/bundle');
      expect(find.textContaining('MCP bundle (mcpb), which Canoa does not unpack or run'), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('mcp-install-confirm'))).onPressed, isNull);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      await tester.pumpWidget(const SizedBox());
      await open(tester, text: '$withCatalog[profile.mcp.files]\ntype = "stdio"\ncommand = "files"\n');
      await review(tester, 'io.github.ana/files');
      expect(find.textContaining('already has a server named files'), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('mcp-install-confirm'))).onPressed, isNull);
    });

    testWidgets('a registry added in Discover is listed beside the official one, read by its own address, and can be taken away', (tester) async {
      final asked = <Uri>[];
      await open(tester, fetch: (uri) async {
        asked.add(uri);
        return uri.host == 'registry.team.test'
            ? page([{'name': 'dev.team/tracker', 'description': 'The team tracker.', 'version': '2.0.0'}])
            : registryAnswer;
      });
      await openTab(tester, 'mcps');
      await tapKey(tester, 'mcps-discover');
      expect(find.byKey(const Key('mcps-registry-registry.modelcontextprotocol.io')), findsOneWidget);

      await tester.enterText(find.byKey(const Key('mcps-registry-address')), 'http://registry.team.test');
      await tapKey(tester, 'mcps-registry-add');
      expect(find.textContaining('https address'), findsOneWidget, reason: 'plain http to another computer is refused, with why');
      expect(saved, isEmpty);

      await tester.enterText(find.byKey(const Key('mcps-registry-address')), 'https://registry.team.test');
      await tapKey(tester, 'mcps-registry-add');
      expect(saved.last.storeRegistries, ['https://registry.team.test']);
      expect(asked.last.host, 'registry.team.test');
      expect(find.byKey(const Key('mcp-entry-dev.team/tracker')), findsOneWidget);
      expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsNothing);
      expect(find.textContaining('Listed by registry.team.test'), findsOneWidget);

      await tapKey(tester, 'mcps-registry-registry.modelcontextprotocol.io');
      expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsOneWidget);
      await tapKey(tester, 'mcps-registry-registry.team.test');
      await tapKey(tester, 'mcps-registry-remove');
      expect(find.byKey(const Key('confirm-remove')), findsOneWidget);
      await tapKey(tester, 'confirm-remove-no');
      expect(find.byKey(const Key('mcps-registry-remove')), findsOneWidget, reason: 'Keep it leaves the registry chosen');
      await tapKey(tester, 'mcps-registry-remove');
      await tapKey(tester, 'confirm-remove-yes');
      expect(saved.last.storeRegistries, isEmpty);
      expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsOneWidget, reason: 'back to the official registry');
    });

    testWidgets('a registry that cannot be reached is a line, not an exception', (tester) async {
      await open(tester, fetch: (uri) async => throw const RegistryRefusal('The registry did not answer in time'));
      await openTab(tester, 'mcps');
      await tapKey(tester, 'mcps-discover');
      expect(find.byKey(const Key('mcps-registry-problem')), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  });

  test('the registry answer used here is the shape the registry serves', () {
    final read = jsonDecode(registryAnswer) as Map<String, dynamic>;
    expect(read['servers'], hasLength(2));
    expect((read['servers'] as List).first['server']['name'], 'io.github.ana/files');
  });
}
