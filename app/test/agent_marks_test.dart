// The vendors' marks: what a host lists is the files of the vendor crates, each listed provider draws its mark, any other
// id and a command a person wrote keep the generic icons, and the source picker draws them in both looks.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/providers/provider_catalog.dart';
import 'package:canoa/sources/agent_marks.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/sources/svg_path.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/vendor_looks_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _known = ['claude', 'codex', 'antigravity', 'grok', 'opencode', 'pi'];

Widget _host(Widget child, {Color? ink}) => MaterialApp(
  home: Scaffold(body: Center(child: IconTheme(data: IconThemeData(color: ink), child: child))),
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/agent_marks_test.dart')));

  test('the marks a host lists are the files of the vendor crates', () {
    expect([for (final each in providerCatalog.providers) if (each.mark != null) each.id], _known);
    for (final id in _known) {
      final svg = File('../crates/canoa-$id/mark.svg').readAsStringSync();
      final root = RegExp('<svg[^>]*>').firstMatch(svg)![0]!;
      final box = RegExp(r'viewBox="0 0 (\d+) (\d+)"').firstMatch(root)!;
      final mark = providerCatalog.byId(id)!.mark!;
      expect((double.parse(box[1]!), double.parse(box[2]!)), (mark.side, mark.side), reason: id);
      final shapes = RegExp('<path[^>]*>').allMatches(svg).map((m) => m[0]!).toList();
      expect(shapes, hasLength(mark.shapes.length), reason: id);
      for (var i = 0; i < shapes.length; i++) {
        expect(RegExp(r'\sd="([^"]*)"').firstMatch(shapes[i])![1], mark.shapes[i].$1, reason: '$id shape $i');
        final fill = RegExp(r'\sfill="#([0-9A-Fa-f]{6})"').firstMatch(shapes[i]) ?? RegExp(r'\sfill="#([0-9A-Fa-f]{6})"').firstMatch(root);
        expect(mark.shapes[i].$2, fill == null ? isNull : Color(0xFF000000 | int.parse(fill[1]!, radix: 16)), reason: '$id shape $i');
      }
    }
  });

  test('every mark parses and stays inside its box', () {
    for (final provider in providerCatalog.providers) {
      final mark = provider.mark;
      if (mark == null) continue;
      for (final (d, _) in mark.shapes) {
        final bounds = parseSvgPath(d).getBounds();
        expect(bounds.isEmpty, isFalse, reason: provider.id);
        // getBounds is the control points' box, which an arc's curve overshoots a little.
        final side = mark.side, slack = side * 0.3;
        expect(bounds.left, greaterThan(-slack), reason: provider.id);
        expect(bounds.top, greaterThan(-slack), reason: provider.id);
        expect(bounds.right, lessThan(side + slack), reason: provider.id);
        expect(bounds.bottom, lessThan(side + slack), reason: provider.id);
      }
    }
  });

  test('text that is no square mark with a path is no mark', () {
    expect(parseMarkSvg('<svg viewBox="0 0 10 20"><path d="M0 0H1Z"/></svg>'), isNull);
    expect(parseMarkSvg('<svg viewBox="0 0 10 10"></svg>'), isNull);
    expect(parseMarkSvg('not svg'), isNull);
    final mark = parseMarkSvg('<svg viewBox="0 0 10 10" fill="#112233"><path d="M0 0H1Z"/><path fill="#445566" d="M1 1H2Z"/></svg>')!;
    expect(mark.side, 10.0);
    expect(mark.shapes.map((each) => each.$2).toList(), [const Color(0xFF112233), const Color(0xFF445566)]);
  });

  test('a path command the marks do not use is refused', () {
    expect(() => parseSvgPath('M 0 0 Q 1 1 2 2'), throwsFormatException);
    expect(parseSvgPath('M0 0h10v10H0z').getBounds(), const Rect.fromLTRB(0, 0, 10, 10));
    expect(parseSvgPath('M0 5a5 5 0 0 1 10 0').getBounds().width, closeTo(10, 0.01));
  });

  for (final id in _known) {
    testWidgets('provider $id draws its own mark, not an icon', (tester) async {
      await tester.pumpWidget(_host(SourceMark.provider(id, size: 20)));
      expect(find.byType(Icon), findsNothing);
      expect(find.descendant(of: find.byType(SourceMark), matching: find.byType(CustomPaint)), findsOneWidget);
      expect(tester.getSize(find.byType(SourceMark)), const Size(20, 20));
    });
  }

  testWidgets('an unknown provider, none and a written command keep the generic icons', (tester) async {
    for (final (widget, icon) in [
      (const SourceMark.provider('something-else'), Icons.dns_outlined),
      (const SourceMark.provider(null), Icons.dns_outlined),
      (const SourceMark.of(null), Icons.terminal),
      (const SourceMark.of(AgentSource.hosted('mine')), Icons.dns_outlined),
      (const SourceMark.of(AgentSource.hosted('Scripted', local: true)), Icons.dns_outlined),
    ]) {
      await tester.pumpWidget(_host(widget));
      expect(find.byIcon(icon), findsOneWidget, reason: '$widget');
    }
  });

  testWidgets('a source draws the mark of its provider, from its entry or from the name of the host\'s own', (tester) async {
    for (final source in const [
      AgentSource.hosted('Claude', harness: 'claude', local: true),
      AgentSource.hosted('review', harness: 'codex'),
      AgentSource.hosted('Antigravity', local: true),
      AgentSource.hosted('Pi', local: true),
      AgentSource.hosted('Grok', local: true),
      AgentSource.hosted('OpenCode', local: true),
    ]) {
      await tester.pumpWidget(_host(SourceMark.of(source)));
      expect(find.byType(Icon), findsNothing, reason: source.label);
    }
  });

  testWidgets('monochrome marks take the text colour, coloured ones keep the vendor\'s', (tester) async {
    Future<Set<int>> inks(String id, Color ink) async {
      final key = GlobalKey();
      await tester.pumpWidget(_host(RepaintBoundary(key: key, child: SourceMark.provider(id, size: 24)), ink: ink));
      final image = (await tester.runAsync(() async {
        final boundary = key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
        return await boundary.toImage(pixelRatio: 2);
      }))!;
      final bytes = (await tester.runAsync(() async => (await image.toByteData())!))!;
      final seen = <int>{};
      for (var i = 0; i < bytes.lengthInBytes; i += 4) {
        if (bytes.getUint8(i + 3) == 255) seen.add(bytes.getUint8(i) << 16 | bytes.getUint8(i + 1) << 8 | bytes.getUint8(i + 2));
      }
      return seen;
    }

    expect(await inks('codex', const Color(0xFF123456)), {0x123456});
    expect(await inks('codex', const Color(0xFFABCDEF)), {0xABCDEF});
    expect(await inks('claude', const Color(0xFF123456)), {0xD97757});
    expect(await inks('antigravity', const Color(0xFFABCDEF)), {0x3186FF});
    expect(await inks('opencode', const Color(0xFFABCDEF)), {0xABCDEF});
    expect(await inks('pi', const Color(0xFFABCDEF)), {0xF09082, 0x4D9ABF, 0xF1BE58});
  });

  for (final look in allLooks) {
    testWidgets('the source picker shows each provider\'s mark in the ${look.label} look', (tester) async {
      await pumpApp(tester, SessionStore(), config: CanoaConfig(look: look, agents: [scripted]), size: const Size(900, 640));
      await newSession(tester);
      await tester.tap(find.byKey(const Key('new-session-source')));
      await frames(tester);
      expect(find.byKey(const Key('source Claude')), findsOneWidget);
      for (final label in ['Claude', 'Codex', 'Antigravity', 'Grok', 'Pi']) {
        expect(find.descendant(of: find.byKey(Key('source $label')), matching: find.byType(CustomPaint)), findsWidgets, reason: label);
      }
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_picker_${look.name}.png'));
    });
  }

  testWidgets('each session row in the rail carries the mark of its source', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    const sources = [
      AgentSource.hosted('Claude', harness: 'claude', local: true),
      AgentSource.hosted('Codex', harness: 'codex', local: true),
      AgentSource.hosted('Antigravity', local: true),
      AgentSource.hosted('Pi', local: true),
      AgentSource.hosted('Grok', local: true),
    ];
    for (final (index, source) in sources.indexed) {
      // Started sessions: a new one has no row until it is sent.
      store.createDraft(source: source, title: 'Session ${index + 1}').status = SessionStatus.connected;
    }
    await pumpApp(tester, store, size: const Size(900, 640));
    await frames(tester);
    expect(find.byType(SourceMark), findsAtLeastNWidgets(sources.length));
    expect(tester.takeException(), isNull);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/agent_marks_rail.png'));
  });
}
