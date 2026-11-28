// The skills directory's search (skills.sh) as a source the person turns on: off until then, kept in canoa.cfg as `[store] skills_search`,
// results that only say which repository holds a skill, and installing one reads that repository like any other source. The host's
// side is tested in canoa-host against a directory on loopback; here the host is a stand-in.
@Timeout(Duration(minutes: 3))
library;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/store/store_section.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show withCatalog, world;
import 'skills_from_source_test.dart' show pdfFiles;

const commit = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678';
const anthropics = 'https://github.com/anthropics/skills.git';

void main() {
  group('canoa.cfg [store] skills_search', () {
    test('is off until it is written, and true names skills.sh', () {
      expect(CanoaConfig.parse('').skillsSearch, isNull);
      expect(CanoaConfig.parse('[store]\nskills_search = false\n').skillsSearch, isNull);
      expect(CanoaConfig.parse('[store]\nskills_search = true\n').skillsSearch, 'https://skills.sh');
    });

    test('an address names another directory: https, or http to this computer', () {
      expect(CanoaConfig.parse('[store]\nskills_search = "https://find.example/skills/"\n').skillsSearch, 'https://find.example/skills');
      expect(CanoaConfig.parse('[store]\nskills_search = "http://127.0.0.1:8123"\n').skillsSearch, 'http://127.0.0.1:8123');
      expect(CanoaConfig.parse('[store]\nskills_search = "http://localhost:8123"\n').skillsSearch, 'http://localhost:8123');
      for (final bad in [
        '"http://find.example"',
        '"https://u:p@find.example"',
        '"https://find.example/?token=1"',
        '"https://find.example/#x"',
        '"ftp://find.example"',
        '"find.example"',
        '"https://find.example/a b"',
        '3',
      ]) {
        expect(() => CanoaConfig.parse('[store]\nskills_search = $bad\n'), throwsFormatException, reason: bad);
      }
    });

    test('turning it on and off writes and removes the key, and a limit saved beside it keeps it', () {
      var config = CanoaConfig.parse('[store]\ncatalog_max_entries = 3000\n');
      config = config.withSkillsSearch(true);
      expect(config.encode(), contains('skills_search = true'));
      expect(CanoaConfig.parse(config.encode()).skillsSearch, 'https://skills.sh');
      expect(config.store.catalogMaxEntries, 3000);
      expect(CanoaConfig.parse(config.encode()).store.catalogMaxEntries, 3000);
      config = config.withSkillsSearch(false);
      expect(config.encode(), isNot(contains('skills_search')));
      expect(config.skillsSearch, isNull);
      expect(config.store.catalogMaxEntries, 3000);
    });
  });

  group('a result', () {
    test('names a GitHub owner/repo and a plain skill, and nothing else is followed', () {
      final hit = SkillHit.fromJson({'id': 'anthropics/skills/pdf', 'source': 'anthropics/skills', 'skillId': 'pdf', 'name': 'pdf', 'installs': 231770})!;
      expect((hit.source, hit.skillId, hit.name, hit.installs, hit.id), ('anthropics/skills', 'pdf', 'pdf', 231770, 'anthropics/skills/pdf'));
      expect(SkillHit.fromJson({'source': 'o/r', 'skillId': 's', 'installs': -3, 'name': ''})!.installs, 0);
      expect(SkillHit.fromJson({'source': 'o/r', 'skillId': 's'})!.name, 's');
      for (final source in ['/etc', 'https://evil.example/o/r', 'git@evil.example:o/r', 'o/r/deeper', '../r', 'o', '']) {
        expect(SkillHit.fromJson({'source': source, 'skillId': 's'}), isNull, reason: source);
      }
      for (final skill in ['../x', 'a/b', '', '-x', 'a b']) {
        expect(SkillHit.fromJson({'source': 'o/r', 'skillId': skill}), isNull, reason: skill);
      }
      expect(SkillHit.fromJson('text'), isNull);
    });
  });

  group('the Skills page', () {
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    late CanoaConfig config;

    const pdf = ScannedSkill(name: 'pdf', description: 'Reads and fills PDFs.', path: 'skills/pdf');
    const docx = ScannedSkill(name: 'docx', description: 'Makes Word documents.', path: 'skills/docx');
    const pdfHit = SkillHit(source: 'anthropics/skills', skillId: 'pdf', name: 'pdf', installs: 231770);

    setUp(() {
      overlays = world()
        ..scans[anthropics] = SkillScan(commit: commit, skills: const [docx, pdf])
        ..hits['pdf'] = [pdfHit, const SkillHit(source: 'ana/gone', skillId: 'pdf-tools', name: 'pdf-tools', installs: 1)]
        ..hits['nothing here'] = []
        ..scans['https://github.com/ana/gone.git'] = SkillScan(commit: commit, skills: const [docx])
        ..putSkill('$anthropics@$commit#skills/pdf', pdfFiles);
      saved = [];
    });

    Future<void> tapKey(WidgetTester tester, String key) async {
      await tester.ensureVisible(find.byKey(Key(key)));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(Key(key)));
      await tester.pumpAndSettle();
    }

    Future<void> open(WidgetTester tester, {String text = withCatalog}) async {
      await tester.binding.setSurfaceSize(const Size(1280, 2000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      config = CanoaConfig.parse(text);
      await tester.pumpWidget(MaterialApp(
        home: StoreSection(
          readAhead: false,
          config: config,
          home: MemoryHome(overlays),
          index: standInIndex(overlays, () => saved.isEmpty ? config : saved.last),
          agents: const ['Claude'],
          onSave: (changed) async {
            saved.add(changed);
            return changed;
          },
          readCatalog: standInCatalogRead(overlays, () => saved.isEmpty ? config : saved.last, registry: (uri) async => '{}'),
          findProgram: (program) async => '/usr/bin/$program',
        ),
      ));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-tab-skills')));
      await tester.pumpAndSettle();
      await tapKey(tester, 'skills-discover');
    }

    Future<void> search(WidgetTester tester, String words) async {
      await tester.enterText(find.byKey(const Key('skills-search-query')), words);
      await tapKey(tester, 'skills-search-go');
    }

    testWidgets('with the search off, Discover offers no search and nothing is asked', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('skills-from-search')), findsNothing);
      expect(overlays.searched, isEmpty);
    });

    testWidgets('Store settings turns it on in canoa.cfg, and off again removes it', (tester) async {
      await open(tester);
      await tapKey(tester, 'store-settings');
      expect(tester.widget<SwitchListTile>(find.byKey(const Key('store-skills-search'))).value, isFalse);
      expect(find.textContaining('third party'), findsOneWidget);
      await tapKey(tester, 'store-skills-search');
      expect(saved.last.skillsSearch, 'https://skills.sh');
      expect(CanoaConfig.parse(saved.last.encode()).skillsSearch, 'https://skills.sh', reason: 'it is kept in the file');
      expect(overlays.searched, isEmpty, reason: 'turning it on asks nothing');
      await tapKey(tester, 'store-skills-search');
      expect(saved.last.skillsSearch, isNull);
      expect(saved.last.encode(), isNot(contains('skills_search')));
    });

    testWidgets('turned on, a search lists the results with their installs, and asks only for what was typed', (tester) async {
      await open(tester, text: '$withCatalog[store]\nskills_search = true\n');
      expect(overlays.searched, isEmpty, reason: 'choosing the search asks nothing');
      await tapKey(tester, 'skills-from-search');
      expect(find.byKey(const Key('skills-search-unsearched')), findsOneWidget);
      expect(find.textContaining('Only the words you search for are sent to skills.sh'), findsOneWidget);
      await search(tester, '  pdf ');
      expect(overlays.searched, ['pdf']);
      expect(find.byKey(const Key('store-search-skill-anthropics/skills/pdf')), findsOneWidget);
      expect(find.textContaining('231770 installs'), findsOneWidget);
      expect(find.textContaining('anthropics/skills/pdf'), findsWidgets);
      expect(find.byKey(const Key('store-search-skill-ana/gone/pdf-tools')), findsOneWidget);
      expect(overlays.scanned, ['$anthropics@HEAD#'], reason: 'listing a result reads no repository; only the default source was read, when Discover was shown');

      await search(tester, 'nothing here');
      expect(find.byKey(const Key('skills-search-nothing')), findsOneWidget);
      await search(tester, 'x');
      expect(find.textContaining('at least two characters'), findsOneWidget);
      expect(overlays.searched, ['pdf', 'nothing here'], reason: 'a one-letter search is not sent');
    });

    testWidgets('a refusal of the host is said, and the results are not left standing', (tester) async {
      await open(tester, text: '$withCatalog[store]\nskills_search = true\n');
      await tapKey(tester, 'skills-from-search');
      await search(tester, 'pdf');
      expect(find.byKey(const Key('store-search-skill-anthropics/skills/pdf')), findsOneWidget);
      await search(tester, 'unknown query');
      expect(find.byKey(const Key('skills-search-problem')), findsOneWidget);
      expect(find.textContaining('Skill search is off'), findsOneWidget);
      expect(find.byKey(const Key('store-search-skill-anthropics/skills/pdf')), findsNothing);
    });

    testWidgets('installing a result reads its repository as a source, then asks, then fetches the commit that was read', (tester) async {
      await open(tester, text: '$withCatalog[store]\nskills_search = true\n');
      await tapKey(tester, 'skills-from-search');
      await search(tester, 'pdf');
      await tapKey(tester, 'store-search-skill-install-anthropics/skills/pdf');
      expect(overlays.scanned, ['$anthropics@HEAD#', '$anthropics@HEAD#'], reason: 'the repository itself is read, not anything the directory sent');
      expect(overlays.scannedOwn, [false, false], reason: 'a directory\'s result is fetched with none of the person\'s git settings');
      expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
      expect(find.textContaining('$commit#skills/pdf'), findsWidgets);
      expect(overlays.skillFetches, isEmpty);
      await tapKey(tester, 'store-skill-install-confirm');
      expect(overlays.skillFetches, ['$anthropics@$commit#skills/pdf']);
      expect(overlays.skillFetchOwn, [false]);
      expect(overlays.skills.keys, ['pdf']);
      expect(saved.last.loadoutSkills, {'pdf'});
    });

    testWidgets('a result whose repository holds no such skill is said so and installs nothing', (tester) async {
      await open(tester, text: '$withCatalog[store]\nskills_search = true\n');
      await tapKey(tester, 'skills-from-search');
      await search(tester, 'pdf');
      await tapKey(tester, 'store-search-skill-install-ana/gone/pdf-tools');
      expect(find.textContaining('ana/gone holds no skill called pdf-tools'), findsOneWidget);
      expect(find.byKey(const Key('store-skill-review')), findsNothing);
      expect(overlays.skillFetches, isEmpty);
    });

    testWidgets('turning it off forgets the results and takes the search away', (tester) async {
      await open(tester, text: '$withCatalog[store]\nskills_search = true\n');
      await tapKey(tester, 'skills-from-search');
      await search(tester, 'pdf');
      await tapKey(tester, 'store-settings');
      await tapKey(tester, 'store-skills-search');
      expect(saved.last.skillsSearch, isNull);
      Navigator.of(tester.element(find.byKey(const Key('store-settings-page')))).pop();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-settings-page')), findsNothing);
      expect(find.byKey(const Key('skills-from-search')), findsNothing);
      expect(find.byKey(const Key('store-search-skill-anthropics/skills/pdf')), findsNothing);
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing, reason: 'an overlay skill is never offered alone');
      expect(find.byKey(const Key('skills-choose-source')), findsOneWidget);
    });
  });
}
