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
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/profile/profiles_panel.dart';
import 'package:canoa/profile/skill_copies.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:canoa/store/skills.dart';
import 'package:canoa/store/store.dart';
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
    List<(String, aos.Catalog)>? catalogs, Map<String, InstalledManifest> others = const {},
    Map<String, String> skillPins = const {}}) =>
    previewOverlay(overlays, name, GitOverlay(repo: repo, reference: 'main'), os: 'linux', shared: const {},
        catalogs: catalogs ?? [('main', mainCatalog)], others: others, skillPins: skillPins);

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

  group('Loadout', () {
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
        home: LoadoutPage(
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

    Future<void> tapKey(WidgetTester tester, String key) async {
      await tester.ensureVisible(find.byKey(Key(key)));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(Key(key)));
      await tester.pumpAndSettle();
    }

    testWidgets('the review names the skills with their sources, one yes installs them, and Skills shows who names each',
        (tester) async {
      await open(tester);
      await tapKey(tester, 'loadout-tab-skills');
      await tapKey(tester, 'skills-discover');
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsOneWidget, reason: 'the catalog offers a skill');

      await tapKey(tester, 'loadout-tab-overlays');
      await tapKey(tester, 'overlays-discover');
      await tapKey(tester, 'store-entry-main-team');
      await tapKey(tester, 'store-fetch');
      final shown = shownText(tester);
      expect(shown, contains('Skills from elsewhere'));
      expect(shown, contains('$skillsHost/tools.git#skills/review'));
      expect(shown, contains('pinned ${pinOf('review', reviewV1)}'));
      expect(shown, contains('Nothing has been fetched yet'));
      expect(overlays.skillFetches, isEmpty);

      await tapKey(tester, 'store-install');
      expect(overlays.skills.keys, containsAll(['pdf', 'review']));
      expect(find.byKey(const Key('store-skill-outcome-review')), findsOneWidget);

      await tester.pageBack();
      await tester.pumpAndSettle();
      await tapKey(tester, 'loadout-tab-skills');
      await tapKey(tester, 'skills-installed');
      final card = find.byKey(const Key('skill-installed-review'));
      expect(find.descendant(of: card, matching: find.text('Comes with team')), findsOneWidget);
      await tapKey(tester, 'skills-discover');
      expect(find.byKey(const Key('store-offered-skill-main-pdf')), findsNothing, reason: 'pdf is here now');
      expect(tester.takeException(), isNull);
    });

    testWidgets('the sync dialog names the skills with their sources before the one yes', (tester) async {
      final shown = await preview(overlays);
      await tester.pumpWidget(MaterialApp(
          home: Scaffold(body: OverlayUpdateDialog(update: shown.update, skills: shown.skills))));
      expect(find.byKey(const Key('overlay-skills')), findsOneWidget);
      final text = shownText(tester);
      expect(text, contains('$skillsHost/tools.git#skills/review'));
      expect(text, contains('$skillsHost/pdf.git'));
      expect(text, contains('Fetching runs nothing'));
      expect(overlays.skillFetches, isEmpty);
    });

    testWidgets('a skill the catalog offers is installed by choice after its source and pin are shown', (tester) async {
      await open(tester);
      await tapKey(tester, 'loadout-tab-skills');
      await tapKey(tester, 'skills-discover');
      await tapKey(tester, 'store-skill-install-main-pdf');
      expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
      expect(shownText(tester), contains('$skillsHost/pdf.git'));
      expect(overlays.skillFetches, isEmpty, reason: 'shown before it is fetched');
      await tapKey(tester, 'store-skill-install-confirm');
      expect(overlays.skills['pdf']!.chosen, isTrue);
      final card = find.byKey(const Key('skill-installed-pdf'));
      expect(find.descendant(of: card, matching: find.text('Chosen by you')), findsOneWidget);
    });
  });


  group('one holder for a name, whichever overlay is installed first', () {
    const baseRepo = 'https://example.invalid/base.git';
    final config = CanoaConfig.parse('');

    MemoryOverlays withBase() => world(pdf: false)
      ..put(baseRepo, {'overlay.cfg': 'name = "base"\n', 'skills/review/SKILL.md': skillMd('review')});

    Future<OverlayPreview> previewAgainst(MemoryOverlays overlays, CanoaConfig installed, String name, String repo) async =>
        preview(overlays, name: name, repo: repo, catalogs: [], others: await installedManifests(MemoryHome(overlays), installed));

    test('a fetched skill does not take the name of an installed overlay\'s bundled skill: shown, not fetched, not delivered',
        () async {
      final overlays = withBase();
      final first = await installPreview(overlays, config,
          await preview(overlays, name: 'base', repo: baseRepo, catalogs: []), catalog: 'main');
      final shown = await previewAgainst(overlays, first, 'team', teamRepo);
      final item = shown.skills!.items.single;
      expect(item.status, SkillStatus.shadowed);
      expect(item.why, contains('base'));
      final report = SkillInstallReport();
      await installPreview(overlays, first, shown, catalog: 'main', report: report);
      expect(report.of('review')!.result, SkillResult.notDelivered);
      expect(overlays.skillFetches, isEmpty, reason: 'a shadowed skill is not fetched');
      expect(overlays.skills, isEmpty);
    });

    test('an overlay installed later that bundles the name wins it, and the fetched copy of the earlier one is not delivered',
        () async {
      final overlays = withBase();
      var changed = await installPreview(overlays, config, await preview(overlays, catalogs: []), catalog: 'main');
      final shown = await previewAgainst(overlays, changed, 'base', baseRepo);
      expect(shown.skills!.items, isEmpty, reason: 'the incoming overlay names none from elsewhere');
      changed = await installPreview(overlays, changed, shown, catalog: 'main');
      expect(overlays.skills.keys, ['review'], reason: 'the copy stays until its overlay is removed');

      final layers = await profileLayers(overlays: changed.overlays, user: const Profile(), agents: const {}, agent: 'Claude',
          home: MemoryHome(overlays));
      Layer layer(String name) => layers.layers.firstWhere((layer) => layer.id.name == name);
      expect(layer('team').profile.skills.keys, isNot(contains('review')));
      expect(layer('base').profile.skills.keys, contains('review'));
      expect(layers.effective.skills['review']!.layer.name, 'base');
      expect(layers.notes.map((note) => note.text), contains(startsWith('Not delivered: review')));
    });
  });

  group('what the review and the install do not leave to chance', () {
    final config = CanoaConfig.parse('');

    test('a copy damaged on disk is shown as an update and not fetched again until the yes', () async {
      final overlays = world(pdf: false);
      await installPreview(overlays, config, await preview(overlays, catalogs: []), catalog: 'main');
      final copy = overlays.skills['review']!;
      overlays.skills['review'] = SkillCopy(name: copy.name, source: copy.source, pin: copy.pin, digest: copy.digest,
          overlays: copy.overlays, files: {...copy.files, 'notes.md': utf8.encode('damaged\n')});
      final fetched = overlays.skillFetches.length;
      final shown = await preview(overlays, catalogs: []);
      expect(shown.skills!.items.single.status, SkillStatus.update);
      expect(shown.skills!.items.single.why, contains('not what was accepted'));
      expect(overlays.skillFetches.length, fetched);
      await installPreview(overlays, config, shown, catalog: 'main');
      expect(overlays.skillFetches.length, fetched + 1, reason: 'the yes fetches it again');
      expect(utf8.decode(overlays.skills['review']!.files['notes.md']!), 'one\n');
    });

    test('a copy damaged after the review was shown kept is reported failed, never fetched silently', () async {
      final overlays = world(pdf: false);
      await installPreview(overlays, config, await preview(overlays, catalogs: []), catalog: 'main');
      final shown = await preview(overlays, catalogs: []);
      expect(shown.skills!.items.single.status, SkillStatus.unchanged);
      final copy = overlays.skills['review']!;
      overlays.skills['review'] = SkillCopy(name: copy.name, source: copy.source, pin: copy.pin, digest: copy.digest,
          overlays: copy.overlays, files: {...copy.files, 'notes.md': utf8.encode('damaged\n')});
      final fetched = overlays.skillFetches.length;
      final report = await installSkills(overlays, shown.skills!);
      expect(report.of('review')!.result, SkillResult.failed);
      expect(overlays.skillFetches.length, fetched);
    });

    test('a skill the person chose is kept when an overlay names another source for it', () async {
      final overlays = world(pdf: false, names: ['pdf'])
        ..put(teamRepo, {
          'overlay.cfg': 'name = "team"\nskills = [{ name = "pdf", source = "$skillsHost/pdf-fork.git" }]\n',
        })
        ..putSkill('$skillsHost/pdf-fork.git', {'SKILL.md': skillMd('pdf', 'x: y\n')});
      final item = reviewChosenSkill(overlays, const {}, 'main', 'pdf', mainCatalog.skills!.entries['pdf']!);
      await installChosenSkill(overlays, item);
      final shown = await preview(overlays, catalogs: []);
      expect(shown.skills!.items.single.status, SkillStatus.refused);
      expect(shown.skills!.items.single.why, contains('you chose'));
      await installPreview(overlays, config, shown, catalog: 'main');
      expect(overlays.skills['pdf']!.source, '$skillsHost/pdf.git');
      expect(overlays.skills['pdf']!.chosen, isTrue);
    });

    test('an insecure transport and a private address are shown in the review', () async {
      final overlays = MemoryOverlays()
        ..put(teamRepo, {
          'overlay.cfg': 'name = "team"\nskills = [\n'
              '  { name = "plain", source = "http://skills.example.test/plain.git" },\n'
              '  { name = "lan", source = "https://10.1.2.3/lan.git" },\n]\n',
        });
      final shown = await preview(overlays, catalogs: []);
      final byName = {for (final item in shown.skills!.items) item.name: item};
      expect(byName['plain']!.warnings, ['insecure-transport']);
      expect(byName['lan']!.warnings, ['private-source']);
    });

    test('a catalog that now gives the name is a change the yes accepts', () async {
      final overlays = world();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main');
      final shown = await preview(overlays, catalogs: [('other', mainCatalog)]);
      final item = shown.skills!.items.firstWhere((item) => item.name == 'pdf');
      expect(item.status, SkillStatus.update);
      expect(item.catalog, 'other');
      expect(overlays.skills['pdf']!.catalog, 'main', reason: 'nothing changes before the yes');
    });

    test('a skill whose copy cannot be written is reported failed and the others are installed', () async {
      final overlays = world()..failSave.add('pdf');
      final report = SkillInstallReport();
      await installPreview(overlays, config, await preview(overlays), catalog: 'main', report: report);
      expect(report.of('pdf')!.result, SkillResult.failed);
      expect(report.of('review')!.result, SkillResult.installed);
      expect(overlays.skills.keys, ['review']);
      expect(overlays.records.keys, ['team']);
    });
  });

  group('canoa.cfg [skill_pins]', () {
    final config = CanoaConfig.parse('');
    String cfg(String? entryPin) => 'name = "team"\nskills = [{ name = "review", source = "$skillsHost/tools.git#skills/review"'
        '${entryPin == null ? '' : ', pin = "$entryPin"'} }]\n';

    test('the table is read by lower-case name, and a name or digest that is none is an error', () {
      final read = CanoaConfig.parse('[skill_pins]\nReview = "${pinOf('review', reviewV1).toUpperCase().replaceFirst('SHA256', 'sha256')}"\n');
      expect(read.skillPins.keys, ['review']);
      expect(read.skillPins['review'], pinOf('review', reviewV1));
      expect(() => CanoaConfig.parse('[skill_pins]\nreview = "abc"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[skill_pins]\n"../x" = "$wrongPin"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[skill_pins]\nA = "$wrongPin"\na = "$wrongPin"\n'), throwsFormatException);
      expect(CanoaConfig.parse('[skill_pins]\nreview = "$wrongPin"\n').withCatalog('x', 'https://e.invalid/c.git@main#c.toml')
          .skillPins['review'], wrongPin, reason: 'it survives a save');
    });

    test('a configured pin anchors an unpinned entry, and what is fetched must equal it', () async {
      final overlays = MemoryOverlays()
        ..put(teamRepo, {'overlay.cfg': cfg(null)})
        ..putSkill('$skillsHost/tools.git#skills/review', reviewV1);
      final pins = {'review': pinOf('review', reviewV1)};
      final shown = await preview(overlays, catalogs: [], skillPins: pins);
      final item = shown.skills!.items.single;
      expect(item.pin, pins['review']);
      expect(item.unpinned, isFalse);
      expect(item.configuredPin, pins['review']);
      expect(shown.skills!.plan.unpinned, isNotEmpty, reason: 'the entry itself has none, which the plan says');
      await installPreview(overlays, config, shown, catalog: 'main');
      expect(overlays.skills['review']!.pin, pins['review']);

      final wrong = MemoryOverlays()
        ..put(teamRepo, {'overlay.cfg': cfg(null)})
        ..putSkill('$skillsHost/tools.git#skills/review', reviewV2);
      final report = SkillInstallReport();
      await installPreview(wrong, config, await preview(wrong, catalogs: [], skillPins: pins), catalog: 'main', report: report);
      expect(report.of('review')!.result, SkillResult.refused);
      expect(wrong.skills, isEmpty);
    });

    test('a pin of the entry and a pin in canoa.cfg that differ refuse the skill before any fetch', () async {
      final overlays = MemoryOverlays()
        ..put(teamRepo, {'overlay.cfg': cfg(pinOf('review', reviewV1))})
        ..putSkill('$skillsHost/tools.git#skills/review', reviewV1);
      final shown = await preview(overlays, catalogs: [], skillPins: {'review': wrongPin});
      expect(shown.skills!.items.single.status, SkillStatus.refused);
      expect(shown.skills!.items.single.why, contains('[skill_pins]'));
      await installPreview(overlays, config, shown, catalog: 'main');
      expect(overlays.skillFetches, isEmpty);
    });

    test('a catalog\'s pin and a configured pin must both hold for a skill named without a source', () async {
      final overlays = world(reviewPin: pinOf('review', reviewV1));
      final shown = await preview(overlays, skillPins: {'pdf': wrongPin});
      expect(shown.skills!.items.firstWhere((item) => item.name == 'pdf').status, SkillStatus.refused);
      final chosen = reviewChosenSkill(overlays, const {}, 'main', 'pdf', mainCatalog.skills!.entries['pdf']!,
          pins: {'pdf': wrongPin});
      expect(chosen.status, SkillStatus.refused);
    });

    test('a copy whose digest is not the configured pin is not delivered', () async {
      final overlays = world(pdf: false);
      await installPreview(overlays, config, await preview(overlays, catalogs: []), catalog: 'main');
      final tree = treeOfFiles(overlays.installedFiles['team']!).tree;
      final copies = await overlays.skillCopies();
      expect(overlayLayer(identity: 'team', root: '/o', os: 'linux', tree: tree, fetched: copies,
          skillPins: {'review': pinOf('review', reviewV1)}).profile.skills.keys, contains('review'));
      final refused = overlayLayer(identity: 'team', root: '/o', os: 'linux', tree: tree, fetched: copies,
          skillPins: {'REVIEW'.toLowerCase(): wrongPin});
      expect(refused.profile.skills.keys, isNot(contains('review')));
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

    test('ssh is given no configuration, no key and no agent of the person\'s, only an empty identity', () {
      final environment = store.isolatedEnvironment();
      final empty = environment['GIT_CONFIG_GLOBAL']!;
      expect(environment['GIT_SSH_COMMAND'], OverlayStore.sshCommand(empty));
      final command = OverlayStore.sshCommand('/cache/empty');
      for (final option in const ['-F "/cache/empty"', '-o BatchMode=yes', '-o IdentitiesOnly=yes', '-o IdentityAgent=none',
        '-o IdentityFile="/cache/empty"']) {
        expect(command, contains(option));
      }
      expect(File(empty).readAsStringSync(), isEmpty);
      expect(empty, contains('.skills'), reason: 'the empty file is in the store\'s own directory');
    });

    test('every fetch of a skill checks the objects it receives and follows no redirect', () {
      expect(OverlayStore.isolatedSettings, containsAll(['transfer.fsckObjects=true', 'http.followRedirects=false', 'credential.helper=']));
    });

    test('a record that names a directory outside .skills is never read, and nothing outside it is removed', () async {
      final skills = Directory('${temp.path}/home/overlays/.skills')..createSync(recursive: true);
      final outside = File('${temp.path}/home/overlays/keep.txt')..writeAsStringSync('keep');
      Map<String, Object> record(String name, [List<String> overlays = const ['team']]) => {
        'name': name, 'source': '$skillsHost/tools.git', 'digest': wrongPin, 'overlays': overlays, 'chosen': false,
      };
      File('${skills.path}/x.json').writeAsStringSync(jsonEncode(record('../../x')));
      File('${skills.path}/...json').writeAsStringSync(jsonEncode(record('..')));
      File('${skills.path}/y.json').writeAsStringSync(jsonEncode(record('y', ['../../team'])));
      File('${skills.path}/z.json').writeAsStringSync(jsonEncode(record('z')));
      Directory('${skills.path}/z').createSync();
      File('${skills.path}/z/SKILL.md').writeAsStringSync(skillMd('z'));
      final copies = await store.skillCopies();
      expect(copies.keys, ['z'], reason: 'only a record that names its own file, with overlay names, is read');
      await expectLater(store.removeSkill('..'), throwsA(isA<OverlayException>()));
      await expectLater(store.removeSkill('../keep.txt'), throwsA(isA<OverlayException>()));
      expect(outside.existsSync(), isTrue);
      expect(Directory('${temp.path}/home/overlays').existsSync(), isTrue);
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
