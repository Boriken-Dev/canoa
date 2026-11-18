import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/profile_editor.dart';
import 'package:canoa/profile/profile_items.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// What the host answers to `profile_items/list`: the items Canoa carries and, as a stand-in extension's, one that sets a region and
/// a list of hosts.
Map<String, Object?> _listed({bool extension = true}) => {
  'items': [
    {
      'id': 'model',
      'title': 'Model',
      'origin': 'built-in',
      'stored': 'setting',
      'harnesses': <String>[],
      'fields': [
        {'name': 'model', 'label': 'Model', 'shape': 'text', 'required': true, 'choices': <String>[]},
      ],
    },
    {
      'id': 'access',
      'title': 'Access',
      'help': 'What a command may reach.',
      'origin': 'built-in',
      'stored': 'setting',
      'harnesses': <String>[],
      'fields': [
        {'name': 'sandbox', 'label': 'Sandbox', 'shape': 'choice', 'required': false, 'choices': ['none', 'workspace', 'read-only']},
        {'name': 'network', 'label': 'Network', 'shape': 'bool', 'required': false, 'choices': <String>[]},
        {'name': 'approvals', 'label': 'Approvals', 'shape': 'choice', 'required': false, 'choices': ['ask', 'edits', 'never']},
      ],
    },
    if (extension)
      {
        'id': 'region',
        'title': 'Region',
        'help': 'Where the session runs.',
        'origin': 'regions',
        'stored': 'item',
        'harnesses': ['claude-code'],
        'fields': [
          {'name': 'name', 'label': 'Name', 'shape': 'text', 'required': true, 'choices': <String>[]},
          {'name': 'hosts', 'label': 'Hosts', 'shape': 'list', 'required': false, 'choices': <String>[]},
          {'name': 'tier', 'label': 'Tier', 'shape': 'choice', 'required': false, 'default': 'standard', 'choices': ['standard', 'fast']},
        ],
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
    expect([for (final field in region.fields) '${field.name}:${field.shape}'], ['name:text', 'hosts:list', 'tier:choice']);
    expect(region.fields.last.defaultValue, 'standard');
    expect(region.fields.last.choices, ['standard', 'fast']);
    expect(catalog.byId('access')!.isSetting, isTrue);
  });

  testWidgets('an item an extension adds has its form in the editor, and what is entered is kept under the item', (tester) async {
    final saved = await _pump(tester, ProfileItemCatalog()..adopt(_listed()), const Preset(model: 'opus'));
    expect(find.byKey(const Key('item-section-region')), findsOneWidget);
    expect(find.text('Where the session runs.'), findsOneWidget);
    expect(find.text('For claude-code only'), findsOneWidget);
    expect(find.byKey(const Key('item-section-model')), findsNothing, reason: 'the model has a row of its own');

    await tester.enterText(find.byKey(const Key('item-field-region-name')), 'north');
    await tester.enterText(find.byKey(const Key('item-field-region-hosts')), 'a.example\n\n b.example ');
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

  testWidgets('an item emptied is no longer set', (tester) async {
    final saved = await _pump(
      tester,
      ProfileItemCatalog()..adopt(_listed()),
      const Preset(items: {'region': {'name': 'north'}}),
    );
    await tester.enterText(find.byKey(const Key('item-field-region-name')), '');
    await _save(tester);
    expect(saved.single.$2.items, isEmpty);
  });

  testWidgets('the access is a form the host describes, and it is the profile\'s own access setting', (tester) async {
    final saved = await _pump(tester, ProfileItemCatalog()..adopt(_listed()), const Preset());
    expect(find.byKey(const Key('item-section-access')), findsOneWidget);
    await tester.tap(find.byKey(const Key('item-field-access-sandbox')));
    await tester.pump();
    await tester.tap(find.text('read-only').last);
    await tester.pump();
    await tester.tap(find.byKey(const Key('item-field-access-network')));
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
