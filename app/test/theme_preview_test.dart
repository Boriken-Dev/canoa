// A look or brightness chosen in Settings is the whole app's at once and is written as it is chosen; the page's own preview
// shows the choice in a sample conversation.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
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
  await tester.tap(find.byKey(const Key('settings-nav-appearance')));
  await frames(tester);
  return written;
}

Future<void> _choose(WidgetTester tester, String key, String label) async {
  await tester.tap(find.byKey(Key(key)));
  await frames(tester);
  await tester.tap(find.text(label).last);
  await frames(tester);
}

void main() {
  testWidgets('picking a look or brightness changes the app at once and writes it, with Settings still open', (tester) async {
    final written = await _open(tester);
    expect(_look(tester), LookId.canoa);
    await _choose(tester, 'settings-look', 'Terminal');
    expect(find.byKey(const Key('settings-section-appearance')), findsOneWidget);
    expect(_look(tester), LookId.terminal);
    expect(written.last.look, LookId.terminal);
    await _choose(tester, 'settings-brightness', 'Light');
    expect(_mode(tester), ThemeMode.light);
    expect(written.last.brightness, 'light');
    expect(written.last.look, LookId.terminal, reason: 'the second change keeps the first');
    expect(find.text('Save'), findsNothing);
    await finish(tester);
  });

  testWidgets('what was chosen is there when Settings is opened again', (tester) async {
    await _open(tester);
    await _choose(tester, 'settings-look', 'Terminal');
    await tester.pageBack();
    await frames(tester);
    await tester.tap(find.byTooltip('Canoa settings'));
    await frames(tester);
    await tester.tap(find.byKey(const Key('settings-nav-appearance')));
    await frames(tester);
    expect(tester.widget<DropdownButton<LookId>>(find.byKey(const Key('settings-look'))).value, LookId.terminal);
    await finish(tester);
  });
}
