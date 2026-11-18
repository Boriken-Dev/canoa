import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/forms/schema_fields.dart';
import 'package:canoa/profile/profile_editor.dart';
import 'package:canoa/profile/profile_items.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A guatiao schema of an object, as the host answers it.
Map<String, Object?> _object(Map<String, Object?> properties, {List<String> required = const []}) => {
  'type': 'object',
  'properties': properties,
  if (required.isNotEmpty) 'required': required,
};

/// What the host answers to `profile_items/list`: the items Canoa carries and, as a stand-in extension's, one that sets a region, a list
/// of hosts and a token that is a secret. Each item's values are a schema and a form, as a provider's own are.
Map<String, Object?> _listed({bool extension = true}) => {
  'items': [
    {
      'id': 'model',
      'title': 'Model',
      'origin': 'built-in',
      'stored': 'setting',
      'harnesses': <String>[],
      'schema': _object({
        'model': {'type': 'string', 'title': 'Model'},
      }, required: ['model']),
      'form': <String, Object?>{},
    },
    {
      'id': 'access',
      'title': 'Access',
      'help': 'What a command may reach.',
      'origin': 'built-in',
      'stored': 'setting',
      'harnesses': <String>[],
      'schema': _object({
        'sandbox': {
          'type': ['string', 'null'],
          'title': 'Sandbox',
          'enum': ['none', 'workspace', 'read-only'],
        },
        'network': {
          'type': ['boolean', 'null'],
          'title': 'Network',
        },
        'approvals': {
          'type': ['string', 'null'],
          'title': 'Approvals',
          'enum': ['ask', 'edits', 'never'],
        },
      }),
      'form': <String, Object?>{},
    },
    if (extension)
      {
        'id': 'region',
        'title': 'Region',
        'help': 'Where the session runs.',
        'origin': 'regions',
        'stored': 'item',
        'harnesses': ['claude-code'],
        'schema': _object({
          'name': {'type': 'string', 'title': 'Name', 'x-section': 'where'},
          'hosts': {
            'type': 'array',
            'items': {'type': 'string'},
            'title': 'Hosts',
            'x-section': 'where',
          },
          'tier': {'type': 'string', 'title': 'Tier', 'enum': ['standard', 'fast'], 'default': 'standard', 'x-section': 'where'},
          'token': {'type': ['string', 'null'], 'title': 'Token', 'writeOnly': true, 'x-sensitive': true, 'x-section': 'access'},
        }, required: ['name']),
        'form': {
          'sections': [
            {'id': 'where', 'title': 'Where'},
            {'id': 'access', 'title': 'Access'},
          ],
          'fields': {
            'name': {'placeholder': 'eu-west'},
          },
        },
      },
  ],
};

Future<List<(String, Preset)>> _pump(WidgetTester tester, ProfileItemCatalog catalog, Preset preset) async {
  await tester.binding.setSurfaceSize(const Size(900, 2400));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final saved = <(String, Preset)>[];
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: ProfileEditor(
            original: 'Deep',
            preset: preset,
            taken: const {'default', 'Deep'},
            sources: const ['Claude'],
            loadouts: const [],
            catalog: catalog,
            onSave: (name, made) async {
              saved.add((name, made));
              return null;
            },
            onCancel: () {},
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  return saved;
}

Future<void> _save(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('profile-edit-save')));
  await tester.pump();
}

void main() {
  test('the host\'s list of items is read as each provider describes its form', () {
    final catalog = ProfileItemCatalog()..adopt(_listed());
    expect([for (final item in catalog.items) item.id], ['model', 'access', 'region']);
    final region = catalog.byId('region')!;
    expect(region.origin, 'regions');
    expect(region.isSetting, isFalse);
    expect(region.harnesses, ['claude-code']);
    expect(schemaProperties(region.schema).keys, ['name', 'hosts', 'tier', 'token']);
    expect(schemaRequired(region.schema), {'name'});
    expect((region.form['sections']! as List).length, 2);
    expect(region.hasForm, isTrue);
    expect(catalog.byId('access')!.isSetting, isTrue);
  });

  testWidgets('an item an extension adds has its form in the editor, and what is entered is kept under the item', (tester) async {
    final saved = await _pump(tester, ProfileItemCatalog()..adopt(_listed()), const Preset(model: 'opus'));
    expect(find.byKey(const Key('item-section-region')), findsOneWidget);
    expect(find.text('Where the session runs.'), findsOneWidget);
    expect(find.text('For claude-code only'), findsOneWidget);
    expect(find.byKey(const Key('item-section-model')), findsNothing, reason: 'the model has a row of its own');
    expect(tester.widget<TextField>(find.byKey(const Key('item-region-field-name'))).decoration!.hintText, 'eu-west', reason: 'the form says its placeholder');

    await tester.enterText(find.byKey(const Key('item-region-field-name')), 'north');
    await tester.enterText(find.byKey(const Key('item-region-field-hosts')), 'a.example\n\n b.example ');
    await _save(tester);
    final made = saved.single.$2;
    expect(made.items, {
      'region': {'name': 'north', 'hosts': ['a.example', 'b.example']},
    });
    expect(made.model, 'opus', reason: 'the rest of the profile is as it was');
    expect(made.toTable()['items'], {
      'region': {'name': 'north', 'hosts': ['a.example', 'b.example']},
    });
  });

  testWidgets('a field the schema marks sensitive is masked and still reaches the profile as typed', (tester) async {
    final saved = await _pump(tester, ProfileItemCatalog()..adopt(_listed()), const Preset());
    final token = find.byKey(const Key('item-region-field-token'));
    expect(tester.widget<TextField>(token).obscureText, isTrue);
    expect(tester.widget<TextField>(find.byKey(const Key('item-region-field-name'))).obscureText, isFalse);
    await tester.enterText(find.byKey(const Key('item-region-field-name')), 'north');
    await tester.enterText(token, 's3cret');
    await _save(tester);
    expect(saved.single.$2.items, {
      'region': {'name': 'north', 'token': 's3cret'},
    });
    final written = CanoaConfig.parse('').withPresets({'Deep': saved.single.$2}).encode();
    expect(CanoaConfig.parse(written).presets['Deep']!.items['region'], {'name': 'north', 'token': 's3cret'}, reason: 'it round-trips through canoa.cfg');
  });

  testWidgets('an item emptied is no longer set', (tester) async {
    final saved = await _pump(
      tester,
      ProfileItemCatalog()..adopt(_listed()),
      const Preset(items: {'region': {'name': 'north'}}),
    );
    await tester.enterText(find.byKey(const Key('item-region-field-name')), '');
    await _save(tester);
    expect(saved.single.$2.items, isEmpty);
  });

  testWidgets('the access is a form the host describes, and it is the profile\'s own access setting', (tester) async {
    final saved = await _pump(tester, ProfileItemCatalog()..adopt(_listed()), const Preset());
    expect(find.byKey(const Key('item-section-access')), findsOneWidget);
    await tester.tap(find.byKey(const Key('item-access-field-sandbox')));
    await tester.pump();
    await tester.tap(find.text('read-only').last);
    await tester.pump();
    await tester.tap(find.byKey(const Key('item-access-field-network')));
    await tester.pump();
    await tester.tap(find.text('No').last);
    await tester.pump();
    await _save(tester);
    final made = saved.single.$2;
    expect(made.access.sandbox, 'read-only');
    expect(made.access.network, false);
    expect(made.access.approvals, isNull);
    expect(made.items, isEmpty, reason: 'the access is kept as the profile\'s own setting, not as an item');
  });

  testWidgets('an extension taken away takes its item from the editor, and the profile keeps the values and says they are not applied', (tester) async {
    final catalog = ProfileItemCatalog()..adopt(_listed());
    final saved = await _pump(tester, catalog, const Preset(items: {'region': {'name': 'north'}}));
    expect(find.byKey(const Key('item-section-region')), findsOneWidget);
    expect(find.byKey(const Key('item-gone-region')), findsNothing);

    catalog.adopt(_listed(extension: false));
    await tester.pump();
    expect(find.byKey(const Key('item-section-region')), findsNothing);
    expect(find.byKey(const Key('item-gone-region')), findsOneWidget);
    expect(find.textContaining('kept and not applied'), findsOneWidget);
    await _save(tester);
    expect(saved.single.$2.items, {'region': {'name': 'north'}}, reason: 'saving does not drop what the profile holds');

    await tester.tap(find.byKey(const Key('item-gone-remove-region')));
    await tester.pump();
    expect(find.byKey(const Key('item-gone-region')), findsNothing);
  });

  test('the items are asked for again when the extensions the host reports change, and only then', () async {
    final catalog = ProfileItemCatalog();
    var asked = 0;
    var extension = true;
    final refresh = ProfileItemsRefresh(() async {
      asked++;
      catalog.adopt(_listed(extension: extension));
    });
    refresh.extensionsAre('regions:Region');
    refresh.extensionsAre('regions:Region');
    await Future<void>.delayed(Duration.zero);
    expect(asked, 0, reason: 'the first answer only says what the items were made under, and an unchanged one asks nothing');
    extension = false;
    refresh.extensionsAre('');
    await Future<void>.delayed(Duration.zero);
    expect(asked, 1);
    expect(catalog.byId('region'), isNull, reason: 'the item of the extension that went is gone');
    extension = true;
    refresh.extensionsAre('regions:Region');
    await Future<void>.delayed(Duration.zero);
    expect(asked, 2);
    expect(catalog.byId('region'), isNotNull, reason: 'and one an extension added shows');
  });

  test('a profile with items is read from canoa.cfg and written back as it was', () {
    final config = CanoaConfig.parse(
      '[profiles.Deep]\nmodel = "opus"\n[profiles.Deep.items.region]\nname = "north"\nhosts = ["a.example"]\n',
    );
    final deep = config.presets['Deep']!;
    expect(deep.items, {'region': {'name': 'north', 'hosts': ['a.example']}});
    final written = config.withPresets(config.presets);
    expect(written.presets['Deep'], deep);
    expect(config.presets.keys.first, presetDefault);
    expect(() => CanoaConfig.parse('[profiles.Deep.items.Region]\nname = "x"\n').presets, throwsFormatException);
    expect(() => CanoaConfig.parse('[profiles.Deep]\nitems = "x"\n').presets, throwsFormatException);
  });
}
