// Catalogs of Skills and MCP servers, whichever provider serves them (a source or registry the host reads, a program, an extension):
// the page asks the host's `store/catalog` for the label, lists what it answers, and installs a skill from the source it names. The
// host's own reading is tested in canoa-host; here the host is a stand-in whose answers have the shape the Rust ones have.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/store/catalog_types.dart' show CatalogReadFailed;
import 'package:canoa/store/store_section.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show withCatalog, world;
import 'mcp_registry_test.dart' show npmServer, page;
import 'skills_from_source_test.dart' show pdfFiles;

const commit = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678';
const tools = 'https://example.test/team/tools.git';
const anthropics = 'https://github.com/anthropics/skills.git';

const generated = '''$withCatalog
[store.skills.catalogs.made]
provider = "command"
command = ["make-skills"]

[store.mcps.catalogs.made]
provider = "command"
command = ["make-servers"]

[store.mcps.catalogs.team]
source = "https://registry.team.test"
''';

void main() {
  late MemoryOverlays overlays;
  late List<CanoaConfig> saved;
  late CanoaConfig config;
  late List<String> asked;
  Object? failure;

  setUp(() {
    overlays = world()
      ..scans[tools] = const SkillScan(
        commit: commit,
        skills: [ScannedSkill(name: 'pdf', description: 'Reads PDFs.', path: 'skills/pdf')],
      )
      ..putSkill('$tools@$commit#skills/pdf', pdfFiles)
      ..putSkill('$anthropics@$commit#skills/pdf', pdfFiles);
    saved = [];
    asked = [];
    failure = null;
  });

  Future<Map<Object?, Object?>> readCatalog(String type, String label, {String? search, String? cursor}) async {
    asked.add('$type/$label/${search ?? ''}/${cursor ?? ''}');
    if (failure != null) throw CatalogReadFailed('$failure');
    if (type == 'skills' && label == 'anthropics') {
      // What the host answers for a `read` source: the commit it scanned, and each skill with the source it was found in.
      return {
        'type': 'skills',
        'label': label,
        'provider': 'read',
        'commit': commit,
        'skills': [
          {'name': 'pdf', 'description': 'Reads PDFs.', 'source': 'anthropics/skills', 'path': 'skills/pdf'},
        ],
        'skipped': 1,
        'truncated': false,
      };
    }
    if (type == 'skills') {
      return {
        'commit': null,
        'skills': [
          {'name': 'pdf', 'description': 'Reads PDFs.', 'source': tools, 'path': 'skills/pdf'},
          {'name': 'odd', 'description': 'Odd.', 'source': 'x', 'path': '', 'problem': 'x is no source'},
        ],
        'skipped': 2,
        'truncated': false,
      };
    }
    return jsonDecode(page([npmServer()], next: 'more')) as Map<Object?, Object?>;
  }

  Future<void> tapKey(WidgetTester tester, String key) async {
    await tester.ensureVisible(find.byKey(Key(key)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key(key)));
    await tester.pumpAndSettle();
  }

  Future<void> open(WidgetTester tester, String tab) async {
    await tester.binding.setSurfaceSize(const Size(1280, 2000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    config = CanoaConfig.parse(generated);
    await tester.pumpWidget(MaterialApp(
      home: StoreSection(
        readAhead: false,
        config: config,
        home: MemoryHome(overlays),
        index: standInIndex(overlays, () => saved.isEmpty ? config : saved.last),
        agents: const ['Claude'],
        catalogTypes: standInCatalogTypes,
        onSave: (changed) async {
          saved.add(changed);
          return changed;
        },
        readCatalog: readCatalog,
        findProgram: (program) async => '/usr/bin/$program',
      ),
    ));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key('store-tab-$tab')));
    await tester.pumpAndSettle();
    await tapKey(tester, '$tab-discover');
  }

  testWidgets('a generated skills catalog is a source the page reads through the host, and a listed skill is installed from the source it names', (tester) async {
    await open(tester, 'skills');
    expect(find.byKey(const Key('skills-source-made')), findsOneWidget);
    expect(asked, isEmpty, reason: 'listing a source reads nothing');

    await tapKey(tester, 'skills-source-made');
    expect(asked, ['skills/made//']);
    expect(find.byKey(const Key('store-source-skill-made-pdf')), findsOneWidget);
    expect(find.textContaining('Cannot be installed: x is no source'), findsOneWidget);
    expect(find.byKey(const Key('skills-source-left-out')), findsOneWidget);
    expect(overlays.scanned, isEmpty, reason: 'the listing reads no repository');

    await tapKey(tester, 'store-source-skill-install-made-pdf');
    expect(overlays.scanned, ['$tools@HEAD#skills/pdf'], reason: 'the source it names is read, at the folder it names');
    expect(overlays.scannedOwn, [false], reason: 'a program wrote the address, so none of your git settings are used');
    expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
    expect(overlays.skillFetches, isEmpty, reason: 'nothing is fetched before the yes');
    await tapKey(tester, 'store-skill-install-confirm');
    expect(overlays.skillFetches, ['$tools@$commit#skills/pdf']);
    expect(overlays.skillFetchOwn, [false]);
    expect(overlays.skills.keys, ['pdf']);
  });

  testWidgets('a catalog the host cannot read says why and lists nothing', (tester) async {
    failure = 'make-skills ended with exit code 3:\nit broke here';
    await open(tester, 'skills');
    await tapKey(tester, 'skills-source-made');
    expect(find.byKey(const Key('skills-source-problem')), findsOneWidget);
    expect(find.textContaining('ended with exit code 3'), findsOneWidget);
    expect(find.byKey(const Key('store-source-skill-made-pdf')), findsNothing);
  });

  testWidgets('a generated MCP registry is chosen beside the official one, read through the host with the search, and taken away', (tester) async {
    await open(tester, 'mcps');
    expect(find.byKey(const Key('mcps-registry-store:made')), findsOneWidget);
    await tapKey(tester, 'mcps-registry-store:made');
    expect(asked, ['mcps/official//', 'mcps/made//']);
    expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsOneWidget);
    expect(find.byKey(const Key('mcps-registry-remove')), findsOneWidget);

    await tapKey(tester, 'mcps-registry-remove');
    await tapKey(tester, 'confirm-remove-no');
    expect(find.byKey(const Key('mcps-registry-store:made')), findsOneWidget, reason: 'Keep it removes nothing');
    await tapKey(tester, 'mcps-registry-remove');
    await tapKey(tester, 'confirm-remove-yes');
    expect(saved.last.catalogSpecs('mcps').keys, ['team'], reason: 'its entry is gone from canoa.cfg, and the registry added by address stays');
    expect(find.byKey(const Key('mcps-registry-store:made')), findsNothing);
  });

  testWidgets('a read skills source is listed through the host too, and installing a skill from it reads no scan of its own', (tester) async {
    await open(tester, 'skills');
    expect(asked, isEmpty);
    await tapKey(tester, 'skills-source-anthropics');
    expect(asked, ['skills/anthropics//']);
    expect(overlays.scanned, isEmpty, reason: 'the host scanned the source; the page asks nothing of skill/scan to list it');
    expect(find.byKey(const Key('store-source-skill-anthropics-pdf')), findsOneWidget);
    expect(find.textContaining('Read at commit $commit'), findsOneWidget);
    expect(find.textContaining('none of your git settings'), findsOneWidget);

    await tapKey(tester, 'store-source-skill-install-anthropics-pdf');
    expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
    expect(overlays.skillFetches, isEmpty);
    await tapKey(tester, 'store-skill-install-confirm');
    expect(overlays.skillFetches, ['$anthropics@$commit#skills/pdf'], reason: 'the commit the host read is the one fetched');
    expect(overlays.scanned, isEmpty);
    expect(overlays.skills.keys, ['pdf']);
  });

  testWidgets('the official registry and one the person added are read through the host by label', (tester) async {
    await open(tester, 'mcps');
    expect(asked, ['mcps/official//'], reason: 'the official registry is the host default row');
    expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsOneWidget);

    await tapKey(tester, 'mcps-registry-registry.team.test');
    expect(asked, ['mcps/official//', 'mcps/team//']);
    expect(find.byKey(const Key('mcp-entry-io.github.ana/files')), findsOneWidget);
  });

  testWidgets('a registry the host cannot read says why', (tester) async {
    failure = 'The registry answered 404';
    await open(tester, 'mcps');
    expect(find.byKey(const Key('mcps-registry-problem')), findsOneWidget);
    expect(find.textContaining('The registry answered 404'), findsOneWidget);
  });
}
