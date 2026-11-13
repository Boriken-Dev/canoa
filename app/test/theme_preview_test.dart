// Settings previews a chosen look and brightness on the whole app at once; closing it any way but Save puts back what
// was in effect, and nothing is written until Save.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';

LookId _look(WidgetTester tester) =>
    tester.widget<MaterialApp>(find.byType(MaterialApp)).theme!.extension<CanoaLook>()!.id;

ThemeMode _mode(WidgetTester tester) => tester.widget<MaterialApp>(find.byType(MaterialApp)).themeMode!;

Future<List<CanoaConfig>> _open(WidgetTester tester) async {
  final written = await pumpApp(tester, SessionStore(),
      config: const CanoaConfig(look: LookId.canoa, brightness: 'dark'));
  await tester.tap(find.byTooltip('Canoa settings'));
  await frames(tester);
  return written;
}

Future<void> _choose(WidgetTester tester, String key, String label) async {
  await tester.tap(find.byKey(Key(key)));
  await frames(tester);
  await tester.tap(find.text(label).last);
  await frames(tester);
}

Future<void> _preview(WidgetTester tester) async {
  await _choose(tester, 'settings-look', 'Terminal');
  await _choose(tester, 'settings-brightness', 'Light');
}

void main() {
  testWidgets('picking a look or brightness changes the app at once, with Settings still open', (tester) async {
    final written = await _open(tester);
    expect(_look(tester), LookId.canoa);
    await _preview(tester);
    expect(find.text('Canoa settings'), findsOneWidget);
    expect(_look(tester), LookId.terminal);
    expect(_mode(tester), ThemeMode.light);
    expect(written, isEmpty, reason: 'nothing is written before Save');
    await finish(tester);
  });

  group('closing without saving puts back the look in effect before', () {
    Future<void> check(WidgetTester tester, Future<void> Function() close) async {
      final written = await _open(tester);
      await _preview(tester);
      await close();
      await frames(tester);
      expect(find.text('Canoa settings'), findsNothing);
      expect(_look(tester), LookId.canoa);
      expect(_mode(tester), ThemeMode.dark);
      expect(written, isEmpty);
      await finish(tester);
    }

    testWidgets('by Cancel', (tester) => check(tester, () => tester.tap(find.text('Cancel'))));
    testWidgets('by Escape', (tester) => check(tester, () => tester.sendKeyEvent(LogicalKeyboardKey.escape)));
    testWidgets('by a tap outside', (tester) => check(tester, () => tester.tapAt(const Offset(4, 4))));
    testWidgets('by back', (tester) => check(tester, () => tester.binding.handlePopRoute()));

    testWidgets('and a second Settings starts from the saved look, not the abandoned one', (tester) async {
      await _open(tester);
      await _preview(tester);
      await tester.tap(find.text('Cancel'));
      await frames(tester);
      await tester.tap(find.byTooltip('Canoa settings'));
      await frames(tester);
      expect(tester.widget<DropdownButton<LookId>>(find.byKey(const Key('settings-look'))).value, LookId.canoa);
      await finish(tester);
    });
  });

  testWidgets('Save keeps the previewed look and writes it', (tester) async {
    final written = await _open(tester);
    await _preview(tester);
    await tester.tap(find.text('Save'));
    await frames(tester);
    expect(_look(tester), LookId.terminal);
    expect(_mode(tester), ThemeMode.light);
    expect(written.last.look, LookId.terminal);
    expect(written.last.brightness, 'light');
    await finish(tester);
  });
}
