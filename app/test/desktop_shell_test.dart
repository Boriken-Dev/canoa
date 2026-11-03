// The window and tray decisions, through the real window_manager and tray_manager packages with their native sides
// answered here: what closing does with and without a tray, what a double click is, and that quitting ends what the app
// started before the window goes. The native sides themselves run in integration_test/desktop_test.dart.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final calls = <String>[];
  final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  const window = MethodChannel('window_manager');
  const tray = MethodChannel('tray_manager');
  var now = DateTime(2026);

  setUp(() {
    calls.clear();
    messenger.setMockMethodCallHandler(window, (call) async {
      if (!call.method.startsWith('is')) calls.add('window.${call.method}');
      // Asked before acting: a shown window that is not minimized.
      if (call.method.startsWith('is')) return call.method == 'isVisible';
      return null;
    });
    messenger.setMockMethodCallHandler(tray, (call) async {
      calls.add('tray.${call.method}');
      return null;
    });
  });

  tearDown(() {
    messenger.setMockMethodCallHandler(window, null);
    messenger.setMockMethodCallHandler(tray, null);
  });

  /// What the native side sends when something happens to the window or the icon.
  Future<void> nativeSays(MethodChannel channel, String method, [Object? arguments]) async {
    await messenger.handlePlatformMessage(
      channel.name,
      const StandardMethodCodec().encodeMethodCall(MethodCall(method, arguments)),
      (_) {},
    );
    await Future<void>.delayed(Duration.zero);
  }

  Future<TrayShell> started({
    bool hasTray = true,
    CanoaConfig config = const CanoaConfig(),
    void Function()? onHosting,
  }) async {
    final shell = TrayShell(trayAvailable: () async => hasTray, desktop: true, clock: () => now);
    await shell.initialize(config, onSettings: () {}, onHosting: onHosting);
    addTearDown(shell.dispose);
    calls.clear();
    return shell;
  }

  test('closing the window puts it in the tray, and nothing ends', () async {
    final shell = await started();
    await nativeSays(window, 'onEvent', {'eventName': 'close'});
    expect(calls, ['window.hide']);
    await shell.dispose();
  });

  test('with no tray to come back from, closing minimizes instead of hiding', () async {
    final shell = await started(hasTray: false);
    expect(shell.hasTray, isFalse);
    await nativeSays(window, 'onEvent', {'eventName': 'close'});
    expect(calls, ['window.minimize']);
    await shell.dispose();
  });

  test('starting hidden goes to the tray, or to the taskbar with no tray', () async {
    for (final (hasTray, expected) in [(true, 'window.hide'), (false, 'window.minimize')]) {
      calls.clear();
      final shell = TrayShell(trayAvailable: () async => hasTray, desktop: true);
      await shell.initialize(const CanoaConfig(startHidden: true), onSettings: () {});
      expect(calls.where((call) => call == 'window.hide' || call == 'window.minimize'), [expected]);
      expect(calls.any((call) => call.startsWith('tray.')), hasTray);
      await shell.dispose();
    }
  });

  test('two clicks on the icon within a double click open the window; one, or two far apart, do not', () async {
    final shell = await started();
    await nativeSays(tray, 'onTrayIconMouseDown');
    expect(calls, isEmpty);
    now = now.add(const Duration(milliseconds: 700));
    await nativeSays(tray, 'onTrayIconMouseDown');
    expect(calls, isEmpty, reason: 'too far apart');
    now = now.add(const Duration(milliseconds: 300));
    await nativeSays(tray, 'onTrayIconMouseDown');
    expect(calls, ['window.show', 'window.restore', 'window.focus']);
    await shell.dispose();
  });

  test('the menu opens the chat, and Hosting opens the window first', () async {
    var hosting = 0;
    final shell = await started(onHosting: () => hosting++);
    final menu = shell.menu!.items!.map((item) => item.key).toList();
    expect(menu, ['open', 'hosting', 'settings', 'hide', null, 'quit']);
    final hostingId = shell.menu!.items![1].id;
    await nativeSays(tray, 'onTrayMenuItemClick', {'id': hostingId});
    expect(calls, ['window.show', 'window.restore', 'window.focus']);
    expect(hosting, 1);
    await shell.dispose();
  });

  test('quitting ends what the app started before the window goes, even when that fails', () async {
    final shell = await started();
    final order = <String>[];
    shell.beforeQuit = () async {
      order.add('ending');
      await Future<void>.delayed(const Duration(milliseconds: 10));
      order.add('ended');
      throw StateError('an agent would not end');
    };
    await expectLater(shell.quit(), throwsStateError);
    expect(order, ['ending', 'ended']);
    expect(calls.last, 'window.destroy');
    await nativeSays(window, 'onEvent', {'eventName': 'close'});
    expect(calls.where((call) => call == 'window.hide'), isEmpty, reason: 'a quitting window is not hidden');
  });
}
