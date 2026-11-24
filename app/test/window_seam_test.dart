// The windowing seam: one file reaches Flutter's multi-window API, and what it does with that API is checked against the test
// binding's own windowing owner, which makes real controllers and views without a native window.
import 'dart:io';

import 'package:canoa/popout/flutter_windows.dart';
import 'package:canoa/popout/popout_window.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

/// What a window draws: a window has no app above it to give it a reading direction.
Widget _words(String text) => Directionality(textDirection: TextDirection.ltr, child: Text(text));

void main() {
  test('only the seam imports Flutter\'s windowing API', () {
    final importers = <String>[];
    for (final root in ['lib', 'test', 'integration_test']) {
      for (final entity in Directory(root).listSync(recursive: true)) {
        if (entity is! File || !entity.path.endsWith('.dart')) continue;
        final path = entity.path.replaceAll(r'\', '/');
        // This file names what it looks for.
        if (path == 'test/window_seam_test.dart') continue;
        if (RegExp(r'''import\s+['"]package:flutter/src/(widgets/_window|foundation/_features)''').hasMatch(entity.readAsStringSync())) {
          importers.add(path);
        }
      }
    }
    expect(importers, ['lib/popout/flutter_windows.dart']);
  });

  test('the seam says why its import is an exception to the lint', () {
    final source = File('lib/popout/flutter_windows.dart').readAsStringSync();
    expect(source, contains('// ignore: implementation_imports'));
    expect(source, contains('invalid_use_of_internal_member'));
  });

  group('over the test binding\'s windows', () {
    late FlutterWindows windows;

    setUp(() => windows = FlutterWindows(brand: 'Canoa'));
    tearDown(() => windows.dispose());

    Widget app(Widget content) => windows.host(Directionality(textDirection: TextDirection.ltr, child: content));

    testWidgets('is available where the build has windowing on, and opens nothing without a builder', (tester) async {
      expect(windows.available, isTrue, reason: 'flutter test runs with the windowing feature on');
      await expectLater(windows.open(title: 'T', look: 'canoa', density: 'normal'), throwsArgumentError);
    });

    testWidgets('a window is listed with its title and size, and what its builder gives is drawn beside the app', (tester) async {
      await tester.pumpWidget(app(const Text('main')));
      final seen = <int>[];
      windows.addListener(() => seen.add(windows.windows.length));

      final window = await windows.open(
        title: 'Fix calc',
        look: 'canoa',
        density: 'normal',
        size: const Size(400, 500),
        builder: (_) => _words('in the window'),
      );
      await tester.pump();

      expect(windows.windows, [window]);
      expect(seen, [1]);
      expect(window.link, isNull, reason: 'a window of the engine draws the conversation itself');
      expect(window.title, 'Fix calc');
      expect(window.size, const Size(400, 500));
      expect(find.text('main'), findsOneWidget);
      expect(find.text('in the window'), findsOneWidget);
      expect(tester.viewOf(find.text('in the window')).viewId, isNot(tester.viewOf(find.text('main')).viewId), reason: 'its own view');
    });

    testWidgets('several windows are listed in the order they opened, each with its own id', (tester) async {
      await tester.pumpWidget(app(const Text('main')));
      final a = await windows.open(title: 'A', look: 'canoa', density: 'normal', builder: (_) => _words('a'));
      final b = await windows.open(title: 'B', look: 'canoa', density: 'normal', builder: (_) => _words('b'));
      await tester.pump();
      expect(windows.windows, [a, b]);
      expect(a.id, isNot(b.id));
      expect(find.text('a'), findsOneWidget);
      expect(find.text('b'), findsOneWidget);
    });

    testWidgets('title and size are set, and focus and close go to the window', (tester) async {
      await tester.pumpWidget(app(const Text('main')));
      final window = await windows.open(title: 'A', look: 'canoa', density: 'normal', builder: (_) => _words('a'));
      await tester.pump();

      window.setTitle('B');
      expect(window.title, 'B');
      window.setSize(const Size(300, 300));
      expect(window.size, const Size(300, 300));
      await window.focus();

      await window.close();
      await tester.pump();
      await window.closed;
      expect(windows.windows, isEmpty);
      expect(find.text('a'), findsNothing);
      expect(window.size, Size.zero);
      // Asking a window that is gone is not an error.
      window.setTitle('C');
      window.setSize(const Size(10, 10));
      await window.focus();
      await window.close();
    });

    testWidgets('closing one window leaves the app and the other windows as they were', (tester) async {
      await tester.pumpWidget(app(const Text('main')));
      final a = await windows.open(title: 'A', look: 'canoa', density: 'normal', builder: (_) => _words('a'));
      final b = await windows.open(title: 'B', look: 'canoa', density: 'normal', builder: (_) => _words('b'));
      await tester.pump();

      await a.close();
      await tester.pump();
      expect(windows.windows, [b]);
      expect(find.text('main'), findsOneWidget);
      expect(find.text('b'), findsOneWidget);
      expect(find.text('a'), findsNothing);
    });
  });

  test('a platform with no windows offers none, and its host is the app itself', () {
    final none = NoPopoutWindows();
    expect(none.available, isFalse);
    expect(none.windows, isEmpty);
    const app = SizedBox();
    expect(none.host(app), same(app));
    expect(() => none.open(title: 'T', look: 'canoa', density: 'normal'), throwsUnsupportedError);
  });
}
