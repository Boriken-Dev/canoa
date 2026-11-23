// A2UI 1.0 views: the messages an agent may send and what is refused, the bounds against an agent that sends too much, the widgets
// drawn in the colours of the look, and a button's action going back as the protocol's message.
import 'dart:convert';

import 'package:canoa/surfaces/a2ui/catalog.dart';
import 'package:canoa/surfaces/a2ui/limits.dart';
import 'package:canoa/surfaces/a2ui/messages.dart';
import 'package:canoa/surfaces/a2ui/model.dart';
import 'package:canoa/surfaces/a2ui/view.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> msg(String key, Map<String, Object?> body) => {'version': '1.0', key: body};

Map<String, Object?> create(String id, {String? catalog, List<Object?>? components, Object? data}) =>
    msg('createSurface', {'surfaceId': id, 'catalogId': ?catalog, 'components': ?components, 'dataModel': ?data});

Map<String, Object?> components(String id, List<Object?> list) => msg('updateComponents', {'surfaceId': id, 'components': list});

Map<String, Object?> data(String id, String path, Object? value) => msg('updateDataModel', {'surfaceId': id, 'path': path, 'value': value});

Widget app(Widget child, {LookId look = LookId.canoa, Brightness brightness = Brightness.dark}) => MaterialApp(
  theme: themeFor(look, brightness),
  home: Scaffold(body: SizedBox(width: 600, height: 700, child: child)),
);

A2uiProcessor processor({A2uiLimits limits = const A2uiLimits(), DateTime Function()? clock, A2uiCatalogs? catalogs}) =>
    A2uiProcessor(limits: limits, clock: clock, catalogs: catalogs);

void main() {
  group('messages', () {
    const limits = A2uiLimits();

    test('a message has the version and exactly one key', () {
      expect(readA2uiMessage(create('s'), limits).message, isA<CreateSurface>());
      expect(readA2uiMessage({'createSurface': {'surfaceId': 's'}}, limits).problem?.message, contains('version'));
      expect(readA2uiMessage({'version': '0.9', 'createSurface': {'surfaceId': 's'}}, limits).problem, isNotNull);
      final two = {'version': '1.0', 'createSurface': {'surfaceId': 's'}, 'deleteSurface': {'surfaceId': 's'}};
      expect(readA2uiMessage(two, limits).problem?.message, contains('exactly one'));
      expect(readA2uiMessage({'version': '1.0', 'surprise': {'surfaceId': 's'}}, limits).problem?.message, contains('not a message'));
    });

    test('a surface carries no theme: the app draws it', () {
      final read = readA2uiMessage(msg('createSurface', {'surfaceId': 's', 'theme': {'primaryColor': '#ff0000'}}), limits);
      expect(read.problem?.code, 'VALIDATION_FAILED');
      expect(read.problem?.message, contains('theme'));
    });

    test('a function call is answered with an error, never run', () {
      for (final key in ['callRendererFunction', 'callAgentFunction']) {
        final read = readA2uiMessage(msg(key, {'surfaceId': 's', 'call': 'openUrl'}), limits);
        expect(read.problem?.message, contains('not run'), reason: key);
        expect(read.problem?.toMessage(), {
          'version': '1.0',
          'error': {'code': 'VALIDATION_FAILED', 'surfaceId': 's', 'path': '', 'message': read.problem!.message},
        });
      }
    });

    test('one message holds at most the limit', () {
      final big = create('s', components: [for (var i = 0; i < 200; i++) {'id': 'c$i', 'component': 'Text', 'text': 'x' * 100}]);
      expect(readA2uiMessage(big, const A2uiLimits(maxMessageBytes: 1000)).problem?.message, contains('1000'));
    });

    test('the carrier takes a message, a list, or an object with a target and a title', () {
      final one = create('s');
      expect(a2uiCarried({'a2ui': one})!.messages, [one]);
      expect(a2uiCarried({'a2ui': [one, one]})!.messages, hasLength(2));
      final placed = a2uiCarried({'a2ui': {'messages': [one], 'target': 'tab', 'title': 'Report'}})!;
      expect((placed.target, placed.title), ('tab', 'Report'));
      expect(a2uiCarried({'other': 1}), isNull);
      expect(a2uiCarried(null), isNull);
    });

    test('JSON pointers read and write with the escapes, and removal is a null', () {
      final model = {'a': {'b/c': [1, 2]}, 'x~y': 'z'};
      expect(pointerRead(model, '/a/b~1c/1'), (true, 2));
      expect(pointerRead(model, '/x~0y'), (true, 'z'));
      expect(pointerRead(model, '/a/none').$1, isFalse);
      expect(pointerWrite(model, '/a/new', 3), {'a': {'b/c': [1, 2], 'new': 3}, 'x~y': 'z'});
      expect(pointerWrite(model, '/a/b~1c/2', 9), {'a': {'b/c': [1, 2, 9]}, 'x~y': 'z'});
      expect(pointerWrite(model, '/x~0y', null, removed: true), {'a': {'b/c': [1, 2]}});
      expect(pointerWrite(model, '/a/b~1c/7', 1), isNull, reason: 'past the end of a list');
      expect(pointerWrite(model, 'no-slash', 1), isNull);
    });
  });

  group('the processor', () {
    test('components that arrive before root are held, and the surface draws once root is there', () {
      final p = processor();
      p.receive([
        create('s'),
        components('s', [{'id': 'a', 'component': 'Text', 'text': 'hi'}]),
      ]);
      expect(p.surfaces['s']!.hasRoot, isFalse);
      p.receive([components('s', [{'id': 'root', 'component': 'Column', 'children': ['a']}])]);
      expect(p.surfaces['s']!.hasRoot, isTrue);
      expect(p.problems, isEmpty);
    });

    test('a message for a surface that was never created, or an unknown catalog, is a problem and changes nothing', () {
      final p = processor();
      p.receive([components('ghost', []), create('s', catalog: 'https://elsewhere.example/catalog.json')]);
      expect(p.surfaces, isEmpty);
      expect(p.problems.map((each) => each.message), [contains('createSurface comes first'), contains('No catalog')]);
    });

    test('updateDataModel sets a path, replaces the model at /, and removes with a null', () {
      final p = processor()..receive([create('s', data: {'a': 1}), data('s', '/b/c', 'x'), data('s', '/a', null)]);
      expect(p.surfaces['s']!.data, {'b': {'c': 'x'}});
      p.receive([data('s', '/', {'z': true})]);
      expect(p.surfaces['s']!.data, {'z': true});
    });

    test('deleteSurface removes it', () {
      final p = processor()..receive([create('s'), msg('deleteSurface', {'surfaceId': 's'})]);
      expect(p.surfaces, isEmpty);
    });

    test('components past the limit are dropped, said once, and never grow the surface', () {
      final p = processor(limits: const A2uiLimits(maxNodes: 5))
        ..receive([
          create('s', components: [for (var i = 0; i < 50; i++) {'id': i == 0 ? 'root' : 'c$i', 'component': 'Text', 'text': 'x'}]),
        ]);
      expect(p.surfaces['s']!.components, hasLength(5));
      expect(p.surfaces['s']!.cuts, hasLength(1));
      expect(p.surfaces['s']!.cuts.single, contains('more than 5'));
    });

    test('a data model past its size is refused and keeps what it had', () {
      final p = processor(limits: const A2uiLimits(maxDataBytes: 100))
        ..receive([create('s', data: {'a': 1}), data('s', '/big', 'x' * 500)]);
      expect(p.surfaces['s']!.data, {'a': 1});
      expect(p.problems.single.message, contains('100 bytes'));
    });

    test('a session keeps at most the limit of surfaces', () {
      final p = processor(limits: const A2uiLimits(maxSurfaces: 2))..receive([create('a'), create('b'), create('c')]);
      expect(p.surfaces.keys, ['a', 'b']);
      expect(p.problems.single.message, contains('at most 2'));
    });

    test('messages past the rate in one second are dropped, and the next second starts again', () {
      var now = DateTime(2026, 1, 1);
      final p = processor(limits: const A2uiLimits(maxUpdatesPerSecond: 3), clock: () => now)..receive([for (var i = 0; i < 10; i++) create('s$i')]);
      expect(p.surfaces.keys, ['s0', 's1', 's2']);
      expect(p.problems, hasLength(1), reason: 'said once for the second');
      now = now.add(const Duration(seconds: 2));
      p.receive([create('later')]);
      expect(p.surfaces.containsKey('later'), isTrue);
    });

    test('the data from createSurface and a refused sendDataModel', () {
      final asked = msg('createSurface', {'surfaceId': 's', 'sendDataModel': true});
      expect((processor()..receive([asked])).surfaces['s']!.sendDataModel, isFalse, reason: 'off unless the app allows it');
      expect((A2uiProcessor(allowSendDataModel: true)..receive([asked])).surfaces['s']!.sendDataModel, isTrue);
    });
  });

  group('the catalogs', () {
    test('an overlay catalog only renames and presets components Canoa draws, under its own id', () {
      final ok = A2uiCatalog.fromOverlay({
        'catalogId': 'urn:overlay:x',
        'components': {
          'Stat': {'extends': 'Metric', 'defaults': {'label': 'Open'}, 'allowedParents': ['Column']},
        },
      });
      expect(ok.problems, isEmpty);
      expect(ok.catalog!['Stat']!.native, 'Metric');
      expect(ok.catalog!['Stat']!.required, ['value'], reason: 'the label is preset');
      expect(ok.catalog!['Text'], isNotNull, reason: 'Canoa\'s components stay');

      final bad = A2uiCatalog.fromOverlay({
        'catalogId': canoaCatalogId,
        'components': {'Wasm': {'extends': 'NativeCode'}, 'Text': {'extends': 'Text'}},
      });
      expect(bad.catalog, isNull);
      expect(bad.problems, hasLength(3));
    });

    test('a component an overlay allows only in some parents says UNALLOWED_PARENT', () {
      final catalog = A2uiCatalog.fromOverlay({
        'catalogId': 'urn:overlay:x',
        'components': {'Stat': {'extends': 'Metric', 'allowedParents': ['Card']}},
      }).catalog!;
      final p = processor(catalogs: A2uiCatalogs([catalog]))
        ..receive([
          create('s', catalog: 'urn:overlay:x', components: [
            {'id': 'root', 'component': 'Column', 'children': ['m']},
            {'id': 'm', 'component': 'Stat', 'label': 'a', 'value': 'b'},
          ]),
        ]);
      expect(p.problems.single.code, 'UNALLOWED_PARENT');
    });

    test('a component without a property it cannot be drawn without is a problem', () {
      final p = processor()..receive([create('s', components: [{'id': 'root', 'component': 'Button', 'child': 'x'}])]);
      expect(p.problems.single.message, 'Button needs "action"');
    });
  });

  group('drawn', () {
    testWidgets('the basic components draw, and a binding follows the data model', (tester) async {
      final p = processor()
        ..receive([
          create('s', data: {'name': 'Ada', 'items': [{'t': 'one'}, {'t': 'two'}]}, components: [
            {'id': 'root', 'component': 'Column', 'children': ['title', 'list', 'card']},
            {'id': 'title', 'component': 'Text', 'text': {'@call': 'formatString', 'args': {'value': r'Hello ${/name}'}}},
            {'id': 'list', 'component': 'List', 'children': {'path': '/items', 'componentId': 'row'}},
            {'id': 'row', 'component': 'Text', 'text': {'@path': 't'}},
            {'id': 'card', 'component': 'Card', 'child': 'tabs'},
            {'id': 'tabs', 'component': 'Tabs', 'tabs': [{'title': 'A', 'child': 'ta'}, {'title': 'B', 'child': 'tb'}]},
            {'id': 'ta', 'component': 'Text', 'text': 'in a'},
            {'id': 'tb', 'component': 'Text', 'text': 'in b'},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.text('Hello Ada'), findsOneWidget);
      expect(find.text('one'), findsOneWidget);
      expect(find.text('two'), findsOneWidget);
      expect(find.text('in a'), findsOneWidget);
      expect(find.text('in b'), findsNothing, reason: 'only the chosen tab is drawn');
      await tester.tap(find.byKey(const ValueKey('a2ui tab 1')));
      await tester.pump();
      expect(find.text('in b'), findsOneWidget);
      p.receive([data('s', '/name', 'Grace')]);
      await tester.pump();
      expect(find.text('Hello Grace'), findsOneWidget);
    });

    testWidgets('a surface with no root says it is waiting', (tester) async {
      final p = processor()..receive([create('s')]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.byKey(const ValueKey('a2ui waiting')), findsOneWidget);
    });

    testWidgets('an unknown component is a readable notice in its place, and the rest draws', (tester) async {
      final p = processor()
        ..receive([
          create('s', components: [
            {'id': 'root', 'component': 'Column', 'children': ['a', 'b']},
            {'id': 'a', 'component': 'Hologram'},
            {'id': 'b', 'component': 'Text', 'text': 'still here'},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.textContaining('"Hologram" is not a component of this catalog'), findsWidgets);
      expect(find.byKey(const ValueKey('a2ui problems')), findsOneWidget);
      expect(find.text('still here'), findsOneWidget);
    });

    testWidgets('a tree deeper than the limit is cut with a notice, and a cycle does not loop', (tester) async {
      final chain = [
        for (var i = 0; i < 30; i++) {'id': i == 0 ? 'root' : 'n$i', 'component': 'Column', 'children': [if (i < 29) 'n${i + 1}']},
        {'id': 'n29', 'component': 'Column', 'children': ['root']},
      ];
      final p = processor(limits: const A2uiLimits(maxDepth: 4))..receive([create('s', components: chain)]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.textContaining('Deeper than 4 levels'), findsOneWidget);

      final loop = processor()
        ..receive([
          create('l', components: [
            {'id': 'root', 'component': 'Column', 'children': ['x']},
            {'id': 'x', 'component': 'Column', 'children': ['root']},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: loop.surfaces['l']!)));
      expect(find.textContaining('holds itself'), findsOneWidget);
    });

    testWidgets('a list template draws at most the row limit and says how many it left', (tester) async {
      final p = processor(limits: const A2uiLimits(maxRows: 3))
        ..receive([
          create('s', data: {'rows': [for (var i = 0; i < 50; i++) {'t': 'r$i'}]}, components: [
            {'id': 'root', 'component': 'List', 'children': {'path': '/rows', 'componentId': 'row'}},
            {'id': 'row', 'component': 'Text', 'text': {'@path': 't'}},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.text('r2'), findsOneWidget);
      expect(find.text('r3'), findsNothing);
      expect(find.text('47 more rows are not shown.'), findsOneWidget);
    });

    testWidgets('more components than the limit show the cut notice above what is drawn', (tester) async {
      final p = processor(limits: const A2uiLimits(maxNodes: 4))
        ..receive([
          create('s', components: [
            {'id': 'root', 'component': 'Column', 'children': [for (var i = 0; i < 10; i++) 'c$i']},
            for (var i = 0; i < 10; i++) {'id': 'c$i', 'component': 'Text', 'text': 'item $i'},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.byKey(const ValueKey('a2ui cut notice')), findsOneWidget);
      expect(find.text('item 9'), findsNothing);
    });

    testWidgets('an image is drawn only when its bytes are in the message: no address is fetched', (tester) async {
      final p = processor()
        ..receive([
          create('s', components: [
            {'id': 'root', 'component': 'Column', 'children': ['i']},
            {'id': 'i', 'component': 'Image', 'url': 'https://tracker.example/pixel.png', 'description': 'a chart'},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.byKey(const ValueKey('a2ui image placeholder')), findsOneWidget);
      expect(find.byType(Image), findsNothing);
    });

    testWidgets('typing in a field writes the data model, and a checkbox writes a boolean', (tester) async {
      final p = processor()
        ..receive([
          create('s', data: {'q': '', 'on': false}, components: [
            {'id': 'root', 'component': 'Column', 'children': ['f', 'c']},
            {'id': 'f', 'component': 'TextField', 'label': 'Query', 'value': {'@path': '/q'}},
            {'id': 'c', 'component': 'CheckBox', 'label': 'On', 'value': {'@path': '/on'}},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      await tester.enterText(find.byType(TextField), 'abc');
      await tester.tap(find.byType(Checkbox));
      await tester.pump();
      expect(p.surfaces['s']!.data, {'q': 'abc', 'on': true});
    });

    testWidgets('a button\'s event is the protocol\'s action, with its context read from the data model', (tester) async {
      final p = processor()
        ..receive([
          create('s', data: {'who': 'Ada'}, components: [
            {'id': 'root', 'component': 'Column', 'children': ['b']},
            {'id': 'label', 'component': 'Text', 'text': 'Go'},
            {'id': 'b', 'component': 'Button', 'child': 'label', 'variant': 'primary', 'action': {'event': {'name': 'submit', 'context': {'who': {'@path': '/who'}, 'n': 3}}}},
          ]),
        ]);
      final sent = <Map<String, Object?>>[];
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!, onAction: sent.add)));
      await tester.tap(find.byKey(const ValueKey('a2ui button b')));
      final action = sent.single['action']! as Map;
      expect(sent.single['version'], '1.0');
      expect((action['name'], action['surfaceId'], action['sourceComponentId']), ('submit', 's', 'b'));
      expect(action['context'], {'who': 'Ada', 'n': 3});
      expect(DateTime.parse(action['timestamp'] as String).isUtc, isTrue);
      expect(action.containsKey('metadata'), isFalse, reason: 'the data model stays here unless it was asked for and allowed');
      expect(() => jsonEncode(sent.single), returnsNormally);
    });

    testWidgets('a button with no agent behind it is switched off, and a function call other than openUrl does nothing', (tester) async {
      final p = processor()
        ..receive([
          create('s', components: [
            {'id': 'root', 'component': 'Column', 'children': ['b', 'x']},
            {'id': 'label', 'component': 'Text', 'text': 'Go'},
            {'id': 'b', 'component': 'Button', 'child': 'label', 'action': {'event': {'name': 'go'}}},
            {'id': 'x', 'component': 'Button', 'child': 'label', 'action': {'functionCall': {'call': 'deleteEverything', 'args': {}}}},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!, onOpenUrl: (_) async {})));
      expect(tester.widget<OutlinedButton>(find.byKey(const ValueKey('a2ui button b'))).onPressed, isNull);
      expect(tester.widget<OutlinedButton>(find.byKey(const ValueKey('a2ui button x'))).onPressed, isNull);
    });

    testWidgets('openUrl opens only a web address', (tester) async {
      final p = processor()
        ..receive([
          create('s', components: [
            {'id': 'root', 'component': 'Column', 'children': ['good', 'bad']},
            {'id': 'label', 'component': 'Text', 'text': 'Go'},
            {'id': 'good', 'component': 'Button', 'child': 'label', 'action': {'functionCall': {'call': 'openUrl', 'args': {'url': 'https://example.com/a'}}}},
            {'id': 'bad', 'component': 'Button', 'child': 'label', 'action': {'functionCall': {'call': 'openUrl', 'args': {'url': 'file:///etc/passwd'}}}},
          ]),
        ]);
      final opened = <Uri>[];
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!, onOpenUrl: (url) async => opened.add(url))));
      await tester.tap(find.byKey(const ValueKey('a2ui button good')));
      await tester.tap(find.byKey(const ValueKey('a2ui button bad')));
      expect(opened, [Uri.parse('https://example.com/a')]);
    });

    testWidgets('a table draws its columns over the rows of the data model, cut at the row limit', (tester) async {
      final p = processor(limits: const A2uiLimits(maxRows: 2))
        ..receive([
          create('s', data: {'rows': [{'a': 1, 'b': 'x'}, {'a': 2, 'b': 'y'}, {'a': 3, 'b': 'z'}]}, components: [
            {'id': 'root', 'component': 'Table', 'columns': [{'title': 'A', 'path': 'a'}, {'title': 'B', 'path': 'b'}], 'rows': {'path': '/rows'}},
          ]),
        ]);
      await tester.pumpWidget(app(A2uiSurfaceView(surface: p.surfaces['s']!)));
      expect(find.text('2'), findsOneWidget);
      expect(find.text('z'), findsNothing);
      expect(find.text('1 more rows are not shown.'), findsOneWidget);
    });
  });

  group('themed from the look', () {
    final surface = processor()
      ..receive([
        create('s', components: [
          {'id': 'root', 'component': 'Column', 'children': ['ok', 'bad', 'p', 'card']},
          {'id': 'ok', 'component': 'Badge', 'text': 'Fine', 'tone': 'success'},
          {'id': 'bad', 'component': 'Badge', 'text': 'Broken', 'tone': 'danger'},
          {'id': 'p', 'component': 'Progress', 'value': 0.5, 'label': 'Half'},
          {'id': 'card', 'component': 'Card', 'child': 'code'},
          {'id': 'code', 'component': 'CodeBlock', 'text': 'let x = 1;'},
        ]),
      ]);

    for (final (look, brightness) in [(LookId.canoa, Brightness.dark), (LookId.canoa, Brightness.light), (LookId.terminal, Brightness.dark)]) {
      testWidgets('${look.name} ${brightness.name}: states, the bar and the card come from the look, not the surface', (tester) async {
        await tester.pumpWidget(app(A2uiSurfaceView(surface: surface.surfaces['s']!), look: look, brightness: brightness));
        final tokens = themeFor(look, brightness).extension<CanoaLook>()!;
        Color? colourOf(String text) => tester.widget<Text>(find.text(text)).style?.color;
        expect(colourOf('Fine'), tokens.success);
        expect(colourOf('Broken'), tokens.danger);
        final bar = tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator));
        expect((bar.color, bar.backgroundColor), (tokens.accent, tokens.border));
        final code = tester.widget<Text>(find.text('let x = 1;'));
        expect((code.style?.fontFamily, code.style?.color), (CanoaLook.monospaceFont, tokens.text));
        final card = tester.widget<Card>(find.byType(Card));
        expect(card.color, isNull, reason: 'the card takes the theme\'s own colour');
        expect(Theme.of(tester.element(find.byType(Card))).cardTheme.color ?? tokens.surface, isNotNull);
      });
    }

    testWidgets('the two looks draw the same surface in different colours', (tester) async {
      await tester.pumpWidget(app(A2uiSurfaceView(surface: surface.surfaces['s']!), look: LookId.canoa, brightness: Brightness.dark));
      final canoa = tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).backgroundColor;
      await tester.pumpWidget(app(A2uiSurfaceView(surface: surface.surfaces['s']!), look: LookId.terminal, brightness: Brightness.light));
      await tester.pump(const Duration(seconds: 1));
      expect(tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).backgroundColor, isNot(canoa));
    });
  });
}
