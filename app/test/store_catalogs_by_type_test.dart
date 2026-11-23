// Each Store type has its own catalogs: one shape in canoa.cfg, `[store.<type>.catalogs.<label>]`, each type's settings listing only
// its own and adding only to its own, and the form that adds one built from what the host says the catalog sources need. The host
// is a stand-in that answers `store/catalogs` as the real one does, from the configuration it is given.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/catalog_specs.dart';
import 'package:canoa/loadout/loadout_controller.dart';
import 'package:canoa/store/catalog_types.dart';
import 'package:canoa/store/store_catalogs.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _everywhere = {
  'id': 'read',
  'title': 'Read an address or a file',
  'types': <String>[],
  'schema': {
    'type': 'object',
    'required': ['source'],
    'properties': {
      'source': {'type': 'string', 'title': 'Source'},
    },
  },
  'form': <String, Object?>{},
};

const _command = {
  'id': 'command',
  'title': 'Run a program',
  'types': <String>[],
  'schema': {
    'type': 'object',
    'required': ['command'],
    'properties': {
      'command': {'type': 'array', 'items': {'type': 'string'}, 'title': 'Program and arguments'},
      'cwd': {'type': ['string', 'null'], 'title': 'Folder'},
      'timeout': {'type': 'integer', 'title': 'Seconds it may run', 'default': 60},
    },
  },
  'form': <String, Object?>{},
};

/// A source an extension adds, which serves only the Skills type.
const _standin = {
  'id': 'standin',
  'title': 'A stand-in',
  'extension': 'standin_ext',
  'types': ['skills'],
  'schema': {
    'type': 'object',
    'required': ['team'],
    'properties': {
      'team': {'type': 'string', 'title': 'Team name', 'description': "Which team's list"},
      'seats': {'type': ['integer', 'null'], 'title': 'Seats'},
      'layout': {'type': ['string', 'null'], 'title': 'Layout', 'enum': ['flat', 'nested']},
      'recent': {'type': ['boolean', 'null'], 'title': 'Recent only'},
      'key': {'type': ['string', 'null'], 'title': 'Key', 'writeOnly': true, 'x-sensitive': true},
    },
  },
  'form': {
    'fields': {
      'team': {'placeholder': 'platform'},
    },
  },
};

const _types = [
  ('overlays', 'Overlays', 'catalog', 'catalogs', 'overlays', 'boriken', defaultCatalogSpec),
  ('skills', 'Skills', 'source', 'sources', 'skills', 'anthropics', 'anthropics/skills'),
  ('mcps', 'MCP servers', 'registry', 'registries', 'MCP servers', 'official', 'https://registry.modelcontextprotocol.io'),
  ('extensions', 'Extensions', 'catalog', 'catalogs', 'extensions', null, null),
];

/// What the host answers for [config]: each type's rows, the default unless the table names its label, and the sources that serve it.
Map<String, Object?> _answer(CanoaConfig config) => {
  'schema': 1,
  'types': [
    for (final (id, title, noun, plural, lists, defaultLabel, defaultSource) in _types)
      {
        'type': id,
        'title': title,
        'words': {'noun': noun, 'plural': plural, 'lists': lists},
        'default': defaultLabel == null ? null : {'label': defaultLabel, 'source': defaultSource},
        if (id == 'skills') 'skills_search': CanoaConfig.defaultSkillsSearch,
        'catalogs': [
          if (defaultLabel != null &&
              !((config.catalogTable(id) as Map?)?.keys.any((key) => '$key'.toLowerCase() == defaultLabel) ?? false))
            {'label': defaultLabel, 'provider': 'read', 'fields': {'source': defaultSource}, 'source': defaultSource, 'default': true},
          for (final MapEntry(:key, :value) in config.catalogSpecs(id).entries)
            {'label': key, 'provider': value.provider, 'fields': value.fields, if (value.source != null) 'source': value.source, 'default': false},
        ],
        'sources': [
          for (final source in [_everywhere, _command, _standin])
            if ((source['types']! as List).isEmpty || (source['types']! as List).contains(id)) source,
        ],
      },
  ],
};

const _file = '''
[store.overlays.catalogs]
boriken = false
team = "https://example.invalid/team.git@main#catalog.toml"

[store.skills.catalogs]
anthropics = false
mine = "owner/repo"

[store.mcps.catalogs]
corp = "https://registry.corp.test"

[store.extensions.catalogs]
plugins = "https://example.invalid/plugins.git@main#extensions.toml"
''';

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/store_catalogs_by_type_test.dart')));

  group('canoa.cfg: one shape for every type', () {
    final config = CanoaConfig.parse(_file);

    test('each type lists only the catalogs written under it', () {
      expect(config.catalogSpecs('overlays').keys, ['team']);
      expect(config.catalogSpecs('skills').keys, ['mine']);
      expect(config.catalogSpecs('mcps').keys, ['corp']);
      expect(config.catalogSpecs('extensions').keys, ['plugins']);
      expect(config.catalogSpecs('echoes'), isEmpty, reason: 'a type nobody wrote a catalog under has none');
      expect(config.catalogs.keys, ['team']);
      expect(config.skillSources.keys, ['mine']);
      expect(config.storeRegistries, ['https://registry.corp.test']);
      expect(config.extensionCatalogs.keys, ['plugins']);
    });

    test('a catalog added under one type is never listed under another, and what was there stays', () {
      final added = config.withCatalogOf('skills', 'more', CatalogSpec.read('owner/more'));
      expect(added.catalogSpecs('skills').keys, ['mine', 'more']);
      expect(added.catalogSpecs('overlays').keys, ['team']);
      expect(added.catalogSpecs('mcps').keys, ['corp']);
      expect(added.catalogSpecs('extensions').keys, ['plugins']);
      final again = CanoaConfig.parse(added.encode());
      expect(again.catalogSpecs('skills').keys, ['mine', 'more']);
      expect(again.catalogSpecs('overlays').keys, ['team']);

      final removed = added.withoutCatalogOf('skills', 'more');
      expect(removed.catalogSpecs('skills').keys, ['mine']);
      expect(removed.catalogSpecs('overlays').keys, ['team']);
    });

    test('a type an extension adds keeps its catalogs in the same shape, and the file keeps them', () {
      final echoes = config.withCatalogOf('echoes', 'loud', const CatalogSpec('standin', {'team': 'a', 'seats': 3}));
      expect(echoes.catalogSpecs('echoes')['loud'], const CatalogSpec('standin', {'team': 'a', 'seats': 3}));
      final read = CanoaConfig.parse(echoes.encode());
      expect(read.catalogSpecs('echoes')['loud']!.provider, 'standin');
      expect(read.catalogSpecs('echoes')['loud']!.fields, {'team': 'a', 'seats': 3});
      expect(read.catalogSpecs('overlays').keys, ['team']);
    });

    test('a string is the read provider, and a table names its provider and the fields of its form', () {
      final c = CanoaConfig.parse('''
[store.overlays.catalogs]
boriken = false
plain = "https://example.invalid/a.git@main#c.toml"
made = { provider = "command", command = ["gen", "--all"], timeout = 5 }
named = { provider = "read", source = "https://example.invalid/b.git@main#c.toml" }
''');
      final specs = c.catalogSpecs('overlays');
      expect(specs['plain']!.provider, readProvider);
      expect(specs['made']!.provider, 'command');
      expect(specs['made']!.fields, {'command': ['gen', '--all'], 'timeout': 5});
      expect(specs['named']!.source, 'https://example.invalid/b.git@main#c.toml');
      expect(c.catalogs.keys, ['plain', 'named'], reason: 'what a program makes is no git source to read');
      expect(c.overlayCatalogs.keys, ['plain', 'made', 'named'], reason: 'but it is a catalog of overlays');
      expect(CanoaConfig.parse(c.encode()).catalogSpecs('overlays')['made']!.fields['command'], ['gen', '--all']);
    });

    test('the old tables are refused outright, with no key decoded', () {
      // A top-level table nothing reads is kept as found, and no key of it is decoded into a catalog.
      final old = CanoaConfig.parse('[catalogs]\nteam = "https://example.invalid/a.git@main#c.toml"\n');
      expect(old.tables['catalogs'], {'team': 'https://example.invalid/a.git@main#c.toml'});
      expect(old.overlayCatalogs.keys, [defaultCatalogLabel]);
      for (final text in [
        '[store]\nregistries = ["https://x.test"]\n',
        '[store.skill_sources]\nweb = "o/r"\n',
        '[store.extension_catalogs]\nteam = "https://example.invalid/e.git@main#e.toml"\n',
      ]) {
        expect(() => CanoaConfig.parse(text), throwsFormatException, reason: text);
      }
    });

    test('a table that is no catalog says where', () {
      for (final text in [
        '[store.overlays.catalogs]\nteam = 3\n',
        '[store.overlays.catalogs]\nteam = { provider = 7 }\n',
        '[store.skills.catalogs]\n"two words" = "o/r"\n',
        '[store.echoes.catalogs]\nloud = true\n',
      ]) {
        expect(() => CanoaConfig.parse(text), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('store.'))), reason: text);
      }
    });
  });

  group('the settings of a type', () {
    late CanoaConfig stored;
    late LoadoutController controller;

    Future<void> open(WidgetTester tester, String type, {String text = _file, bool detail = false}) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      stored = CanoaConfig.parse(text);
      controller = LoadoutController(
        config: stored,
        home: null,
        save: (changed) async => stored = changed,
        catalogTypes: () async => catalogTypesFromAnswer(_answer(stored)),
      );
      addTearDown(controller.dispose);
      await controller.loadTypes();
      await tester.pumpWidget(MaterialApp(home: StoreSettingsPage(controller: controller, type: type, loadedDetail: detail)));
      await tester.pumpAndSettle();
    }

    Set<String> shown(WidgetTester tester) => {
      for (final element in find.byWidgetPredicate((widget) => widget.key is Key && '${widget.key}'.contains('store-catalog-')).evaluate())
        if (element.widget is ListTile) ((element.widget.key! as ValueKey).value as String).substring('store-catalog-'.length),
    };

    testWidgets("Overlays lists only the overlay catalogs, in that type's words", (tester) async {
      await open(tester, 'overlays');
      expect(shown(tester), {'team'});
      expect(find.text('Catalogs 1'), findsOneWidget);
      expect(find.text('Add a catalog'), findsOneWidget);
      expect(find.text('Add catalog'), findsOneWidget);
    });

    testWidgets('Skills lists only the skill sources, and calls them sources', (tester) async {
      await open(tester, 'skills');
      expect(shown(tester), {'mine'});
      expect(find.text('Sources 1'), findsOneWidget);
      expect(find.text('Add a source'), findsOneWidget);
    });

    testWidgets('MCP servers lists only its registries and Extensions only its catalogs', (tester) async {
      await open(tester, 'mcps');
      expect(shown(tester), {'official', 'corp'});
      expect(find.text('Registries 2'), findsOneWidget);
      await open(tester, 'extensions');
      expect(shown(tester), {'plugins'});
    });

    testWidgets('adding a source under Skills adds it to Skills only', (tester) async {
      await open(tester, 'skills');
      await tester.enterText(find.byKey(const Key('store-catalog-source')), 'addyosmani/web-quality-skills');
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      expect(stored.catalogSpecs('skills').values.map((spec) => spec.source), ['owner/repo', 'addyosmani/web-quality-skills']);
      expect(stored.catalogSpecs('overlays').keys, ['team']);
      expect(stored.catalogSpecs('mcps').keys, ['corp']);
      expect(stored.catalogSpecs('extensions').keys, ['plugins']);
      expect(shown(tester), hasLength(2));
    });

    testWidgets('Skills settings drawn: its sources, the form that adds one and the limits', (tester) async {
      await open(tester, 'skills');
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_type_settings_desktop.png'));
    });

    testWidgets('the limits on what a catalog is read as are on every type\'s page, written to [store], and one set back to its default is not written', (tester) async {
      for (final type in const ['overlays', 'skills', 'mcps', 'extensions']) {
        await open(tester, type);
        expect(find.byKey(const Key('limit-catalog_max_bytes')), findsOneWidget, reason: type);
      }
      await open(tester, 'skills');
      await tester.enterText(find.byKey(const Key('limit-catalog_max_depth')), '90');
      await tester.pumpAndSettle();
      expect(stored.store.catalogMaxDepth, 90);
      expect((stored.tables['store'] as Map)['catalog_max_depth'], 90);
      await tester.enterText(find.byKey(const Key('limit-catalog_max_depth')), '64');
      await tester.pumpAndSettle();
      expect((stored.tables['store'] as Map).containsKey('catalog_max_depth'), isFalse, reason: 'back to its default, so not written');
    });

    testWidgets('the official MCP registry is listed first and can be removed', (tester) async {
      await open(tester, 'mcps');
      expect(find.byKey(const Key('store-catalog-official')), findsOneWidget);
      await tester.tap(find.byKey(const Key('store-remove-catalog-official')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-yes')));
      await tester.pumpAndSettle();
      expect(stored.catalogTable('mcps'), {'official': false, 'corp': 'https://registry.corp.test'});
    });

    testWidgets('the sources a type is offered are those that serve it, and the others are not', (tester) async {
      await open(tester, 'overlays');
      Future<List<String>> offered() async {
        await tester.tap(find.byKey(const Key('store-source-provider')));
        await tester.pumpAndSettle();
        final titles = [for (final item in tester.widgetList<DropdownMenuItem<String>>(find.byType(DropdownMenuItem<String>))) item.value!];
        await tester.tap(find.text('Read an address or a file').last);
        await tester.pumpAndSettle();
        return titles.toSet().toList();
      }

      expect(await offered(), ['read', 'command']);
      await open(tester, 'skills');
      expect(await offered(), ['read', 'command', 'standin']);
      await open(tester, 'mcps');
      expect(await offered(), ['read', 'command'], reason: 'a program can generate any catalog; the stand-in serves Skills only');
    });

    testWidgets("a source's form is made of the fields it described, and what is filled in is kept under its type", (tester) async {
      await open(tester, 'skills');
      await tester.tap(find.byKey(const Key('store-source-provider')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('A stand-in (standin_ext)').last);
      await tester.pumpAndSettle();
      expect(find.text('Team name'), findsOneWidget);
      expect(find.text('Seats'), findsOneWidget);
      expect(find.text("Which team's list"), findsOneWidget);
      expect(find.byKey(const Key('store-catalog-source')), findsNothing, reason: 'the read form is gone');

      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      expect(find.text('Team name is needed'), findsOneWidget);
      expect(stored.catalogSpecs('skills').keys, ['mine']);

      await tester.enterText(find.byKey(const Key('store-standin-field-team')), 'platform');
      await tester.enterText(find.byKey(const Key('store-standin-field-seats')), 'many');
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      expect(find.text('Seats is a whole number'), findsOneWidget);

      await tester.enterText(find.byKey(const Key('store-standin-field-seats')), '12');
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      final added = stored.catalogSpecs('skills')['standin']!;
      expect(added.provider, 'standin');
      expect(added.fields, {'team': 'platform', 'seats': 12});
      expect(stored.catalogSpecs('overlays').keys, ['team']);
      expect(find.textContaining('Read by A stand-in'), findsOneWidget);
    });

    testWidgets('a yes or no and a choice are menus, a sensitive field is masked, and what is filled in is written to canoa.cfg as typed', (tester) async {
      await open(tester, 'skills');
      await tester.tap(find.byKey(const Key('store-source-provider')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('A stand-in (standin_ext)').last);
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(find.byKey(const Key('store-standin-field-team'))).decoration!.hintText, 'platform');
      expect(tester.widget<TextField>(find.byKey(const Key('store-standin-field-key'))).obscureText, isTrue);
      expect(tester.widget<TextField>(find.byKey(const Key('store-standin-field-team'))).obscureText, isFalse);
      await tester.enterText(find.byKey(const Key('store-standin-field-team')), 'platform');
      await tester.enterText(find.byKey(const Key('store-standin-field-key')), 'abc');
      await tester.tap(find.byKey(const Key('store-standin-field-layout')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('nested').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-standin-field-recent')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Yes').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      const fields = {'team': 'platform', 'layout': 'nested', 'recent': true, 'key': 'abc'};
      expect(stored.catalogSpecs('skills')['standin']!.fields, fields);
      expect(CanoaConfig.parse(stored.encode()).catalogSpecs('skills')['standin']!.fields, fields);
    });

    testWidgets('a program as the source of overlays: its words, one a line, and the seconds it may run', (tester) async {
      await open(tester, 'overlays');
      await tester.tap(find.byKey(const Key('store-source-provider')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Run a program').last);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('store-catalog-label')), 'made');
      await tester.enterText(find.byKey(const Key('store-command-field-command')), 'python\ngen.py\n--all');
      await tester.enterText(find.byKey(const Key('store-command-field-timeout')), '5');
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      final made = stored.catalogSpecs('overlays')['made']!;
      expect(made.provider, 'command');
      expect(made.fields, {'command': ['python', 'gen.py', '--all'], 'timeout': 5});
      expect(CanoaConfig.parse(stored.encode()).catalogSpecs('overlays')['made']!.fields['command'], ['python', 'gen.py', '--all']);
      expect(stored.catalogSpecs('skills').keys, ['mine']);
    });

    testWidgets('with no catalog the first-run form offers the sources the host lists, and a program makes the first catalog', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 1800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      stored = CanoaConfig.parse('[store.overlays.catalogs]\nboriken = false\n');
      controller = LoadoutController(
        config: stored,
        home: null,
        save: (changed) async => stored = changed,
        catalogTypes: () async => catalogTypesFromAnswer(_answer(stored)),
      );
      addTearDown(controller.dispose);
      await controller.loadTypes();
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: NoCatalogs(controller: controller)))));
      await tester.pumpAndSettle();
      expect(stored.catalogSpecs('overlays'), isEmpty);
      expect(find.byKey(const Key('store-source-provider')), findsOneWidget, reason: 'the host lists more than the read form');

      await tester.tap(find.byKey(const Key('store-source-provider')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Run a program').last);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('store-catalog-source')), findsNothing, reason: 'the read form is gone');
      await tester.enterText(find.byKey(const Key('store-command-field-command')), 'python gen.py');
      await tester.tap(find.byKey(const Key('store-add-catalog')));
      await tester.pumpAndSettle();
      final made = stored.catalogSpecs('overlays')['command']!;
      expect(made.provider, 'command');
      expect(made.fields['command'], ['python gen.py']);
    });

    testWidgets('removing one takes it from its type only, and a default is written as false', (tester) async {
      await open(tester, 'mcps');
      await tester.tap(find.byKey(const Key('store-remove-catalog-corp')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-no')));
      await tester.pumpAndSettle();
      expect(stored.catalogSpecs('mcps').keys, ['corp'], reason: 'keeping it removes nothing');
      await tester.tap(find.byKey(const Key('store-remove-catalog-corp')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-yes')));
      await tester.pumpAndSettle();
      expect(stored.catalogSpecs('mcps'), isEmpty);
      expect(stored.catalogSpecs('skills').keys, ['mine']);
      await open(tester, 'overlays', text: '[store.overlays.catalogs]\nteam = "https://example.invalid/team.git@main#catalog.toml"\n');
      expect(shown(tester), {'boriken', 'team'});
      await tester.tap(find.byKey(const Key('store-remove-catalog-boriken')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('confirm-remove-yes')));
      await tester.pumpAndSettle();
      expect(stored.catalogTable('overlays'), {'team': 'https://example.invalid/team.git@main#catalog.toml', 'boriken': false});
    });
  });
}
