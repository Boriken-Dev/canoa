// The form renderer for values a provider describes with a guatiao schema and a guatiao-intake form: it lays members out and shows
// them as guatiao-intake does (the vectors in test/fixtures/form_vectors.json, which crates/canoa/tests/form_vectors.rs holds
// guatiao-intake to), reads a number as JSON Schema does, keeps an enum of any type, and refuses to save what the schema refuses.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/forms/schema_fields.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> _object(Object? value) => (value! as Map).cast<String, Object?>();

void main() {
  final vectors = _object(jsonDecode(File('test/fixtures/form_vectors.json').readAsStringSync()));
  final cases = [for (final each in vectors['cases']! as List) _object(each)];

  test('the vectors were read', () => expect(cases.length, greaterThanOrEqualTo(8)));

  for (final each in cases) {
    final schema = _object(each['schema']), form = _object(each['form']);
    test('layout: ${each['name']}', () {
      final groups = [
        for (final group in schemaLayout(schema, form))
          {'section': group.section, if (group.title.isNotEmpty) 'title': group.title, 'fields': group.keys},
      ];
      expect(groups, each['layout']);
    });
    test('shown: ${each['name']}', () {
      for (final shown in each['shown']! as List) {
        final values = _object(_object(shown)['values']);
        expect(
          [for (final key in schemaProperties(schema).keys) if (schemaVisible(schema, form, values, key)) key],
          _object(shown)['members'],
          reason: 'for $values',
        );
      }
    });
  }

  group('numbers', () {
    final schema = {
      'type': 'object',
      'properties': {
        'count': {'type': 'integer', 'title': 'Count'},
        'offset': {'type': 'integer', 'title': 'Offset', 'minimum': -10},
        'ratio': {'type': 'number', 'title': 'Ratio'},
        'port': {'type': 'integer', 'title': 'Port', 'minimum': 1, 'maximum': 65535},
      },
    };
    String? problem(Map<String, Object?> values) => schemaProblem(schema, const {}, values);

    test('a number is any number, and an integer a whole one, negative unless the schema bounds it', () {
      expect(problem({'ratio': 2.5}), isNull);
      expect(problem({'ratio': -0.5}), isNull);
      expect(problem({'ratio': 'x'}), 'Ratio is a number');
      expect(problem({'count': -3}), isNull, reason: 'no minimum, so a negative whole number is one');
      expect(problem({'count': 2.5}), 'Count is a whole number');
      expect(problem({'count': 4.0}), isNull, reason: 'JSON Schema counts 4.0 an integer');
      expect(problem({'count': 'x'}), 'Count is a whole number');
      expect(problem({'offset': -10}), isNull);
      expect(problem({'offset': -11}), 'Offset is at least -10');
      expect(problem({'port': 0}), 'Port is at least 1');
      expect(problem({'port': 70000}), 'Port is at most 65535');
    });

    testWidgets('typed text is read as a real number, a negative number and a whole number, and what is not one is a problem', (tester) async {
      Map<String, Object?> held = {};
      await tester.pumpWidget(MaterialApp(
        home: Material(
          child: SingleChildScrollView(child: SchemaFields(schema: schema, form: const {}, values: const {}, idPrefix: 'n', onValues: (values) => held = values)),
        ),
      ));
      await tester.enterText(find.byKey(const Key('n-field-ratio')), '2.5');
      await tester.enterText(find.byKey(const Key('n-field-count')), '-3');
      expect(held, {'ratio': 2.5, 'count': -3});
      expect(problem(held), isNull);
      await tester.enterText(find.byKey(const Key('n-field-count')), '2.5');
      expect(problem(held), 'Count is a whole number');
    });
  });

  group('the layout is drawn', () {
    final schema = {
      'type': 'object',
      'properties': {
        'host': {'type': 'string', 'title': 'Host', 'x-section': 'net'},
        'name': {'type': 'string', 'title': 'Name'},
        'port': {'type': 'integer', 'title': 'Port', 'x-section': 'net', 'x-order': 1},
        'proxy': {'type': 'string', 'title': 'Proxy', 'x-section': 'net', 'x-order': 0},
      },
    };
    final form = {
      'sections': [
        {'id': 'net', 'title': 'Network', 'description': 'Where it connects.'},
      ],
    };

    testWidgets('the default section first, a declared section under its title, and members in their order', (tester) async {
      await tester.pumpWidget(MaterialApp(
        home: Material(child: SingleChildScrollView(child: SchemaFields(schema: schema, form: form, values: const {}, idPrefix: 'l', onValues: (_) {}))),
      ));
      double top(String key) => tester.getTopLeft(find.byKey(Key('l-field-$key'))).dy;
      expect(find.text('Network'), findsOneWidget);
      expect(find.text('Where it connects.'), findsOneWidget);
      final heading = tester.getTopLeft(find.byKey(const Key('l-section-net'))).dy;
      expect(top('name'), lessThan(heading), reason: 'the default section comes first');
      expect(heading, lessThan(top('proxy')));
      expect(top('proxy'), lessThan(top('port')), reason: 'x-order 0 before 1');
      expect(top('port'), lessThan(top('host')), reason: 'an explicit order before the rest');
      expect(find.byKey(const Key('l-section-')), findsNothing, reason: 'the default section the form did not declare has no title');
    });
  });

  group('SchemaSettingsForm saves what the schema accepts', () {
    final schema = {
      'type': 'object',
      'required': ['url'],
      'properties': {
        'url': {'type': 'string', 'title': 'Address'},
        'retries': {'type': 'integer', 'title': 'Retries', 'minimum': 0},
        'level': {'type': 'integer', 'title': 'Level', 'enum': [1, 2, 3], 'default': 2},
      },
    };

    Future<List<Map<String, Object?>>> pump(WidgetTester tester) async {
      final saved = <Map<String, Object?>>[];
      await tester.pumpWidget(MaterialApp(
        home: Material(
          child: SingleChildScrollView(
            child: SchemaSettingsForm(
              schema: schema,
              form: const {},
              name: 'x',
              keyPrefix: 's',
              onSave: (settings, cleared, from) async {
                saved.add(settings);
                return null;
              },
            ),
          ),
        ),
      ));
      return saved;
    }

    testWidgets('a required member nobody filled, and a number the schema bounds, stop the save and say why', (tester) async {
      final saved = await pump(tester);
      await tester.tap(find.byKey(const Key('s-save')));
      await tester.pump();
      expect(saved, isEmpty);
      expect(find.byKey(const Key('s-save-error')), findsOneWidget);
      expect(find.text('Address is needed'), findsOneWidget);

      await tester.enterText(find.byKey(const Key('s-field-url')), 'https://x.example');
      await tester.enterText(find.byKey(const Key('s-field-retries')), '-1');
      await tester.tap(find.byKey(const Key('s-save')));
      await tester.pump();
      expect(saved, isEmpty);
      expect(find.text('Retries is at least 0'), findsOneWidget);

      await tester.enterText(find.byKey(const Key('s-field-retries')), '3');
      await tester.tap(find.byKey(const Key('s-save')));
      await tester.pump();
      expect(saved, [
        {'url': 'https://x.example', 'retries': 3},
      ]);
      expect(find.byKey(const Key('s-save-error')), findsNothing);
    });

    testWidgets('an enum of numbers is offered, and the one picked is kept as the number', (tester) async {
      final saved = await pump(tester);
      expect(find.text('2'), findsOneWidget, reason: 'the default is the one shown chosen');
      await tester.tap(find.byKey(const Key('s-field-level')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('3').last);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('s-field-url')), 'https://x.example');
      await tester.tap(find.byKey(const Key('s-save')));
      await tester.pump();
      expect(saved, [
        {'url': 'https://x.example', 'level': 3},
      ]);
    });
  });
}
