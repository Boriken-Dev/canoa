// The store reads its catalogs from the host's `store/index`: one call, the same over any host, and nothing fetched by the app.
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_index.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

final _png = base64Encode(png(32, 32));

/// What the host answers: a catalog with an overlay in its own repository, one elsewhere, a skill and a refused entry, and a
/// source that failed.
Map<String, Object?> answer() => {
  'schema': 1,
  'sources': [
    {
      'id': 'catalog:team', 'kind': 'catalog', 'label': 'team', 'spec': 'https://example.invalid/team.git@main#catalog.toml',
      'commit': 'c0ffee0000000000000000000000000000000000', 'checked': 1, 'stale': false, 'name': 'Team catalog',
      'description': 'What the team shares', 'entries': 4,
    },
    {
      'id': 'catalog:gone', 'kind': 'catalog', 'label': 'gone', 'spec': 'https://example.invalid/gone.git@main#c.toml',
      'commit': '', 'stale': true, 'problem': 'The source has no branch or tag of that name', 'entries': 0,
    },
    {'id': 'programs', 'kind': 'programs', 'stale': false, 'entries': 0},
  ],
  'entries': [
    {
      'id': 'team:overlay:review', 'kind': 'overlay', 'catalog': 'team', 'name': 'review', 'summary': 'Review conventions.',
      'source': 'https://example.invalid/team.git@main#overlays/review',
      'origin': {'kind': 'git', 'location': 'https://example.invalid/team.git', 'ref': 'main', 'path': 'overlays/review'},
      'tags': ['review', 'team'], 'requirements': ['base'], 'pin': null, 'homepage': 'https://example.invalid/review',
      'contents': {'skills': ['lint'], 'hooks': [], 'env_scripts': [], 'mcp_servers': []},
      'icon': {'file': 'icon-0.png', 'format': 'png', 'width': 32, 'height': 32, 'bytes': 10, 'data': _png},
    },
    {
      'id': 'team:overlay:docs', 'kind': 'overlay', 'catalog': 'team', 'name': 'docs', 'summary': null,
      'source': 'https://example.invalid/docs.git@v1',
      'origin': {'kind': 'git', 'location': 'https://example.invalid/docs.git', 'ref': 'v1', 'path': null},
      'tags': [], 'pin': 'sha256:${'a' * 64}', 'icon': {'file': 'icon-1.png', 'format': 'png', 'bytes': 99999999},
    },
    {
      'id': 'team:skill:pdf', 'kind': 'skill', 'catalog': 'team', 'name': 'pdf', 'summary': 'Read PDFs',
      'source': 'https://example.invalid/skills.git@main#pdf',
      'origin': {'kind': 'git', 'location': 'https://example.invalid/skills.git', 'ref': 'main', 'path': 'pdf'}, 'tags': [],
    },
    {
      'id': 'team:overlay:Not A Name', 'kind': 'overlay', 'catalog': 'team', 'name': 'Not A Name', 'origin': null,
      'problem': 'The catalog\'s entry was refused (invalid-name)',
    },
    {'id': 'programs:program:claude_store:claude', 'kind': 'program', 'catalog': 'programs', 'name': 'Claude Code'},
  ],
};

void main() {
  group('catalogsFromIndex', () {
    test('gives the overlays, skills, refusals, icons and failures of each source, in the host\'s order', () {
      final catalogs = catalogsFromIndex(answer());
      expect([for (final each in catalogs) each.label], ['team', 'gone'], reason: 'programs are not a catalog');

      final team = catalogs.first;
      expect(team.problem, isNull);
      expect(team.commit, 'c0ffee0000000000000000000000000000000000');
      expect(team.source.spec, 'https://example.invalid/team.git@main#catalog.toml');
      final store = team.catalog!;
      expect((store.name, store.description), ('Team catalog', 'What the team shares'));
      expect(store.entries.keys, ['docs', 'review']);
      expect(store.skills.keys, ['pdf']);
      expect(store.refused, [('Not A Name', 'The catalog\'s entry was refused (invalid-name)')]);

      final review = store.entries['review']!;
      expect(review.source.kind, 'git');
      expect(displaySource(review.source), 'https://example.invalid/team.git@main#overlays/review');
      expect(review.about['summary'], 'Review conventions.');
      expect(review.about['tags'], ['review', 'team']);
      expect(review.about['homepage'], 'https://example.invalid/review');
      expect(review.about['icon'], 'icon-0.png');
      expect(team.icons['review']!.bytes, png(32, 32));
      expect(installSource(review).path, 'overlays/review');

      final docs = store.entries['docs']!;
      expect(docs.pin, 'sha256:${'a' * 64}');
      expect(installSource(docs).reference, 'v1');
      expect(team.icons['docs']!.bytes, isNull);
      expect(team.icons['docs']!.refused, 'icon-unavailable', reason: 'the host kept the icon within its budget and left this one');
      expect(store.skills['pdf']!.source.path, 'pdf');
    });

    test('a source that failed with nothing indexed is a catalog with its reason', () {
      final gone = catalogsFromIndex(answer()).last;
      expect(gone.catalog, isNull);
      expect(gone.problem, 'The source has no branch or tag of that name');
    });

    test('a source that failed but still has its last index keeps the entries', () {
      final stale = answer();
      (stale['sources']! as List).first['problem'] = 'git ran too long and was ended';
      final team = catalogsFromIndex(stale).first;
      expect(team.problem, isNull);
      expect(team.catalog!.entries.keys, ['docs', 'review']);
    });

    test('anything else is no catalog', () {
      expect(catalogsFromIndex(null), isEmpty);
      expect(catalogsFromIndex({'sources': [{'kind': 'catalog', 'label': 'x'}]}), isEmpty, reason: 'a source with no spec');
    });
  });

  group('ControlStoreIndex', () {
    test('asks store/index with icons, and again with refresh when told to', () async {
      final calls = <(String, Map<String, Object?>)>[];
      final index = ControlStoreIndex((method, params) async {
        calls.add((method, params));
        return answer();
      });
      expect((await index.catalogs()).length, 2);
      await index.catalogs(refresh: true);
      expect([for (final call in calls) call.$1], ['store/index', 'store/index']);
      expect(calls[0].$2, {'icons': true});
      expect(calls[1].$2, {'icons': true, 'refresh': true});
    });

    test('a refusal of the host is an OverlayException saying why', () async {
      final index = ControlStoreIndex((method, params) async => throw const ControlFailed(-32001, 'this key has no overlay.read grant'));
      await expectLater(index.catalogs(), throwsA(isA<OverlayException>().having((e) => e.message, 'message', contains('overlay.read'))));
    });
  });

  group('the page', () {
    late MemoryOverlays overlays;
    late List<bool> asked;

    Future<void> open(WidgetTester tester, CatalogIndex index) async {
      await tester.binding.setSurfaceSize(const Size(1280, 1600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        home: StoreSection(
        readAhead: false,
          config: CanoaConfig.parse('[catalogs]\nteam = "https://example.invalid/team.git@main#catalog.toml"\n'),
          home: MemoryHome(overlays),
          index: index,
          onSave: (changed) async => changed,
        ),
      ));
      await tester.pumpAndSettle();
    }

    setUp(() {
      overlays = MemoryOverlays();
      asked = [];
    });

    testWidgets('lists what the host indexed and fetches nothing itself', (tester) async {
      await open(tester, ({bool refresh = false}) async {
        asked.add(refresh);
        return catalogsFromIndex(answer());
      });
      expect(find.text('Discover 2'), findsOneWidget, reason: 'two overlays are offered');
      expect(asked, [false]);
      expect(overlays.fetched, isEmpty, reason: 'the app reads no catalog itself');
      await tester.tap(find.byKey(const Key('overlays-discover')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-entry-team-review')), findsOneWidget);
      expect(find.byKey(const Key('store-entry-team-docs')), findsOneWidget);
    });

    testWidgets('Refresh asks the host to ask its sources again', (tester) async {
      await open(tester, ({bool refresh = false}) async {
        asked.add(refresh);
        return catalogsFromIndex(answer());
      });
      await tester.tap(find.byKey(const Key('store-refresh')));
      await tester.pumpAndSettle();
      expect(asked, [false, true]);
    });

    testWidgets('a host that cannot answer leaves the catalogs empty and says so', (tester) async {
      await open(tester, ({bool refresh = false}) async => throw const OverlayException('this key has no overlay.read grant'));
      expect(find.byKey(const Key('loadout-problem')), findsOneWidget);
      expect(find.textContaining('overlay.read'), findsOneWidget);
      expect(find.text('Discover 0'), findsOneWidget);
    });

    testWidgets('a host that is not there, which is no refusal, does not take the page down', (tester) async {
      await open(tester, ({bool refresh = false}) async => throw StateError('the host is not running'));
      expect(find.byKey(const Key('loadout-problem')), findsOneWidget);
      expect(find.textContaining('the host is not running'), findsOneWidget);
    });
  });
}
