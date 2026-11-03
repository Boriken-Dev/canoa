// The desktop app on a real desktop: its own window and tray through their native plugins, canoa-host running as its child,
// and a browser's key reaching a hosted agent while the window is closed.
//
//   flutter test integration_test/desktop_test.dart -d windows   (or -d linux)
//
// CANOA_TEST_HOST names a built canoa-host, with its examples beside it as `cargo build -p canoa-host --bin canoa-host
// --examples` leaves them; CANOA_TEST_NODE names Node 22 or later, for the browser's WebCrypto; CANOA_TEST_APP is this app's
// directory, where the Node fixture is (default: the working directory). Without CANOA_TEST_HOST the test is skipped.
import 'dart:io';

import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';

import '../test/fixtures/node_peer_bridge.dart';

final String? _hostBinary = Platform.environment['CANOA_TEST_HOST'];
final String _appDir = Platform.environment['CANOA_TEST_APP'] ?? Directory.current.path;

/// `gdbus call` on the session bus, its output.
Future<String> _gdbus(List<String> args) async {
  final ran = await Process.run('gdbus', ['call', '--session', ...args]);
  if (ran.exitCode != 0) throw StateError('gdbus ${args.join(' ')}: ${ran.stderr}');
  return ran.stdout as String;
}

/// Picks [label] in this app's tray menu as a StatusNotifier host does on a click: the item the watcher lists, its
/// dbusmenu, and that entry's `clicked` event.
Future<void> pickFromTrayMenu(String label) async {
  final listed = await _gdbus([
    '--dest',
    'org.kde.StatusNotifierWatcher',
    '--object-path',
    '/StatusNotifierWatcher',
    '--method',
    'org.freedesktop.DBus.Properties.Get',
    'org.kde.StatusNotifierWatcher',
    'RegisteredStatusNotifierItems',
  ]);
  final item = RegExp(r"'(:[0-9.]+)(/[^']+)'").allMatches(listed).last;
  final menu = RegExp(r"objectpath '([^']+)'")
      .firstMatch(
        await _gdbus([
          '--dest',
          item.group(1)!,
          '--object-path',
          item.group(2)!,
          '--method',
          'org.freedesktop.DBus.Properties.Get',
          'org.kde.StatusNotifierItem',
          'Menu',
        ]),
      )!
      .group(1)!;
  final layout = await _gdbus([
    '--dest',
    item.group(1)!,
    '--object-path',
    menu,
    '--method',
    'com.canonical.dbusmenu.GetLayout',
    // GOption would read -1 as an option.
    '--',
    '0',
    '-1',
    '[]',
  ]);
  final id = RegExp("\\((\\d+), \\{[^}]*'label': <'${RegExp.escape(label)}'>").firstMatch(layout)?.group(1);
  if (id == null) throw StateError('no $label in the tray menu: $layout');
  await _gdbus([
    '--dest',
    item.group(1)!,
    '--object-path',
    menu,
    '--method',
    'com.canonical.dbusmenu.Event',
    id,
    'clicked',
    '<0>',
    '0',
  ]);
}

String _example(String name) {
  final examples = File(_hostBinary!).absolute.parent.uri.resolve('examples/').toFilePath();
  return '$examples${Platform.isWindows ? '$name.exe' : name}';
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'closed to the tray, the host keeps serving a browser; the app opens again and shows what it hosts',
    (tester) async {
      final dir = await Directory.systemTemp.createTemp('canoa-desktop');
      final cfg = File('${dir.path}${Platform.pathSeparator}Canoa.cfg');
      final config = const CanoaConfig().withHostAgent(
        name: 'fixture',
        program: _example('stdio_agent'),
        listen: '127.0.0.1:0',
      );
      await cfg.writeAsString(config.encode());
      final host = HostSupervisor(
        ProcessHostTool(_hostBinary!, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'}),
      );
      final hosting = ValueNotifier<int>(0);
      final shell = TrayShell();
      await shell.initialize(config, onSettings: () {}, onHosting: () => hosting.value++);
      final browser = await NodePeerBridge.start(script: '$_appDir/test/fixtures/node_peer.mjs');
      addTearDown(() async {
        browser.kill();
        await host.stop();
        await shell.dispose();
        await dir.delete(recursive: true);
      });

      Future<void> until(Future<bool> Function() done, String what) async {
        final deadline = DateTime.now().add(const Duration(seconds: 30));
        while (!await done()) {
          if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
          await tester.pump(const Duration(milliseconds: 100));
        }
      }

      await tester.pumpWidget(
        CanoaApp(
          store: SessionStore(includePreview: false),
          config: config,
          shell: shell,
          host: host,
          hostingSignal: hosting,
        ),
      );
      await host.apply(configPath: cfg.path, hasEntries: true);
      await until(() async => host.phase == HostPhase.running, 'the host to run');
      final hostPid = host.pid;
      debugPrint('platform ${Platform.operatingSystem}: tray shown ${shell.hasTray}, host pid $hostPid');
      expect(await windowManager.isVisible(), isTrue);

      // The window's own close, as its title bar's button sends it.
      await windowManager.close();
      if (shell.hasTray) {
        await until(() async => !await windowManager.isVisible(), 'the window to go to the tray');
      } else {
        // Minimized, which not every desktop reports: what matters is that it is not hidden where nothing brings it back.
        await tester.pump(const Duration(seconds: 1));
        expect(await windowManager.isVisible(), isTrue, reason: 'with no tray, closing does not hide the window');
      }
      expect(host.phase, HostPhase.running);
      expect(host.pid, hostPid, reason: 'closing the window does not touch the host');

      // A browser's key, with the window closed: turned away, paired by the person, then served.
      final url = host.listening.single.url;
      Future<String> browserTurn() async {
        final client = await connectVerifiedAcp(
          chatEndpoints(webSocket: url),
          expectedPeerId: host.peerId!,
          verifyPeer: (channel, expected) => provePeer(browser, channel, expected),
        );
        try {
          final opened = await client.openSession(cwd: dir.path);
          final written = StringBuffer();
          final listening = client.updates.listen((update) => written.write(update.text ?? ''));
          await client.prompt(opened.id, 'hello');
          await Future<void>.delayed(const Duration(milliseconds: 100));
          await listening.cancel();
          return written.toString();
        } finally {
          await client.close();
        }
      }

      await expectLater(browserTurn(), throwsA(anything));
      final browserId = await browser.localId();
      await host.accept(browserId);
      expect(await browserTurn(), 'echo: hello');

      // Back: the tray menu's Open on Linux, picked over the StatusNotifier protocol as a panel does; a double click on the
      // icon elsewhere; the taskbar where there is no tray.
      if (shell.hasTray && Platform.isLinux) {
        await pickFromTrayMenu('Open Canoa');
      } else if (shell.hasTray) {
        shell.onTrayIconMouseDown();
        shell.onTrayIconMouseDown();
      } else {
        await shell.open();
      }
      await until(() async => await windowManager.isVisible() && !await windowManager.isMinimized(), 'the window');

      await tester.tap(find.byTooltip('Hosting'));
      await tester.pumpAndSettle();
      expect(find.text('Hosting, on this computer only.'), findsOneWidget);
      expect(find.text('fixture'), findsOneWidget);
      expect(find.textContaining(url), findsOneWidget);
      expect(find.text(browserId), findsOneWidget, reason: 'the paired key, listed');
      expect(find.text(host.peerId!), findsOneWidget, reason: 'the id a browser pins');
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();

      await host.stop();
      expect(Process.killPid(hostPid!), isFalse, reason: 'stopping ended the host');
      expect(host.listening, isEmpty);
    },
    skip: _hostBinary == null,
  );
}
