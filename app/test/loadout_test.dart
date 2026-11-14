// Loadout: the page with Overlays, Skills and MCPs, each with what is installed and a place to discover more. Overlays and skills
// come from catalogs in memory, MCP servers from a registry's answer written here; nothing touches the person's files.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:canoa/loadout/mcp_registry.dart' show RegistryRefusal;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';
import 'mcp_registry_test.dart' show npmServer, page;
import 'skills_from_source_test.dart' show pdfFiles, pinOf, skillsHost;

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

const withCatalog = '[catalogs]\nmain = "$catalogRepo@main#catalog.toml"\n';

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
      Future<String> Function(Uri)? fetch}) async {
    await tester.binding.setSurfaceSize(size);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    config = CanoaConfig.parse(text);
    await tester.pumpWidget(MaterialApp(
      home: LoadoutPage(
        config: config,
        home: MemoryHome(overlays),
        agents: agents,
        onSave: (changed) async {
          saved.add(changed);
          return changed;
        },
        fetchRegistry: fetch ?? (uri) async => registryAnswer,
        findProgram: findProgram ?? (program) async => '/usr/bin/$program',
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
    await tester.tap(find.byKey(Key('loadout-tab-$name')));
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
    expect(tabText(tester, 'loadout-tab-overlays'), 'Overlays 0');
    expect(tabText(tester, 'loadout-tab-skills'), 'Skills 0');
    expect(tabText(tester, 'loadout-tab-mcps'), 'MCPs 0');
    expect(find.text('Discover 1'), findsOneWidget, reason: 'the catalog offers one overlay');

    await installReview(tester);
    expect(tabText(tester, 'loadout-tab-overlays'), 'Overlays 1');
    expect(find.text('Installed 1'), findsOneWidget);
    expect(tabText(tester, 'loadout-tab-mcps'), 'MCPs 1', reason: 'the overlay brings one MCP server');
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
    expect(saved.last.loadoutOff, {'review'});
    expect(saved.last.defaultOverlays, isEmpty, reason: 'a new session starts without it');
    expect(saved.last.overlays.keys, ['review'], reason: 'it stays installed');
    expect(find.text('Off for new sessions'), findsOneWidget);
    await tapKey(tester, 'overlay-default-review');
    expect(saved.last.loadoutOff, isEmpty);
  });

  testWidgets('removing what the store installed takes its entry and its default with it', (tester) async {
    await open(tester);
    await installReview(tester);
    await tapKey(tester, 'overlays-installed');
    await tapKey(tester, 'overlay-default-review');
    await tapKey(tester, 'overlay-remove-review');
    await tapKey(tester, 'overlay-remove-confirm');
    expect(saved.last.overlays, isEmpty);
    expect(saved.last.loadoutOff, isEmpty);
    expect(overlays.records, isEmpty);
    expect(find.byKey(const Key('overlays-none')), findsOneWidget);
  });

  testWidgets('a Discover card says which catalog it is from and its repository, and the catalog filter narrows the list', (tester) async {
    overlays.put('https://example.invalid/other.git', {'catalog.toml': '[overlays.lint]\nsource = "https://example.invalid/lint.git@main"\n'});
    await open(tester, text: '${withCatalog}other = "https://example.invalid/other.git@main#catalog.toml"\n');
    await tapKey(tester, 'overlays-discover');
    final card = find.byKey(const Key('store-entry-main-review'));
    expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data,
        'from the catalog main · $teamRepo@main');
    expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
    await tapKey(tester, 'catalog-filter-other');
    expect(find.byKey(const Key('store-entry-main-review')), findsNothing);
    expect(find.byKey(const Key('store-entry-other-lint')), findsOneWidget);
  });

  testWidgets('with no catalog, Discover offers a known catalog and a spec to paste, and no bare repository field', (tester) async {
    await open(tester, text: '');
    await tapKey(tester, 'overlays-discover');
    expect(find.byKey(const Key('catalogs-empty')), findsOneWidget);
    expect(find.byKey(const Key('known-catalog-aos-examples')), findsOneWidget);
    expect(find.textContaining('https://github.com/ana/overlays.git@main#catalog.toml'), findsWidgets, reason: 'an example to paste');
    expect(find.textContaining('<repository>'), findsNothing);
    expect(find.textContaining('add one to [overlays]'), findsNothing);

    await tester.enterText(find.byKey(const Key('store-catalog-source')), 'https://example.invalid/team-things.git@main#catalog.toml');
    await tapKey(tester, 'store-add-catalog');
    expect(saved.last.catalogs.keys, ['team-things'], reason: 'the label is the repository\'s name when none is given');
  });

  testWidgets('a known catalog is added in one tap, under its own label', (tester) async {
    await open(tester, text: '');
    await tapKey(tester, 'overlays-discover');
    await tapKey(tester, 'known-catalog-add-aos-examples');
    expect(saved.last.catalogs.keys, ['aos-examples']);
    expect(saved.last.catalogs['aos-examples']!.path, 'examples/registry/catalog.toml');
  });

  testWidgets('a skill the catalog offers is shown with its source and pin, fetched on a yes, chosen and on for new sessions', (tester) async {
    await open(tester);
    expect(find.text('Skills 0'), findsOneWidget);
    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-discover');
    final offered = find.byKey(const Key('store-offered-skill-main-pdf'));
    expect(tester.widget<Text>(find.descendant(of: offered, matching: find.byKey(const Key('provenance')))).data,
        contains('from the catalog main · '));
    await tapKey(tester, 'store-skill-install-main-pdf');
    expect(find.textContaining('pinned ${pinOf('pdf', pdfFiles)}'), findsWidgets);
    expect(overlays.skillFetches, isEmpty, reason: 'nothing is fetched before the yes');
    await tapKey(tester, 'store-skill-install-confirm');
    expect(overlays.skills.keys, ['pdf']);
    expect(tabText(tester, 'loadout-tab-skills'), 'Skills 1');
    expect(saved.last.loadoutSkills, {'pdf'});
    expect(saved.last.profile.skills, isEmpty);

    final card = find.byKey(const Key('skill-installed-pdf'));
    expect(find.descendant(of: card, matching: find.text('Chosen by you')), findsOneWidget);
    expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data, contains('pinned sha256:'));
    await tapKey(tester, 'skill-default-pdf');
    expect(saved.last.loadoutSkills, isEmpty, reason: 'off for new sessions, still installed');
    expect(overlays.skills.keys, ['pdf']);
    await tapKey(tester, 'skill-remove-pdf');
    expect(overlays.skills, isEmpty);
    expect(find.byKey(const Key('skills-none')), findsOneWidget);
  });

  testWidgets('a skill pasted as a source is reviewed before it is fetched, and refused when its directory is no name', (tester) async {
    await open(tester, text: '');
    await openTab(tester, 'skills');
    await tapKey(tester, 'skills-discover');
    await tester.enterText(find.byKey(const Key('skills-paste')), 'https://example.invalid/skills.git@main#');
    await tapKey(tester, 'skills-paste-review');
    expect(find.byKey(const Key('skills-problem')), findsOneWidget);
    overlays.putSkill('https://example.invalid/skills.git#skills/pdf', pdfFiles);
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
      await tapKey(tester, 'mcp-entry-$name');
    }

    testWidgets('the registry\'s servers are listed with where they come from, and a server is reviewed in full before it is written', (tester) async {
      await open(tester);
      await openTab(tester, 'mcps');
      await tapKey(tester, 'mcps-discover');
      final card = find.byKey(const Key('mcp-entry-io.github.ana/files'));
      expect(tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const Key('provenance')))).data,
          allOf(contains('registry.modelcontextprotocol.io'), contains('io.github.ana/files 1.2.0'), contains('https://github.com/ana/files')));
      await tapKey(tester, 'mcp-entry-io.github.ana/files');
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
      expect(changed.loadoutMcp['files']!.server, 'io.github.ana/files');
      expect(changed.loadoutMcp['files']!.digest, startsWith('sha256:'));
      expect(tabText(tester, 'loadout-tab-mcps'), 'MCPs 1');
      expect(find.textContaining('Off for every agent'), findsOneWidget);
      expect(find.textContaining('from the registry registry.modelcontextprotocol.io'), findsOneWidget);

      await tapKey(tester, 'mcp-allowed-files');
      expect(saved.last.agentMcp, {'Claude': {'files': true}, 'Codex': {'files': true}});
      expect(find.textContaining('Allowed for 2 of 2 agents'), findsOneWidget);
      await tapKey(tester, 'mcp-remove-files');
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
