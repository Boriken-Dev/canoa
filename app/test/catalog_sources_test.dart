// Settings' Catalog sources page over a scripted host: each Store type's catalogs with where each comes from, how far it is trusted and
// what pins it; a default turned off and on with the one switch the Providers page uses, replaced by an entry of its label, restored,
// and a source added, each as one `store/catalog_set` call. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same
// scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/catalog_specs.dart';
import 'package:canoa/settings/off_switch.dart';
import 'package:canoa/store/catalog_sources.dart';
import 'package:canoa/store/catalog_types.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/scripted_catalogs.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

CatalogSources _sources(ScriptedCatalogs host) => CatalogSources(host.call);

Finder _tile(String type, String label) => find.byKey(Key('catalog-source-$type-$label'));

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/catalog_sources_test.dart')));

  group('the controller', () {
    test('reads each type with the origin, pin and trust of every catalog', () async {
      final host = ScriptedCatalogs()..table['whisper'] = {'mine': {'provider': 'whisper_hf', 'repo': 'me/models', 'trusted': true}};
      final sources = _sources(host);
      await sources.load();
      expect(sources.types.map((type) => type.id), ['overlays', 'whisper']);
      final whisper = sources.types.last;
      expect(whisper.format, 'canoa.models/1');
      final acme = whisper.rows.first;
      expect((acme.label, acme.origin, acme.enabled, acme.pin, acme.trust, acme.from), ('acme', 'default', true, '5359861c', 'pinned', 'snapshot'));
      expect(acme.hasSnapshot, isTrue);
      final mine = whisper.rows.last;
      expect((mine.origin, mine.trusted, mine.trust, mine.spec?.provider), ('added', true, 'unverified', 'whisper_hf'));
      expect(sources.types.first.format, isNull);
      expect(sources.types.first.rows.first.trust, isNull, reason: 'a type the host reads itself has no trust to say');
      sources.dispose();
    });

    test('a refusal is the host\'s words and leaves what was listed', () async {
      final host = ScriptedCatalogs(refuse: 'the label is not a name this host keeps a catalog under');
      final sources = _sources(host);
      await sources.load();
      expect(await sources.set('overlays', 'x y', 'a'), contains('not a name'));
      expect(sources.types, hasLength(2));
      sources.dispose();
    });
  });

  group('the page', () {
    Future<(ScriptedCatalogs, CatalogSources)> open(WidgetTester tester, {ScriptedCatalogs? scripted, Size size = const Size(1100, 1800)}) async {
      final host = scripted ?? ScriptedCatalogs();
      final sources = _sources(host);
      await openSettings(tester, section: 'catalog_sources', size: size, catalogSources: sources);
      await tester.pumpAndSettle();
      return (host, sources);
    }

    testWidgets('lists each type, its catalogs and their badges', (tester) async {
      await open(tester);
      expect(find.byKey(const Key('catalog-type-overlays')), findsOneWidget);
      expect(find.byKey(const Key('catalog-type-whisper')), findsOneWidget);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.text('default')), findsOneWidget);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.text('pinned')), findsOneWidget);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.textContaining('Pinned at 5359861c')), findsOneWidget);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.textContaining('Shipped copy')), findsOneWidget);
      expect(find.descendant(of: _tile('overlays', 'team'), matching: find.text('added')), findsOneWidget);
      expect(find.byKey(const Key('catalog-trust-overlays-team')), findsNothing, reason: 'the host reads overlays itself');
    });

    testWidgets('a default is turned off and on with the switch, as one store/catalog_set call each', (tester) async {
      final (host, _) = await open(tester);
      final switchKey = find.byKey(const Key('catalog-switch-whisper-acme'));
      expect(tester.widget<OffSwitch>(switchKey).on, isTrue);
      await tester.tap(find.descendant(of: switchKey, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2, {'type': 'whisper', 'label': 'acme', 'entry': false});
      expect(tester.widget<OffSwitch>(switchKey).on, isFalse);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.byType(OffBadge)), findsOneWidget);
      expect(find.byKey(const Key('catalog-trust-whisper-acme')), findsNothing, reason: 'a source that is off is not read, so it says nothing of trust');

      await tester.tap(find.descendant(of: switchKey, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2, {'type': 'whisper', 'label': 'acme', 'entry': null});
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.byType(OffBadge)), findsNothing);
    });

    testWidgets('a default is replaced by an entry of its label, drawn replaced, and restored', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('catalog-replace-whisper-acme')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('catalog-entry-dialog')), findsOneWidget);
      expect(tester.widget<TextField>(find.byKey(const Key('catalog-entry-label'))).readOnly, isTrue, reason: 'it replaces the one that has the label');
      await tester.enterText(find.byKey(const Key('catalog-entry-whisper_hf-field-repo')), 'myorg/whisper.cpp-models');
      await tester.enterText(find.byKey(const Key('catalog-entry-pin')), '0f3c9d1a5b7e2c4d6f8091a2b3c4d5e6f7a8b9c0');
      await tester.tap(find.byKey(const Key('catalog-entry-save')));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2, {
        'type': 'whisper',
        'label': 'acme',
        'entry': {'provider': 'whisper_hf', 'repo': 'myorg/whisper.cpp-models', 'pin': '0f3c9d1a5b7e2c4d6f8091a2b3c4d5e6f7a8b9c0'},
      });
      expect(find.byKey(const Key('catalog-entry-dialog')), findsNothing);
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.text('replaced')), findsOneWidget);
      expect(find.byKey(const Key('catalog-switch-whisper-acme')), findsNothing, reason: 'an entry is taken out, not switched off');

      await tester.tap(find.byKey(const Key('catalog-remove-whisper-acme')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('catalog-remove-confirm-acme')));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2, {'type': 'whisper', 'label': 'acme', 'entry': null});
      expect(find.descendant(of: _tile('whisper', 'acme'), matching: find.text('default')), findsOneWidget);
    });

    testWidgets('a source is added: a bare address is a string, a pin or a trusted entry is a table', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('catalog-add-overlays')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('catalog-entry-label')), 'extra');
      await tester.enterText(find.byKey(const Key('catalog-entry-source')), 'https://example.org/e.git@main#catalog.toml');
      await tester.tap(find.byKey(const Key('catalog-entry-save')));
      await tester.pumpAndSettle();
      expect(host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2['entry'], 'https://example.org/e.git@main#catalog.toml');
      expect(_tile('overlays', 'extra'), findsOneWidget);

      await tester.tap(find.byKey(const Key('catalog-add-whisper')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('catalog-entry-trusted')), findsOneWidget, reason: 'a type whose catalogs answer in a format may take files with no checksum');
      await tester.enterText(find.byKey(const Key('catalog-entry-label')), 'mine');
      await tester.tap(find.byKey(const Key('catalog-entry-provider')));
      await tester.pumpAndSettle();
      await tester.tap(find.textContaining('A Hugging Face repository').last);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('catalog-entry-whisper_hf-field-repo')), 'me/models');
      await tester.tap(find.byKey(const Key('catalog-entry-trusted')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('catalog-entry-save')));
      await tester.pumpAndSettle();
      final entry = host.calls.where((call) => call.$1 == 'store/catalog_set').last.$2['entry']! as Map;
      expect(entry['trusted'], isTrue);
      expect(find.descendant(of: _tile('whisper', 'mine'), matching: find.text('unverified')), findsOneWidget);
      expect(find.byKey(const Key('catalog-trust-whisper-mine')), findsOneWidget);
    });

    testWidgets('what the host refuses is said in the dialog and the dialog stays open', (tester) async {
      final (_, _) = await open(tester, scripted: ScriptedCatalogs(refuse: 'the label is not a name this host keeps a catalog under'));
      await tester.tap(find.byKey(const Key('catalog-add-overlays')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('catalog-entry-label')), 'a b');
      await tester.enterText(find.byKey(const Key('catalog-entry-source')), 'owner/repo');
      await tester.tap(find.byKey(const Key('catalog-entry-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('catalog-entry-dialog')), findsOneWidget);
      expect(find.textContaining('not a name this host keeps'), findsOneWidget);
    });

    testWidgets('when it was read is drawn from what the host kept', (tester) async {
      final host = ScriptedCatalogs();
      final sources = _sources(host);
      await sources.load();
      final at = DateTime(2026, 6, 15, 9, 5).millisecondsSinceEpoch;
      final row = CatalogRow(label: 'read-one', spec: const CatalogSpec('read', {'source': 'x'}), origin: 'added', trust: 'verified', fetched: at, from: 'cache');
      final type = CatalogType(id: 'whisper', title: 'Whisper', words: const CatalogWords(noun: 'source', plural: 'sources', lists: 'models'), format: 'canoa.models/1', rows: [row]);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: _Shown(sources: _Fixed([type])))));
      await tester.pumpAndSettle();
      expect(find.textContaining('Read 2026-06-15 09:05'), findsOneWidget);
      expect(find.text('verified'), findsOneWidget);
    });
  });

  for (final (label, size) in [('desktop', const Size(1280, 900)), ('phone', const Size(390, 900))]) {
    testWidgets('golden at $label width', (tester) async {
      final host = ScriptedCatalogs()
        ..table['whisper'] = {
          'mine': {'provider': 'whisper_hf', 'repo': 'me/models', 'pin': '9d8c7b6a5f4e3d2c1b0a99887766554433221100', 'trusted': true},
          'mirror': {'provider': 'whisper_hf', 'repo': 'corp/whisper-mirror'},
        }
        ..table['overlays'] = {'boriken': false, 'team': 'https://example.org/o.git@main#catalog.toml'};
      final sources = _sources(host);
      await openSettings(tester, section: 'catalog_sources', size: size, catalogSources: sources);
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/catalog_sources_$label.png'));
    });
  }
}

/// A [CatalogSources] that holds the types a test gives it.
final class _Fixed extends CatalogSources {
  _Fixed(this._held) : super((method, params) async => {'types': const []});

  final List<CatalogType> _held;

  @override
  List<CatalogType> get types => _held;

  @override
  bool get loaded => true;
}

class _Shown extends StatelessWidget {
  const _Shown({required this.sources});

  final CatalogSources sources;

  @override
  Widget build(BuildContext context) => CatalogSourcesSection(sources: sources);
}
