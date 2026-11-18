// Skills sources the way `npx skills add` takes them: each written form is read, a source is kept in canoa.cfg under its own table
// beside the catalogs with Anthropic's skills as a default the person can remove, choosing one fetches and lists the skills it holds,
// and installing one fetches exactly the commit that was listed. The host's scan is tested in canoa-host; here it is a stand-in.
@Timeout(Duration(minutes: 3))
library;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/store/skill_sources.dart';
import 'package:canoa/store/store_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show withCatalog, world;
import 'skills_from_source_test.dart' show pdfFiles;

const commit = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678';
const anthropics = 'https://github.com/anthropics/skills.git';
const typed = 'https://github.com/addyosmani/web-quality-skills.git';

SkillSource parse(String text) => SkillSource.parse(text);

void expectRefused(String text, [Pattern? why]) =>
    expect(() => SkillSource.parse(text), throwsA(isA<FormatException>().having((error) => error.message, 'message', why == null ? isNotEmpty : contains(why))), reason: text);

void main() {
  group('a source is written the way npx skills takes it', () {
    test('owner/repo, with a folder, a ref, a version or a skill', () {
      var source = parse('anthropics/skills');
      expect((source.repo, source.ref, source.path, source.only, source.local), (anthropics, null, '', null, false));
      source = parse('addyosmani/web-quality-skills');
      expect(source.repo, typed);
      source = parse('owner/repo/skills/pdf');
      expect((source.repo, source.path), ('https://github.com/owner/repo.git', 'skills/pdf'));
      source = parse('owner/repo#dev');
      expect((source.ref, source.only), ('dev', null));
      source = parse('owner/repo@v1.2.3');
      expect((source.ref, source.only), ('v1.2.3', null));
      source = parse('owner/repo@pdf');
      expect((source.ref, source.only), (null, 'pdf'));
      source = parse('owner/repo@$commit');
      expect((source.ref, source.only), (commit, null));
      source = parse('github:owner/repo');
      expect(source.repo, 'https://github.com/owner/repo.git');
      source = parse('gitlab:owner/repo');
      expect(source.repo, 'https://gitlab.com/owner/repo.git');
    });

    test('a URL, a tree URL and an scp-like address', () {
      var source = parse('https://github.com/addyosmani/web-quality-skills');
      expect((source.repo, source.ref, source.path), ('https://github.com/addyosmani/web-quality-skills', null, ''));
      source = parse('https://github.com/o/r/tree/main/skills/pdf');
      expect((source.repo, source.ref, source.path), ('https://github.com/o/r', 'main', 'skills/pdf'));
      source = parse('https://gitlab.com/o/r/-/tree/v1.0.0');
      expect((source.repo, source.ref, source.path), ('https://gitlab.com/o/r', 'v1.0.0', ''));
      source = parse('https://example.test/team/skills.git@v2.0.0');
      expect((source.repo, source.ref), ('https://example.test/team/skills.git', 'v2.0.0'));
      source = parse('git+https://example.test/team/skills.git#release');
      expect((source.repo, source.ref), ('https://example.test/team/skills.git', 'release'));
      source = parse('git@github.com:o/r.git');
      expect((source.repo, source.ref, source.only), ('git@github.com:o/r.git', null, null));
      source = parse('git@github.com:o/r.git@pdf');
      expect((source.repo, source.only), ('git@github.com:o/r.git', 'pdf'));
      source = parse('https://example.test:8443/o/r.git');
      expect(source.repo, 'https://example.test:8443/o/r.git');
    });

    test('a folder on this computer', () {
      for (final folder in ['/home/ana/skills', './skills', '../skills', r'C:\skills', 'C:/skills', 'file:///home/ana/skills']) {
        final source = parse(folder);
        expect((source.repo, source.local, source.ref, source.path), (folder, true, null, ''), reason: folder);
      }
    });

    test('what Canoa does not read is refused with the reason', () {
      expectRefused('', 'Name a source');
      expectRefused('owner', 'owner/repo');
      expectRefused('owner/repo/../x', '..');
      expectRefused('owner/repo#', 'Nothing follows');
      expectRefused('owner/repo@a b');
      expectRefused('https://user:secret@example.test/o/r.git', 'password');
      expectRefused('https://example.test/o/SKILL.md', 'SKILL.md');
      expectRefused('https://example.test/o/skills.tar.gz', 'archive');
      expectRefused('https://skills.example.test/.well-known/agent-skills/index.json', 'well-known');
      expectRefused('two\nlines');
    });

    test('a label is a name canoa.cfg takes', () {
      expect(parse('anthropics/skills').label, 'anthropics-skills');
      expect(parse('https://github.com/addyosmani/web-quality-skills').label, 'addyosmani-web-quality-skills');
      expect(parse('/home/ana/1skills').label, startsWith('skills-'));
      expect(CanoaConfig.isCatalogLabel(parse('git@github.com:o/r.git').label), isTrue);
    });
  });

  group('canoa.cfg [store.skills.catalogs]', () {
    test('Anthropic\'s skills are first, and a label of its own or false replaces or removes it', () {
      expect(CanoaConfig.parse('').skillSources.keys, ['anthropics']);
      expect(CanoaConfig.parse('[store.skills.catalogs]\nweb = "addyosmani/web-quality-skills"\n').skillSources.keys, ['anthropics', 'web']);
      expect(CanoaConfig.parse('[store.skills.catalogs]\nanthropics = false\n').skillSources, isEmpty);
      final replaced = CanoaConfig.parse('[store.skills.catalogs]\nanthropics = "owner/mine"\n').skillSources;
      expect(replaced['anthropics']!.repo, 'https://github.com/owner/mine.git');
    });

    test('a source is added and removed, and the default stays gone', () {
      var config = CanoaConfig.parse('');
      config = config.withSkillSource('web', 'addyosmani/web-quality-skills');
      expect(config.skillSources.keys, ['anthropics', 'web']);
      expect(CanoaConfig.parse(config.encode()).skillSources['web']!.repo, typed);
      config = config.withoutSkillSource('anthropics');
      expect(config.skillSources.keys, ['web']);
      expect(CanoaConfig.parse(config.encode()).skillSources.keys, ['web'], reason: 'false is written, so a restart does not bring it back');
      config = config.withoutSkillSource('web');
      expect(config.skillSources, isEmpty);
    });

    test('a store limit saved beside them keeps them', () {
      final config = CanoaConfig.parse('[store]\ncatalog_max_entries = 3000\n[store.skills.catalogs]\nweb = "o/r"\n');
      expect(config.store.catalogMaxEntries, 3000);
      expect(config.withSkillSource('two', 'o/two').store.catalogMaxEntries, 3000);
    });

    test('a label or source that is wrong refuses the file', () {
      for (final text in [
        '[store.skills.catalogs]\n"1bad" = "o/r"\n',
        '[store.skills.catalogs]\nweb = 3\n',
        '[store.skills.catalogs]\nweb = "https://example.test/SKILL.md"\n',
        '[store]\nskill_sources = "o/r"\n',
      ]) {
        expect(() => CanoaConfig.parse(text), throwsFormatException, reason: text);
      }
    });
  });

  group('the Skills page', () {
    late MemoryOverlays overlays;
    late List<CanoaConfig> saved;
    late CanoaConfig config;

    SkillScan scanOf(List<ScannedSkill> skills, {int skipped = 0}) => SkillScan(commit: commit, skills: skills, skipped: skipped);

    const pdf = ScannedSkill(name: 'pdf', description: 'Reads and fills PDFs.', path: 'skills/pdf');
    const docx = ScannedSkill(name: 'docx', description: 'Makes Word documents.', path: 'skills/docx');

    setUp(() {
      overlays = world()
        ..scans[anthropics] = scanOf([docx, pdf], skipped: 1)
        ..scans[typed] = scanOf([const ScannedSkill(name: 'performance', description: 'Makes pages fast.', path: 'skills/performance')])
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
          fetchRegistry: (uri) async => '{}',
          findProgram: (program) async => '/usr/bin/$program',
        ),
      ));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-tab-skills')));
      await tester.pumpAndSettle();
      await tapKey(tester, 'skills-discover');
    }

    testWidgets('the default source is listed, and nothing is fetched until it is chosen', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('skills-source-anthropics')), findsOneWidget);
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget, reason: 'the catalogs are what Discover starts with');
      expect(overlays.scanned, isEmpty);

      await tapKey(tester, 'skills-source-anthropics');
      expect(overlays.scanned, ['$anthropics@HEAD#']);
      expect(overlays.scannedOwn, [false], reason: 'the default source is fetched with none of the person\'s git settings');
      expect(find.byKey(const Key('store-source-skill-anthropics-pdf')), findsOneWidget);
      expect(find.byKey(const Key('store-source-skill-anthropics-docx')), findsOneWidget);
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing);
      expect(find.textContaining('none of your git settings'), findsOneWidget);
      expect(find.textContaining('Read at commit $commit'), findsOneWidget);
      expect(find.byKey(const Key('skills-source-left-out')), findsOneWidget);
      expect(overlays.skillFetches, isEmpty, reason: 'reading a source installs nothing');

      await tapKey(tester, 'skills-from-catalogs');
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget);
    });

    testWidgets('a skill of a source is reviewed with the commit that was read, and fetched only on a yes', (tester) async {
      await open(tester);
      await tapKey(tester, 'skills-source-anthropics');
      await tapKey(tester, 'store-source-skill-install-anthropics-pdf');
      expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
      expect(find.textContaining('$commit#skills/pdf'), findsWidgets);
      expect(find.textContaining('unpinned'), findsWidgets);
      expect(overlays.skillFetches, isEmpty);
      await tapKey(tester, 'store-skill-install-confirm');
      expect(overlays.skillFetches, ['$anthropics@$commit#skills/pdf']);
      expect(overlays.skillFetchOwn, [false]);
      expect(overlays.skills.keys, ['pdf']);
      final copy = overlays.skills['pdf']!;
      expect(copy.source, '$anthropics@$commit#skills/pdf');
      expect(copy.chosen, isTrue);
      expect(saved.last.loadoutSkills, {'pdf'});
      expect(find.byKey(const Key('skill-installed-pdf')), findsOneWidget);
      expect(find.textContaining('from a source you named'), findsOneWidget);
    });

    testWidgets('a source the person adds is kept in canoa.cfg, read with their own settings and removable', (tester) async {
      await open(tester);
      await tester.enterText(find.byKey(const Key('skills-source-address')), 'https://example.test/SKILL.md');
      await tapKey(tester, 'skills-source-add');
      expect(find.textContaining('A link to one SKILL.md or an archive is not supported'), findsOneWidget);
      expect(saved, isEmpty);

      await tester.enterText(find.byKey(const Key('skills-source-address')), 'addyosmani/web-quality-skills');
      await tapKey(tester, 'skills-source-add');
      expect(saved.last.skillSources.keys, ['anthropics', 'addyosmani-web-quality-skills']);
      expect(CanoaConfig.parse(saved.last.encode()).skillSources.keys, ['anthropics', 'addyosmani-web-quality-skills'], reason: 'it is kept in the file');
      expect(overlays.scanned, isEmpty, reason: 'adding a source does not read it');

      await tapKey(tester, 'skills-source-addyosmani-web-quality-skills');
      expect(overlays.scanned, ['$typed@HEAD#']);
      expect(overlays.scannedOwn, [true]);
      expect(find.textContaining('You typed this source'), findsOneWidget);
      expect(find.byKey(const Key('store-source-skill-addyosmani-web-quality-skills-performance')), findsOneWidget);

      await tapKey(tester, 'skills-source-remove');
      expect(saved.last.skillSources.keys, ['anthropics']);
      expect(find.byKey(const Key('skills-source-addyosmani-web-quality-skills')), findsNothing);
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget, reason: 'removing the source it was reading puts Discover back on the catalogs');
    });

    testWidgets('a skill from a source the person typed is fetched with their own settings', (tester) async {
      overlays.putSkill('$typed@$commit#skills/performance', {'SKILL.md': '---\nname: performance\ndescription: Makes pages fast.\n---\nBody.\n'});
      await open(tester, text: '$withCatalog[store.skills.catalogs]\nweb = "addyosmani/web-quality-skills"\n');
      await tapKey(tester, 'skills-source-web');
      await tapKey(tester, 'store-source-skill-install-web-performance');
      await tapKey(tester, 'store-skill-install-confirm');
      expect(overlays.skillFetchOwn, [true]);
      expect(overlays.skills.keys, ['performance']);
    });

    testWidgets('removing the default source drops it, and what Discover filters by with it', (tester) async {
      await open(tester);
      await tapKey(tester, 'skills-source-anthropics');
      expect(find.byKey(const Key('store-source-skill-anthropics-pdf')), findsOneWidget);
      await tapKey(tester, 'skills-source-remove');
      expect(saved.last.encode(), contains('anthropics = false'));
      expect(saved.last.skillSources, isEmpty);
      expect(find.byKey(const Key('skills-source-anthropics')), findsNothing);
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget, reason: 'the Store shows the catalogs again, not nothing');
    });

    testWidgets('a source that cannot be read says so, and a skill whose name is not its folder\'s cannot be installed', (tester) async {
      overlays.scans[anthropics] = scanOf([const ScannedSkill(name: 'odd', description: 'Odd one.', path: 'skills/other', problem: 'its name is odd, and the name has to be its folder\'s, other')]);
      overlays.scans.remove(typed);
      await open(tester, text: '$withCatalog[store.skills.catalogs]\nweb = "addyosmani/web-quality-skills"\n');
      await tapKey(tester, 'skills-source-anthropics');
      expect(find.textContaining('Cannot be installed: its name is odd'), findsOneWidget);
      final install = tester.widget<IconButton>(find.byKey(const Key('store-source-skill-install-anthropics-odd')));
      expect(install.onPressed, isNull);
      await tapKey(tester, 'skills-source-web');
      expect(find.byKey(const Key('skills-source-problem')), findsOneWidget);
      expect(find.textContaining('no branch or tag'), findsOneWidget);
    });
  });
}
