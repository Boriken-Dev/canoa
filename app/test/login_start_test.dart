// Starting at sign-in: the entries each system reads (a scratch registry key, a temporary autostart folder, a temporary launch-agent
// folder, never the person's own), what the shell does with the choice, and that the first run decides once and never again.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/desktop/login_start.dart';
import 'package:canoa/desktop/login_start_io.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:canoa/desktop/desktop_shell_io.dart';

final class _Recorded implements LoginStart {
  final calls = <bool>[];
  bool on = false;
  @override
  Future<bool> isOn() async => on;
  @override
  Future<String?> set(bool value) async {
    calls.add(value);
    on = value;
    return null;
  }
}

final class _Shell implements DesktopShell {
  _Shell({this.accepts = true});
  bool accepts;
  final asked = <bool>[];
  final updates = <bool?>[];
  @override
  Future<void> Function()? beforeQuit;
  @override
  bool get isDesktop => true;
  @override
  Future<String?> setStartAtLogin(bool on) async {
    asked.add(on);
    return accepts ? null : 'Access is denied.';
  }

  @override
  Future<void> update(CanoaConfig config) async => updates.add(config.startAtLogin);
  @override
  Future<void> quit() async {}
  @override
  Future<void> showWaiting(int count) async {}
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('Windows Run key', () {
    late RunKeyLoginStart entry;

    setUp(() {
      // A key of the test's own, removed after: the Run key itself is never named here.
      entry = RunKeyLoginStart(
        key: 'Software\\CanoaTest\\login${DateTime.now().microsecondsSinceEpoch}',
        command: r'C:\Program Files\Canoa\CanoaUI.exe',
      );
      expect(entry.key, startsWith('Software\\CanoaTest\\'));
    });
    tearDown(() async => entry.deleteKey());

    test('on writes the quoted program with the background word, off removes it, off again is fine', () async {
      expect(await entry.isOn(), isFalse);
      expect(await entry.set(true), isNull);
      expect(await entry.isOn(), isTrue);
      expect(await entry.read(), r'"C:\Program Files\Canoa\CanoaUI.exe" --background');
      expect(await entry.set(false), isNull);
      expect(await entry.isOn(), isFalse);
      expect(await entry.set(false), isNull);
    });
  }, skip: Platform.isWindows ? false : 'the registry is Windows only');

  group('files', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('canoa-login'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('an XDG autostart entry quotes a path with spaces and is removed again', () async {
      final entry = DesktopEntryLoginStart(folder: '${dir.path}/config/autostart', command: '/opt/My Canoa/CanoaUI');
      expect(await entry.isOn(), isFalse);
      expect(await entry.set(true), isNull);
      final text = File('${dir.path}/config/autostart/canoa.desktop').readAsStringSync();
      expect(text, contains('Exec="/opt/My Canoa/CanoaUI" --background\n'));
      expect(text, contains('Type=Application'));
      expect(await entry.isOn(), isTrue);
      expect(await entry.set(false), isNull);
      expect(await entry.isOn(), isFalse);
      expect(await entry.set(false), isNull);
    });

    test('a launch agent runs at load with the background word', () async {
      final entry = LaunchAgentLoginStart(folder: '${dir.path}/LaunchAgents', command: '/Apps/Canoa & Co/CanoaUI');
      expect(await entry.set(true), isNull);
      final text = File('${dir.path}/LaunchAgents/dev.boriken.canoa.plist').readAsStringSync();
      expect(text, contains('<string>/Apps/Canoa &amp; Co/CanoaUI</string>'));
      expect(text, contains('<string>--background</string>'));
      expect(text, contains('<key>RunAtLoad</key>\n  <true/>'));
      expect(await entry.set(false), isNull);
      expect(await entry.isOn(), isFalse);
    });

    test('a folder that cannot be written says so', () async {
      final blocker = File('${dir.path}/blocked')..writeAsStringSync('');
      expect(await DesktopEntryLoginStart(folder: '${blocker.path}/autostart', command: 'c').set(true), contains('blocked'));
    });
  });

  group('the config key', () {
    test('absent until decided, then kept either way', () {
      expect(CanoaConfig.parse('').startAtLogin, isNull);
      expect(const CanoaConfig().encode(), isNot(contains('start_at_login')));
      for (final value in [true, false]) {
        final saved = CanoaConfig.parse(const CanoaConfig().copyWith(startAtLogin: value).encode());
        expect(saved.startAtLogin, value);
        expect(saved.withHost(null).startAtLogin, value);
      }
    });
  });

  group('the tray shell', () {
    final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    final calls = <String>[];
    setUp(() {
      calls.clear();
      messenger.setMockMethodCallHandler(const MethodChannel('window_manager'), (call) async {
        calls.add('window.${call.method}');
        return call.method.startsWith('is') ? call.method == 'isVisible' : null;
      });
      messenger.setMockMethodCallHandler(const MethodChannel('tray_manager'), (call) async => null);
    });
    tearDown(() {
      messenger.setMockMethodCallHandler(const MethodChannel('window_manager'), null);
      messenger.setMockMethodCallHandler(const MethodChannel('tray_manager'), null);
    });

    Future<TrayShell> started(_Recorded entry, CanoaConfig config, {bool background = false}) async {
      final shell = TrayShell(trayAvailable: () async => true, desktop: true, loginStart: entry);
      await shell.initialize(config, onSettings: () {}, background: background);
      addTearDown(shell.dispose);
      return shell;
    }

    test('nothing is registered before the choice exists, and the choice is applied when it changes', () async {
      final entry = _Recorded();
      final shell = await started(entry, const CanoaConfig());
      await shell.update(const CanoaConfig());
      expect(entry.calls, isEmpty);
      await shell.update(const CanoaConfig(startAtLogin: true));
      await shell.update(const CanoaConfig(startAtLogin: true, closeToTray: false));
      expect(entry.calls, [true], reason: 'a change of another setting does not register again');
      await shell.update(const CanoaConfig(startAtLogin: false));
      expect(entry.calls, [true, false]);
    });

    test('a start with the choice made registers it again, so a moved program is found', () async {
      final entry = _Recorded();
      await started(entry, const CanoaConfig(startAtLogin: true));
      expect(entry.calls, [true]);
    });

    test('the background word starts with no window, and its absence leaves it', () async {
      await started(_Recorded(), const CanoaConfig(), background: true);
      expect(calls, contains('window.hide'));
      calls.clear();
      await started(_Recorded(), const CanoaConfig());
      expect(calls, isNot(contains('window.hide')));
    });
  });

  group('the first run', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('canoa-first'));
    tearDown(() => dir.deleteSync(recursive: true));

    Future<List<CanoaConfig>> run(WidgetTester tester, _Shell shell, CanoaConfig config) async {
      await tester.binding.setSurfaceSize(const Size(1200, 1600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final written = <CanoaConfig>[];
      await tester.pumpWidget(CanoaApp(
        store: SessionStore(),
        config: config,
        shell: shell,
        configPath: '${dir.path}/canoa.cfg',
        writeConfig: (changed) async => written.add(changed),
      ));
      await tester.pump();
      await tester.pump();
      return written;
    }

    testWidgets('turns it on once and writes the answer down', (tester) async {
      final shell = _Shell();
      final written = await run(tester, shell, const CanoaConfig());
      expect(shell.asked, [true]);
      expect(written.map((c) => c.startAtLogin), [true]);
    });

    testWidgets('does not ask again once canoa.cfg holds an answer, off included', (tester) async {
      final shell = _Shell();
      final written = await run(tester, shell, const CanoaConfig(startAtLogin: false));
      expect(shell.asked, isEmpty);
      expect(written, isEmpty);
    });

    testWidgets('a computer that cannot register it is asked again next time, and nothing is written', (tester) async {
      final shell = _Shell(accepts: false);
      final written = await run(tester, shell, const CanoaConfig());
      expect(shell.asked, [true]);
      expect(written, isEmpty);
    });

    testWidgets('Settings shows it on after the first run, and turning it off is applied and kept', (tester) async {
      final shell = _Shell();
      final written = await run(tester, shell, const CanoaConfig());
      await tester.tap(find.byTooltip('Canoa settings'));
      await tester.pumpAndSettle();
      final row = find.byKey(const Key('settings-start-at-login'));
      expect(row, findsOneWidget);
      expect(tester.widget<Switch>(find.descendant(of: row, matching: find.byType(Switch))).value, isTrue);
      await tester.tap(find.descendant(of: row, matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(shell.updates.last, isFalse);
      expect(written.last.startAtLogin, isFalse);
    });

    testWidgets('a registration the computer refuses is said, and the switch keeps what is registered', (tester) async {
      final shell = _Shell();
      final written = await run(tester, shell, const CanoaConfig());
      expect(written.map((c) => c.startAtLogin), [true]);
      await tester.tap(find.byTooltip('Canoa settings'));
      await tester.pumpAndSettle();
      final toggle = find.descendant(of: find.byKey(const Key('settings-start-at-login')), matching: find.byType(Switch));
      shell.accepts = false;
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(find.textContaining('could not be stopped from starting when you sign in: Access is denied.'), findsOneWidget);
      expect(tester.widget<Switch>(toggle).value, isTrue, reason: 'the entry is still registered');
      expect(written.map((c) => c.startAtLogin), everyElement(isTrue), reason: 'nothing false was written');
    });
  });
}
