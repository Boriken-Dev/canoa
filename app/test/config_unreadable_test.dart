// A canoa.cfg the app cannot read is shown to the person, the app runs on its defaults, and nothing is written over the file until
// it can be read.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/canoa_config_file.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Shell implements DesktopShell {
  @override
  Future<void> Function()? beforeQuit;
  @override
  bool get isDesktop => true;
  @override
  Future<void> quit() async {}
  @override
  Future<void> update(CanoaConfig config) async {}
  @override
  Future<void> showWaiting(int count) async {}
  @override
  Future<String?> setStartAtLogin(bool on) async => 'no';
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test('the defaults of an unreadable file carry why, and survive the changes Settings makes', () {
    final config = CanoaConfig.unreadable('line 3: expected a table');
    expect(config.unreadable, 'line 3: expected a table');
    expect(config.brightness, const CanoaConfig().brightness);
    expect(config.copyWith(brightness: 'light').unreadable, isNotNull);
    expect(config.withHost(null).unreadable, isNotNull);
    expect(config.resetKey(['appearance', 'brightness']).unreadable, isNotNull);
    expect(config.source.withOwn('host', {'a': 1}).unreadable, isNotNull);
    expect(CanoaConfig.parse('').unreadable, isNull);
  });

  test('saving them is refused before anything is written', () async {
    await expectLater(saveConfig(CanoaConfig.unreadable('bad')), throwsA(isA<ConfigUnreadable>()));
    expect(const ConfigUnreadable('bad').message, allOf(contains('bad'), contains('writes nothing to the file')));
  });

  testWidgets('the app says so, and a change in Settings is not written', (tester) async {
    final dir = Directory.systemTemp.createTempSync('canoa-unreadable');
    addTearDown(() => dir.deleteSync(recursive: true));
    await tester.binding.setSurfaceSize(const Size(1200, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final written = <CanoaConfig>[];
    await tester.pumpWidget(CanoaApp(
      store: SessionStore(),
      config: CanoaConfig.unreadable('line 3: expected a table'),
      shell: _Shell(),
      configPath: '${dir.path}/canoa.cfg',
      writeConfig: (changed) async => written.add(changed),
    ));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('canoa.cfg could not be read (line 3: expected a table)'), findsOneWidget);

    await tester.tap(find.byTooltip('Canoa settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-nav-appearance')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-brightness')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Light').last);
    await tester.pumpAndSettle();
    expect(written, isEmpty, reason: 'the defaults went over a file that could not be read');
    expect(find.textContaining('writes nothing to the file'), findsOneWidget);
  });
}
