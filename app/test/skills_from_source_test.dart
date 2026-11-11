// An overlay that names skills from elsewhere: planned the way the shared case says (the same skills dotagents gives),
// reviewed before anything is fetched, fetched only on the one yes for the overlay and its skills, held to their pins,
// kept while an overlay still names them, and delivered like a bundled skill. The git tests need git on PATH and fetch
// only from bare repositories made here, a made-up host mapped to them.
@Timeout.factor(4)
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:canoa/profile/overlay_store_io.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/skill_copies.dart';
import 'package:canoa/store/skills.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_page.dart';
import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

const skillsHost = 'https://skills.example.test';
const teamRepo = 'https://example.invalid/team-overlay.git';
const otherRepo = 'https://example.invalid/other-overlay.git';
const wrongPin = 'sha256:abababababababababababababababababababababababababababababababab';

String skillMd(String name, [String more = '']) =>
    '---\nname: $name\ndescription: The $name skill.\n$more---\nBody of $name.\n';

List<(String, Uint8List)> pairs(Map<String, String> files) =>
    [for (final MapEntry(:key, :value) in files.entries) (key, Uint8List.fromList(utf8.encode(value)))];

String pinOf(String name, Map<String, String> files) =>
    '${aos.readFetchedSkill(name, pairs(files), null, skillLimits).digest}';

aos.Catalog catalogOf(String text) =>
    aos.readCatalog(Uint8List.fromList(utf8.encode(text)), 'catalog.toml', null, aos.defaultSettings);

final reviewV1 = {'SKILL.md': skillMd('review'), 'notes.md': 'one\n'};
final reviewV2 = {'SKILL.md': skillMd('review'), 'notes.md': 'two\n'};
final pdfFiles = {'SKILL.md': skillMd('pdf')};

String overlayCfg(String name, {String? reviewPin, List<String> names = const [], bool pdf = false}) => '''
name = "$name"
skills = [
${[
  if (pdf) '  "pdf",',
  for (final other in names) '  "$other",',
  if (reviewPin != null) '  { name = "review", source = "$skillsHost/tools.git#skills/review", pin = "$reviewPin" },',
].join('\n')}
]
''';

MemoryOverlays world({String? reviewPin, List<String> names = const [], bool pdf = true, Map<String, String>? review}) {
  final overlays = MemoryOverlays()
    ..put(teamRepo, {
      'overlay.cfg': overlayCfg('team', reviewPin: reviewPin ?? pinOf('review', reviewV1), names: names, pdf: pdf),
      'skills/notes/SKILL.md': skillMd('notes'),
    })
    ..putSkill('$skillsHost/tools.git#skills/review', review ?? reviewV1)
    ..putSkill('$skillsHost/pdf.git', pdfFiles);
  return overlays;
}

final mainCatalog = catalogOf('[skills]\npdf = { source = "$skillsHost/pdf.git", pin = "${pinOf('pdf', pdfFiles)}" }\n');

Future<OverlayPreview> preview(MemoryOverlays overlays, {String name = 'team', String repo = teamRepo,
    List<(String, aos.Catalog)>? catalogs}) =>
    previewOverlay(overlays, name, GitOverlay(repo: repo, reference: 'main'), os: 'linux', shared: const {},
        catalogs: catalogs ?? [('main', mainCatalog)]);

Layer layerOf(MemoryOverlays overlays, Map<String, SkillCopy> copies, {String name = 'team'}) => overlayLayer(
    identity: name, root: '/o/$name', os: 'linux', tree: treeOfFiles(overlays.installedFiles[name]!).tree, fetched: copies);

Map<String, Object?> planJson(aos.SkillPlan plan) => {
  'skills': [
    for (final skill in plan.skills)
      {
        'name': skill.name,
        'overlay': skill.overlay,
        'origin': skill.origin,
        'source': ?skill.source,
        'pin': ?skill.pin,
        'catalog': ?skill.catalog,
        'catalogs': ?skill.catalogs,
      },
  ],
  'missing': [for (final (name, overlay) in plan.missing) [name, overlay]],
  'ambiguous': [for (final (name, overlay, labels) in plan.ambiguous) [name, overlay, labels]],
  'unpinned': [for (final (name, overlay) in plan.unpinned) [name, overlay]],
  'shadowing': [for (final (name, overlay) in plan.shadowing) [name, overlay]],
  'delivered': plan.delivered,
  'collisions': plan.collisions,
  'fetch': [
    for (final fetch in plan.toFetch)
      {'name': fetch.name, 'overlay': fetch.overlay, 'source': fetch.from.shown(), 'pin': fetch.pin},
  ],
};

void main() {
  group('the shared case', () {
    final cases = (jsonDecode(File('../crates/canoa-host/tests/vectors/skills_from_source.json').readAsStringSync())
        as Map)['cases'] as List;

    test('the cases are all there', () => expect(cases.length, greaterThanOrEqualTo(4)));

    for (final given in cases.cast<Map<String, dynamic>>()) {
      test('${given['name']}', () {
        final manifests = <String, aos.Document>{};
        final trees = <String, aos.OverlayTree>{};
        for (final MapEntry(key: identity, value: files) in (given['overlays'] as Map).entries) {
          final tree = aos.OverlayTree.fromBytes([
            for (final MapEntry(:key, :value) in (files as Map).entries) ('$key', Uint8List.fromList(utf8.encode('$value'))),
          ]);
          trees['$identity'] = tree;
          manifests['$identity'] = aos.readOverlay(tree, aos.defaultSettings).manifest;
        }
        final catalogs = [
          for (final catalog in given['catalogs'] as List)
            (
              '${(catalog as Map)['label']}',
              aos.readCatalog(Uint8List.fromList(utf8.encode('${catalog['text']}')), '${catalog['name']}',
                  catalog['base'] as String?, aos.defaultSettings),
            ),
        ];
        final requested = [for (final name in given['requested'] as List) '$name'];
        final plan = planSkills(manifests, requested, trees, catalogs: catalogs);
        expect(jsonDecode(jsonEncode(planJson(plan))), given['expect']);
      });
    }
  });

  group('installing an overlay that names skills from elsewhere', () {
    final config = CanoaConfig.parse('');

    test('the review names each skill with its source and pin, and fetches none of them', () async {
      final overlays = world();
      final shown = await preview(overlays);
      expect(overlays.skillFetches, isEmpty, reason: 'nothing of a skill is fetched before the yes');
      expect([for (final item in shown.skills!.items) (item.name, item.status, item.source, item.pin, item.catalog)], [
        ('pdf', SkillStatus.fetch, '$skillsHost/pdf.git', pinOf('pdf', pdfFiles), 'main'),
        ('review', SkillStatus.fetch, '$skillsHost/tools.git#skills/review', pinOf('review', reviewV1), null),
      ]);
      expect(shown.skills!.fetches, isTrue);
      expect(overlays.skills, isEmpty);
    });

    test('a skill named by source and one named by name are fetched at their pins on the one yes, and delivered', () async {
      final overlays = world();
      final shown = await preview(overlays);
      final report = SkillInstallReport();
      await installPreview(overlays, config, shown, catalog: 'main', report: report);
      expect(overlays.skillFetches, ['$skillsHost/pdf.git', '$skillsHost/tools.git#skills/review']);
      expect([for (final outcome in report.outcomes) (outcome.name, outcome.result)],
          [('pdf', SkillResult.installed), ('review', SkillResult.installed)]);
      final copy = overlays.skills['review']!;
      expect(copy.digest, pinOf('review', reviewV1));
      expect(copy.pin, pinOf('review', reviewV1));
      expect(copy.source, '$skillsHost/tools.git#skills/review');
      expect(copy.overlays, ['team']);
      expect(copy.files.keys, containsAll(['SKILL.md', 'notes.md']));
      expect(overlays.skills['pdf']!.catalog, 'main');

      final layer = layerOf(overlays, await overlays.skillCopies());
      expect(layer.profile.skills.keys, containsAll(['notes', 'pdf', 'review']));
      expect(layer.aos!.notApplied.where((line) => line.startsWith('skills.')), isEmpty);
      expect(utf8.decode(layer.aos!.blobs['skills/review/SKILL.md']!), skillMd('review'));
      expect(layer.aos!.blobs.keys, contains('skills/pdf/SKILL.md'));
    });

    test('a wrong pin refuses that skill, the overlay still installs, and the skill is not delivered', () async {
      final overlays = world(reviewPin: wrongPin, pdf: false);
      final report = SkillInstallReport();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main', report: report);
      expect(overlays.records.keys, ['team']);
      expect(overlays.skills, isEmpty, reason: 'what failed its pin is not kept');
      expect(report.of('review')!.result, SkillResult.refused);
      expect(report.of('review')!.message, contains('pin'));
      final layer = layerOf(overlays, await overlays.skillCopies());
      expect(layer.profile.skills.keys, ['notes']);
      expect(layer.aos!.notApplied,
          contains('skills.review: from $skillsHost/tools.git#skills/review, not fetched: install or sync the overlay in Canoa'));
    });

    test('a missing and an ambiguous name are reported, and the overlay applies without them', () async {
      final overlays = world(names: ['nothing', 'dup'], pdf: false, reviewPin: pinOf('review', reviewV1));
      final two = [
        ('main', catalogOf('[skills]\ndup = "$skillsHost/dup-a.git"\n')),
        ('extra', catalogOf('[skills]\ndup = "$skillsHost/dup-b.git"\n')),
      ];
      final shown = await preview(overlays, catalogs: two);
      expect([for (final item in shown.skills!.items) (item.name, item.status)],
          [('dup', SkillStatus.ambiguous), ('nothing', SkillStatus.missing), ('review', SkillStatus.fetch)]);
      expect(shown.skills!.items.first.catalogs, ['extra', 'main']);
      final report = SkillInstallReport();
      await installPreview(overlays, config, shown, catalog: 'main', report: report);
      expect(overlays.records.keys, ['team']);
      expect(report.of('dup')!.result, SkillResult.notDelivered);
      expect(report.of('nothing')!.result, SkillResult.notDelivered);
      expect(overlays.skills.keys, ['review']);
      final layer = layerOf(overlays, await overlays.skillCopies());
      expect(layer.profile.skills.keys, containsAll(['notes', 'review']));
      expect(layer.aos!.notApplied,
          containsAll(['skills.dup: a named skill; no catalog resolved it here: install or sync the overlay in Canoa', 'skills.nothing: a named skill; no catalog resolved it here: install or sync the overlay in Canoa']));
    });

    test('a changed pin is an update the review shows and the yes accepts, never applied before', () async {
      final overlays = world();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main');
      expect(overlays.skills['review']!.digest, pinOf('review', reviewV1));
      final fetched = overlays.skillFetches.length;

      overlays.put(teamRepo, {
        'overlay.cfg': overlayCfg('team', reviewPin: pinOf('review', reviewV2), pdf: true),
        'skills/notes/SKILL.md': skillMd('notes'),
      });
      overlays.putSkill('$skillsHost/tools.git#skills/review', reviewV2);
      final shown = await preview(overlays);
      final item = shown.skills!.items.firstWhere((item) => item.name == 'review');
      expect(item.status, SkillStatus.update);
      expect(item.previous!.digest, pinOf('review', reviewV1));
      expect(overlays.skillFetches.length, fetched, reason: 'the review fetches nothing');
      expect(overlays.skills['review']!.digest, pinOf('review', reviewV1), reason: 'and changes nothing');
      expect(shown.skills!.items.firstWhere((item) => item.name == 'pdf').status, SkillStatus.unchanged);

      await installPreview(overlays, config, shown, catalog: 'main');
      expect(overlays.skills['review']!.digest, pinOf('review', reviewV2));
      expect(overlays.skillFetches.length, fetched + 1, reason: 'only the changed skill is fetched again');
    });

    test('before the update is accepted, the old copy is not delivered for the new entry', () async {
      final overlays = world();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main');
      overlays.installedFiles['team'] = {
        'overlay.cfg': utf8.encode(overlayCfg('team', reviewPin: pinOf('review', reviewV2), pdf: true)),
        'skills/notes/SKILL.md': utf8.encode(skillMd('notes')),
      };
      final layer = layerOf(overlays, await overlays.skillCopies());
      expect(layer.profile.skills.keys, isNot(contains('review')));
      expect(layer.aos!.notApplied.single, startsWith('skills.review: its fetched copy is not what was accepted'));
    });

    test('an edited copy is not delivered', () async {
      final overlays = world();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main');
      final copy = overlays.skills['review']!;
      overlays.skills['review'] = SkillCopy(name: copy.name, source: copy.source, pin: copy.pin, digest: copy.digest,
          overlays: copy.overlays, files: {...copy.files, 'notes.md': utf8.encode('edited\n')});
      final layer = layerOf(overlays, await overlays.skillCopies());
      expect(layer.profile.skills.keys, isNot(contains('review')));
    });
  });

  group('removing and choosing', () {
    final config = CanoaConfig.parse('');
    final team = GitOverlay(repo: teamRepo, reference: 'main');

    Future<(MemoryOverlays, CanoaConfig)> installTwo() async {
      final overlays = world(pdf: false)
        ..put(otherRepo, {'overlay.cfg': overlayCfg('other', reviewPin: pinOf('review', reviewV1))});
      var changed = await installPreview(overlays, config, await preview(overlays, catalogs: []), catalog: 'main');
      changed = await installPreview(overlays, changed, await preview(overlays, name: 'other', repo: otherRepo, catalogs: []),
          catalog: 'main');
      return (overlays, changed);
    }

    test('a skill another overlay names stays when the first is removed, and goes, offered, with the last', () async {
      final (overlays, changed) = await installTwo();
      expect(overlays.skills['review']!.overlays, ['other', 'team']);
      expect(overlays.skillFetches.length, 1, reason: 'one skill both name alike is fetched once');

      var offered = <String>[];
      var next = await removeInstalled(overlays, changed, 'team', team, offer: (orphans) async {
        offered = [for (final copy in orphans) copy.name];
        return true;
      });
      expect(offered, isEmpty, reason: 'it is still named, so it is not offered');
      expect(overlays.skills['review']!.overlays, ['other']);

      next = await removeInstalled(overlays, next, 'other', GitOverlay(repo: otherRepo, reference: 'main'),
          offer: (orphans) async {
        offered = [for (final copy in orphans) copy.name];
        return true;
      });
      expect(offered, ['review']);
      expect(overlays.skills, isEmpty);
      expect(next.overlays, isEmpty);
    });

    test('a skill nothing names is kept until the offer is accepted, and listed as named by none', () async {
      final (overlays, changed) = await installTwo();
      await removeInstalled(overlays, changed, 'team', team);
      await removeInstalled(overlays, changed, 'other', GitOverlay(repo: otherRepo, reference: 'main'),
          offer: (_) async => false);
      expect(overlays.skills['review']!.overlays, isEmpty);
      expect([for (final copy in await orphanedSkills(overlays)) copy.name], ['review']);
      expect(await removeOrphans(overlays, ['review']), ['review']);
      expect(overlays.skills, isEmpty);
    });

    test('a skill the person chose is never removed with an overlay, and an overlay naming it is added to its record',
        () async {
      final overlays = world(pdf: true);
      final item = reviewChosenSkill(overlays, await overlays.skillCopies(), 'main', 'pdf', mainCatalog.skills!.entries['pdf']!);
      expect(overlays.skillFetches, isEmpty, reason: 'the review fetches nothing');
      expect(item.status, SkillStatus.fetch);
      expect((await installChosenSkill(overlays, item)).result, SkillResult.installed);
      expect(overlays.skills['pdf']!.chosen, isTrue);
      expect(overlays.skills['pdf']!.overlays, isEmpty);

      var changed = await installPreview(overlays, config, await preview(overlays), catalog: 'main');
      expect(overlays.skills['pdf']!.overlays, ['team']);
      expect(overlays.skills['pdf']!.chosen, isTrue);
      expect(overlays.skillFetches.where((shown) => shown.endsWith('pdf.git')).length, 1, reason: 'already here, not fetched again');

      var offered = <String>[];
      changed = await removeInstalled(overlays, changed, 'team', team, offer: (orphans) async {
        offered = [for (final copy in orphans) copy.name];
        return true;
      });
      expect(offered, ['review'], reason: 'pdf is the person\'s own');
      expect(overlays.skills.keys, ['pdf']);
      expect(await removeOrphans(overlays, ['pdf']), isEmpty, reason: 'nothing offers a chosen skill for removal');
      expect(await releaseChosenSkill(overlays, 'pdf'), isTrue);
      expect(overlays.skills, isEmpty);
    });

    test('a skill another overlay has from another source is refused, not taken from it', () async {
      final (overlays, changed) = await installTwo();
      overlays.put(otherRepo, {
        'overlay.cfg': '''
name = "other"
skills = [{ name = "review", source = "$skillsHost/fork.git#skills/review" }]
''',
      });
      overlays.putSkill('$skillsHost/fork.git#skills/review', reviewV2);
      final shown = await preview(overlays, name: 'other', repo: otherRepo, catalogs: []);
      expect(shown.skills!.items.single.status, SkillStatus.refused);
      final report = SkillInstallReport();
      await installPreview(overlays, changed, shown, catalog: 'main', report: report, movePin: true);
      expect(report.of('review')!.result, SkillResult.notDelivered);
      expect(overlays.skills['review']!.digest, pinOf('review', reviewV1));
    });
  });

  group('the store page', () {
    late MemoryOverlays overlays;

    setUp(() {
      overlays = world(pdf: true)
        ..put('https://example.invalid/catalog.git', {
          'catalog.toml': '''
[overlays.team]
source = "$teamRepo@main"

[skills]
pdf = { source = "$skillsHost/pdf.git", pin = "${pinOf('pdf', pdfFiles)}" }
''',
        });
    });

    Future<void> open(WidgetTester tester) async {
      await tester.binding.setSurfaceSize(const Size(1280, 2400));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        home: StorePage(
          config: CanoaConfig.parse('[catalogs]\nmain = "https://example.invalid/catalog.git@main#catalog.toml"\n'),
          home: MemoryHome(overlays),
          onSave: (changed) async => changed,
        ),
      ));
      await tester.pumpAndSettle();
    }

    String shownText(WidgetTester tester) => [
      for (final widget in tester.widgetList(find.byWidgetPredicate((w) => w is Text || w is SelectableText)))
        widget is Text ? widget.data ?? widget.textSpan?.toPlainText() ?? '' : (widget as SelectableText).data ??
            widget.textSpan!.toPlainText(),
    ].join('\n');

    testWidgets('the review names the skills with their sources, one yes installs them, and the Skills section shows who names each',
        (tester) async {
      await open(tester);
      expect(find.byKey(const Key('store-skills')), findsOneWidget, reason: 'the catalog offers a skill');
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget);

      await tester.tap(find.byKey(const Key('store-entry-main-team')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-fetch')));
      await tester.pumpAndSettle();
      final shown = shownText(tester);
      expect(shown, contains('Skills from elsewhere'));
      expect(shown, contains('$skillsHost/tools.git#skills/review'));
      expect(shown, contains('pinned ${pinOf('review', reviewV1)}'));
      expect(shown, contains('Nothing has been fetched yet'));
      expect(overlays.skillFetches, isEmpty);

      await tester.tap(find.byKey(const Key('store-install')));
      await tester.pumpAndSettle();
      expect(overlays.skills.keys, containsAll(['pdf', 'review']));
      expect(find.byKey(const Key('store-skill-outcome-review')), findsOneWidget);

      await tester.pageBack();
      await tester.pumpAndSettle();
      final tile = tester.widget<ListTile>(find.byKey(const Key('store-skill-review')));
      expect((tile.subtitle! as Text).data, contains('named by team'));
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing, reason: 'pdf is here now');
      expect(tester.takeException(), isNull);
    });

    testWidgets('a skill the catalog offers is installed by choice after its source and pin are shown', (tester) async {
      await open(tester);
      await tester.tap(find.byKey(const Key('store-skill-install-main-pdf')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
      expect(shownText(tester), contains('$skillsHost/pdf.git'));
      expect(overlays.skillFetches, isEmpty, reason: 'shown before it is fetched');
      await tester.tap(find.byKey(const Key('store-skill-install-confirm')));
      await tester.pumpAndSettle();
      expect(overlays.skills['pdf']!.chosen, isTrue);
      final tile = tester.widget<ListTile>(find.byKey(const Key('store-skill-pdf')));
      expect((tile.subtitle! as Text).data, contains('chosen by you'));
    });
  });

  group('from local bare repositories', skip: hasGit ? false : 'git is not on PATH', () {
    late Directory temp;
    late Repo tools, overlayRepo;
    late OverlayStore store;
    final config = CanoaConfig.parse('');

    setUp(() {
      temp = Directory.systemTemp.createTempSync('canoa-skills-');
      final root = temp.path.replaceAll(r'\', '/');
      tools = Repo(root, 'tools');
      tools.write('skills/review/SKILL.md', skillMd('review'));
      tools.write('skills/review/notes.md', 'one\n');
      tools.write('skills/review/node_modules/x/index.js', 'left out\n');
      tools.write('skills/lint/SKILL.md', skillMd('lint', 'allowed-tools: Bash\n'));
      tools.commit('skills');
      store = OverlayStore('$root/home/overlays',
          skillGitConfig: ['url.${tools.location}.insteadOf=$skillsHost/tools.git'],
          skillProtocols: [...OverlayStore.defaultSkillProtocols, 'file']);
      overlayRepo = Repo(root, 'team');
    });
    tearDown(() => temp.deleteSync(recursive: true));

    Future<OverlayPreview> teamPreview(String reviewPin) async {
      overlayRepo.write('overlay.cfg', overlayCfg('team', reviewPin: reviewPin));
      overlayRepo.commit('overlay');
      return previewOverlay(store, 'team', GitOverlay(repo: overlayRepo.location, reference: 'main'), os: 'linux',
          shared: const {});
    }

    test('a skill is fetched from its directory with nothing else, and an entry under an excluded name is left out', () async {
      final files = await store.fetchSkillFiles(aos.Source.parse('$skillsHost/tools.git@main#skills/review'));
      expect({for (final (path, bytes) in files) path: utf8.decode(bytes)},
          {'SKILL.md': skillMd('review'), 'notes.md': 'one\n'});
      final headless = await store.fetchSkillFiles(aos.Source.parse('$skillsHost/tools.git#skills/lint'));
      expect(headless.map((file) => file.$1), ['SKILL.md'], reason: 'no ref is the source\'s HEAD');
    });

    test('a fetch that fails, or names no such directory, is the skill\'s refusal and leaves nothing behind', () async {
      await expectLater(store.fetchSkillFiles(aos.Source.parse('$skillsHost/tools.git@main#skills/none')),
          throwsA(isA<OverlayException>()));
      await expectLater(store.fetchSkillFiles(aos.Source.parse('$skillsHost/nowhere.git@main')),
          throwsA(isA<OverlayException>()));
      final leftover = Directory('${temp.path}/home/overlays/.skills');
      expect(!leftover.existsSync() || leftover.listSync().every((entry) => !entry.path.contains('.fetch-')), isTrue);
    });

    test('git runs without the person\'s credentials, agent, askpass, settings or redirects', () {
      final environment = store.isolatedEnvironment();
      for (final name in const ['SSH_AUTH_SOCK', 'GIT_ASKPASS', 'SSH_ASKPASS', 'GIT_SSH']) {
        expect(environment.keys, isNot(contains(name)));
      }
      expect(environment['GIT_CONFIG_NOSYSTEM'], '1');
      expect(File(environment['GIT_CONFIG_GLOBAL']!).readAsStringSync(), isEmpty);
      expect(environment['GIT_TERMINAL_PROMPT'], '0');
      expect(environment['GIT_ALLOW_PROTOCOL'], contains('https'));
      expect(OverlayStore.defaultSkillProtocols, isNot(contains('file')));
      expect(environment['GIT_SSH_COMMAND'], contains('IdentityAgent=none'));
      expect(environment['HOME'], contains('.skills'));
    });

    test('an overlay installs with its skills at their pins from git, they are recorded, written and delivered', () async {
      final shown = await teamPreview(pinOf('review', {'SKILL.md': skillMd('review'), 'notes.md': 'one\n'}));
      expect(Directory('${temp.path}/home/overlays/.skills/review').existsSync(), isFalse, reason: 'nothing before the yes');
      final report = SkillInstallReport();
      await installPreview(store, config, shown, catalog: 'main', report: report);
      expect(report.of('review')!.result, SkillResult.installed, reason: report.of('review')?.message);
      final copies = await store.skillCopies();
      final copy = copies['review']!;
      expect(copy.files.keys.toSet(), {'SKILL.md', 'notes.md'});
      expect(copy.overlays, ['team']);
      expect(File('${temp.path}/home/overlays/.skills/review/SKILL.md').readAsStringSync(), skillMd('review'));
      expect(Directory('${temp.path}/home/overlays/.skills/review/node_modules').existsSync(), isFalse);
      final record = jsonDecode(File('${temp.path}/home/overlays/.skills/review.json').readAsStringSync()) as Map;
      expect(record['source'], '$skillsHost/tools.git#skills/review');
      expect(record['digest'], copy.pin);
      expect(record['chosen'], false);

      final layer = overlayLayer(identity: 'team', root: store.directoryOf('team'), os: 'linux',
          tree: treeOfFiles({'overlay.cfg': utf8.encode(overlayCfg('team', reviewPin: copy.pin))}).tree, fetched: copies);
      expect(layer.profile.skills.keys, ['review']);
      expect(layer.aos!.blobs['skills/review/notes.md'], utf8.encode('one\n'));
    });

    test('a wrong pin writes nothing of the skill from git, and the overlay is installed', () async {
      final report = SkillInstallReport();
      await installPreview(store, config, await teamPreview(wrongPin), catalog: 'main', report: report);
      expect(report.of('review')!.result, SkillResult.refused);
      expect(await store.skillCopies(), isEmpty);
      expect(Directory('${temp.path}/home/overlays/.skills/review').existsSync(), isFalse);
      expect(store.installed('team'), isNotNull);
    });

    test('a skill whose SKILL.md names another skill is refused even at a pin it matches', () async {
      overlayRepo.write('overlay.cfg',
          'name = "team"\nskills = [{ name = "review", source = "$skillsHost/tools.git#skills/lint", pin = "${pinOf('lint', {
            'SKILL.md': skillMd('lint', 'allowed-tools: Bash\n'),
          })}" }]\n');
      overlayRepo.commit('overlay');
      final shown = await previewOverlay(store, 'team', GitOverlay(repo: overlayRepo.location, reference: 'main'),
          os: 'linux', shared: const {});
      final report = SkillInstallReport();
      await installPreview(store, config, shown, catalog: 'main', report: report);
      expect(report.of('review')!.result, SkillResult.refused);
      expect(await store.skillCopies(), isEmpty);
    });
  });
}
