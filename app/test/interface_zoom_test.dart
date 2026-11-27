// Interface zoom: the steps, the setting in canoa.cfg, the widget that scales the whole interface, and the keys and control that change it.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/interface_zoom.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  group('steps', () {
    test('move one step toward larger or smaller, and stop at the ends', () {
      expect(steppedZoom(1, 1), 1.1);
      expect(steppedZoom(1, -1), 0.9);
      expect(steppedZoom(maxZoom, 1), maxZoom);
      expect(steppedZoom(minZoom, -1), minZoom);
    });

    test('a factor between two steps moves to the next one in that direction', () {
      expect(steppedZoom(1.15, 1), 1.25);
      expect(steppedZoom(1.15, -1), 1.1);
    });
  });

  group('canoa.cfg', () {
    test('writes zoom only when it is not 1, and reads it back', () {
      expect(const CanoaConfig().encode(), isNot(contains('zoom')));
      const chosen = CanoaConfig(zoom: 1.25);
      expect(chosen.encode(), contains('zoom = 1.25'));
      expect(CanoaConfig.parse(chosen.encode()).zoom, 1.25);
      expect(CanoaConfig.parse(const CanoaConfig().encode()).zoom, 1);
      expect(CanoaConfig.parse('#!toml\n[appearance]\nzoom = 2\n').zoom, 2);
    });

    test('refuses a factor that cannot be laid out', () {
      for (final bad in ['0', '-1', '9', '"big"']) {
        expect(() => CanoaConfig.parse('#!toml\n[appearance]\nzoom = $bad\n'), throwsFormatException, reason: bad);
      }
    });

    test('is one of the appearance keys the app edits', () {
      expect(CanoaConfig.editedKeys['appearance'], contains('zoom'));
    });
  });

  group('the widget', () {
    Size drawn(WidgetTester tester) {
      final target = find.byKey(const Key('target'));
      final corner = tester.getBottomRight(target) - tester.getTopLeft(target);
      return Size(corner.dx, corner.dy);
    }

    Widget probe(double factor, {required void Function(Size) onSize, required VoidCallback onTap}) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(size: Size(800, 600)),
        child: InterfaceZoom(
          factor: factor,
          child: Builder(
            builder: (context) {
              onSize(MediaQuery.sizeOf(context));
              return Align(
                alignment: Alignment.topLeft,
                child: GestureDetector(onTap: onTap, child: const SizedBox(key: Key('target'), width: 100, height: 50, child: ColoredBox(color: Color(0xFF000000)))),
              );
            },
          ),
        ),
      ),
    );

    testWidgets('lays the child out in a window 1/factor the size and draws it factor times as large', (tester) async {
      await tester.binding.setSurfaceSize(const Size(800, 600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      Size? seen;
      await tester.pumpWidget(probe(2, onSize: (size) => seen = size, onTap: () {}));
      expect(seen, const Size(400, 300));
      expect(drawn(tester), const Size(200, 100));
      await tester.pumpWidget(probe(0.5, onSize: (size) => seen = size, onTap: () {}));
      expect(seen, const Size(1600, 1200));
      expect(drawn(tester), const Size(50, 25));
    });

    testWidgets('lands a tap where the scaled child is drawn', (tester) async {
      await tester.binding.setSurfaceSize(const Size(800, 600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      var taps = 0;
      await tester.pumpWidget(probe(2, onSize: (_) {}, onTap: () => taps++));
      await tester.tapAt(const Offset(150, 75));
      expect(taps, 1, reason: 'inside the 200 x 100 it is drawn at');
      await tester.tapAt(const Offset(250, 75));
      expect(taps, 1, reason: 'outside it, though inside the 100 x 50 it is laid out at');
    });

    testWidgets('at 1 draws the child as it is', (tester) async {
      await tester.binding.setSurfaceSize(const Size(800, 600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      Size? seen;
      await tester.pumpWidget(probe(1, onSize: (size) => seen = size, onTap: () {}));
      expect(seen, const Size(800, 600));
    });
  });

  group('in the app', () {
    Future<List<CanoaConfig>> app(WidgetTester tester, {CanoaConfig config = const CanoaConfig()}) async {
      await tester.binding.setSurfaceSize(const Size(1280, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final written = <CanoaConfig>[];
      await tester.pumpWidget(CanoaApp(store: SessionStore(), config: config, writeConfig: (changed) async => written.add(changed)));
      await tester.pump();
      return written;
    }

    Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
    }

    testWidgets('Ctrl+= and Ctrl+- step the zoom and Ctrl+0 returns it, each kept in canoa.cfg', (tester) async {
      final written = await app(tester);
      await press(tester, LogicalKeyboardKey.equal);
      expect(written.last.zoom, 1.1);
      await press(tester, LogicalKeyboardKey.equal);
      expect(written.last.zoom, 1.25);
      await press(tester, LogicalKeyboardKey.minus);
      expect(written.last.zoom, 1.1);
      await press(tester, LogicalKeyboardKey.digit0);
      expect(written.last.zoom, 1);
      expect(written, hasLength(4));
    });

    testWidgets('the whole interface is laid out smaller than the window at 2', (tester) async {
      await app(tester, config: const CanoaConfig(zoom: 2));
      final app0 = tester.element(find.byType(Scaffold).first);
      expect(MediaQuery.sizeOf(app0), const Size(640, 400));
    });
  });
}
