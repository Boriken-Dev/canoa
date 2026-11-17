// The glyphs a runtime's store may name as its icon: the Dart path data is the files in assets/icons/runtimes (both written by
// tool/runtime_glyphs.py), the names are the ones the Rust crate lists, each draws in the text colour on its tile, and a store that
// names none, or one the app does not draw, gets the generic icon.
import 'dart:io';

import 'package:canoa/sources/svg_path.dart';
import 'package:canoa/store/runtime_glyphs.dart';
import 'package:canoa/store/runtime_mark.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// The names `STORE_GLYPHS` lists in crates/canoa/src/store.rs.
List<String> _rustNames() {
  final source = File('../crates/canoa/src/store.rs').readAsStringSync();
  final list = RegExp(r'STORE_GLYPHS: \[&str; (\d+)\] = \[([^\]]*)\]').firstMatch(source)!;
  final names = RegExp(r'"([a-z]+)"').allMatches(list[2]!).map((each) => each[1]!).toList();
  expect(names, hasLength(int.parse(list[1]!)));
  return names;
}

Widget _host(Widget child, {Brightness brightness = Brightness.light}) => MaterialApp(
  debugShowCheckedModeBanner: false,
  theme: ThemeData(colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF3DAFA1), brightness: brightness)),
  home: Scaffold(body: Center(child: child)),
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/runtime_glyphs_test.dart')));

  test('the glyphs are the Rust crate\'s names and the path data of the files beside them', () {
    expect(runtimeGlyphs.keys.toList(), _rustNames());
    for (final MapEntry(:key, :value) in runtimeGlyphs.entries) {
      final svg = File('assets/icons/runtimes/$key.svg').readAsStringSync();
      expect(RegExp(r'viewBox="0 0 (\d+) (\d+)"').firstMatch(svg)!.groups([1, 2]), ['${value.side.toInt()}', '${value.side.toInt()}'], reason: key);
      final paths = RegExp(r'<path d="([^"]*)"').allMatches(svg).map((each) => each[1]!).toList();
      expect(paths, [for (final (d, _) in value.shapes) d], reason: key);
      expect(value.shapes.every((shape) => shape.$2 == null), isTrue, reason: '$key is one colour, the text\'s');
    }
  });

  test('every glyph parses, fills its box and stays inside it', () {
    for (final MapEntry(:key, :value) in runtimeGlyphs.entries) {
      var box = Rect.zero;
      for (final (d, _) in value.shapes) {
        final bounds = parseSvgPath(d).getBounds();
        expect(bounds.isEmpty, isFalse, reason: key);
        box = box.isEmpty ? bounds : box.expandToInclude(bounds);
      }
      expect(box.left, greaterThanOrEqualTo(0), reason: key);
      expect(box.top, greaterThanOrEqualTo(0), reason: key);
      expect(box.right, lessThanOrEqualTo(value.side), reason: key);
      expect(box.bottom, lessThanOrEqualTo(value.side), reason: key);
      expect(box.width, greaterThan(value.side * 0.6), reason: '$key is drawn small');
      expect(box.height, greaterThan(value.side * 0.6), reason: '$key is drawn small');
    }
  });

  for (final name in runtimeGlyphs.keys) {
    testWidgets('the $name glyph is drawn in the text colour of its tile', (tester) async {
      final key = GlobalKey();
      await tester.pumpWidget(_host(RepaintBoundary(key: key, child: RuntimeMark(icon: name, size: 48))));
      expect(find.byKey(Key('runtime-glyph-$name')), findsOneWidget);
      expect(find.byType(Icon), findsNothing);
      final scheme = Theme.of(tester.element(find.byType(RuntimeMark))).colorScheme;
      final bytes = (await tester.runAsync(() async {
        final image = await (key.currentContext!.findRenderObject()! as RenderRepaintBoundary).toImage();
        return (await image.toByteData())!;
      }))!;
      final seen = <int>{};
      for (var i = 0; i < bytes.lengthInBytes; i += 4) {
        if (bytes.getUint8(i + 3) == 255) seen.add(bytes.getUint8(i) << 16 | bytes.getUint8(i + 1) << 8 | bytes.getUint8(i + 2));
      }
      int rgb(Color c) => (c.toARGB32() & 0xFFFFFF);
      expect(seen, contains(rgb(scheme.secondaryContainer)));
      expect(seen, contains(rgb(scheme.onSecondaryContainer)), reason: 'the glyph is drawn');
    });
  }

  testWidgets('a store that names no glyph, or one the app does not draw, gets the generic icon', (tester) async {
    for (final icon in [null, 'nothing-like-it', '']) {
      await tester.pumpWidget(_host(RuntimeMark(icon: icon)));
      expect(find.byIcon(Icons.memory), findsOneWidget, reason: '$icon');
      expect(find.byType(CustomPaint).evaluate().where((each) => (each.widget as CustomPaint).painter != null), isEmpty, reason: '$icon');
    }
  });

  for (final brightness in Brightness.values) {
    testWidgets('the glyphs at the two sizes the Store draws them, ${brightness.name}', (tester) async {
      tester.view.physicalSize = const Size(640, 220);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(_host(
        Column(mainAxisSize: MainAxisSize.min, children: [
          for (final size in const [56.0, 32.0])
            Padding(
              padding: const EdgeInsets.all(8),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                for (final name in runtimeGlyphs.keys) Padding(padding: const EdgeInsets.all(6), child: RuntimeMark(icon: name, size: size)),
                Padding(padding: const EdgeInsets.all(6), child: RuntimeMark(icon: null, size: size)),
              ]),
            ),
        ]),
        brightness: brightness,
      ));
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/runtime_glyphs_${brightness.name}.png'));
    });
  }
}
