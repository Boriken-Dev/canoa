import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/canoa_mark.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the painter draws the paths of canoa.svg', () {
    final svg = File('assets/icons/canoa.svg').readAsStringSync();
    String attribute(String id, String name) {
      final tag = RegExp('<path id="$id"[^>]*>').firstMatch(svg)?[0];
      expect(tag, isNotNull, reason: 'canoa.svg has no path $id');
      return RegExp('\\s$name="([^"]*)"').firstMatch(tag!)![1]!;
    }

    expect(attribute('hull', 'd'), CanoaMarkPainter.hull);
    expect(attribute('cut', 'd'), CanoaMarkPainter.cut);
    expect(attribute('spiral', 'd'), CanoaMarkPainter.spiral);
    expect(double.parse(attribute('spiral', 'stroke-width')), CanoaMarkPainter.spiralWidth);
    expect(attribute('paddle', 'd'), CanoaMarkPainter.paddle);
  });

  test('a path command the mark does not use is refused', () {
    expect(() => CanoaMarkPainter.parse('M 0 0 Q 1 1 2 2'), throwsFormatException);
  });

  testWidgets('the rail\'s wordmark is the canoe in the look\'s accent', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    addTearDown(store.dispose);
    await tester.pumpWidget(CanoaApp(store: store, config: CanoaConfig(look: LookId.bubbles)));
    await tester.pumpAndSettle();
    final mark = tester.widget<CanoaMark>(find.byType(CanoaMark));
    expect(mark.color, tester.element(find.byType(CanoaMark)).look.accent);
    expect(tester.getSize(find.byType(CanoaMark)), const Size(24, 24));
    expect(find.byIcon(Icons.sailing_rounded), findsNothing);
  });

  testWidgets('the mark drops the paddle below 20 px and carves the spiral from 64 px', (tester) async {
    await tester.binding.setSurfaceSize(const Size(420, 280));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: Center(child: RepaintBoundary(child: ColoredBox(
        color: const Color(0xFF17242D),
        child: Padding(padding: const EdgeInsets.all(8), child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            for (final size in [16.0, 24.0, 64.0, 256.0]) ...[
              CanoaMark(size: size, color: const Color(0xFF6DE2D2)),
              const SizedBox(width: 8),
            ],
          ],
        )),
      ))),
    ));
    await expectLater(find.byType(RepaintBoundary).first, matchesGoldenFile('goldens/canoa_mark.png'));
  });
}
