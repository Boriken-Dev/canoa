// The built app hosting agents from the library it carries, with no `canoa` anywhere: its own window and tray through their
// native plugins, the scripted agent as the host's child, a key served while the window is closed, and what the app does on
// Quit ending every agent.
//
//   flutter test integration_test/embedded_host_test.dart -d windows   (or -d linux)
//
// `node` on PATH runs the scripted agent; CANOA_TEST_NODE names Node 22 or later for the browser's WebCrypto; CANOA_TEST_APP
// is this app's directory (default: the working directory). CANOA_HOST has to be unset, as it is for a person.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/child_flag.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';

import '../test/fixtures/node_peer_bridge.dart';

final String _appDir = (Platform.environment['CANOA_TEST_APP'] ?? Directory.current.path).replaceAll(r'\', '/');

/// The ids of the processes whose command line holds [marker], this test's own excepted.
Future<List<int>> _processes(String marker) async {
  final ran = Platform.isWindows
      ? await Process.run('powershell', [
          '-NoProfile',
          '-Command',
          "(Get-CimInstance Win32_Process | Where-Object { \$_.CommandLine -like '*$marker*' -and "
              "\$_.Name -notlike 'powershell*' }).ProcessId",
        ])
      : await Process.run('pgrep', ['-f', '--', marker]);
  return [
    for (final line in const LineSplitter().convert(ran.stdout as String))
      if (int.tryParse(line.trim()) case final int id)
        if (id != pid) id,
  ];
}

String _toml(String text) => jsonEncode(text);

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'with no canoa on disk the app hosts the scripted agent, through a closed window, and stops everything on Quit',
    (tester) async {
      expect(Platform.environment['CANOA_HOST'], isNull, reason: 'the route under test takes no program');

      // What the build put beside the executable.
      final executable = Platform.resolvedExecutable;
      final built = File(executable).parent;
      expect(File(CanoaNative.beside(executable)).existsSync(), isTrue, reason: 'the library is in the built output');
      for (final name in ['canoa', 'canoa.exe', 'canoa-host', 'canoa-host.exe']) {
        expect(File('${built.path}${Platform.pathSeparator}$name').existsSync(), isFalse, reason: name);
      }
      debugPrint('built output ${built.path}: ${[for (final file in built.listSync()) file.uri.pathSegments.lastWhere((part) => part.isNotEmpty)]}');

      // Started as its host's own command it answers without Flutter or a window, and takes nothing else.
      final refused = await Process.run(executable, [canoaChildFlag, 'peers']);
      expect(refused.exitCode, 2, reason: '${refused.stderr}');

      final dir = await Directory.systemTemp.createTemp('canoa-embedded');
      final marker = 'canoa-embedded-marker-${DateTime.now().microsecondsSinceEpoch}';
      final script = '$_appDir/integration_test/scripted_agent.mjs';
      final state = '${dir.path}${Platform.pathSeparator}$marker.json';
      final cfg = File('${dir.path}${Platform.pathSeparator}canoa.cfg');
      await cfg.writeAsString('''
[host.agents.scripted]
program = "node"
args = [${_toml(script)}, ${_toml('--state=$state')}]
listen = "127.0.0.1:0"

[host.agents.profiled]
program = "node"
args = [${_toml(script)}, ${_toml('--state=$state')}]
listen = "127.0.0.1:0"
profiles = ["plain"]

[host.profiles.plain]
context = { text = "A profile, so that this entry starts through the host's own launch command." }
''');

      expect(findHostTool, returnsNormally);
      expect(findHostTool(), isA<LibraryHostTool>(), reason: 'nothing but the library is looked for');
      final tool = LibraryHostTool(CanoaNative.open(), home: '${dir.path}/home');
      final host = HostSupervisor(tool);
      final hosting = ValueNotifier<int>(0);
      final shell = TrayShell();
      await shell.initialize(const CanoaConfig(), onSettings: () {}, onHosting: () => hosting.value++);
      final browser = await NodePeerBridge.start(script: '$_appDir/test/fixtures/node_peer.mjs');
      addTearDown(() async {
        browser.kill();
        await host.stop();
        await shell.dispose();
        try {
          await dir.delete(recursive: true);
        } on FileSystemException {
          // Windows may hold a file a moment longer.
        }
      });

      Future<void> until(Future<bool> Function() done, String what) async {
        final deadline = DateTime.now().add(const Duration(seconds: 30));
        while (!await done()) {
          if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what: ${host.log}');
          await tester.pump(const Duration(milliseconds: 100));
        }
      }

      await tester.pumpWidget(
        CanoaApp(
          store: SessionStore(includePreview: false),
          config: const CanoaConfig(),
          shell: shell,
          host: host,
          hostingSignal: hosting,
        ),
      );
      await host.apply(configPath: cfg.path);
      await until(() async => host.phase == HostPhase.running, 'the host to run');
      expect(host.pid, pid, reason: 'the host runs in this process');
      expect(host.listening.map((entry) => entry.name), ['scripted', 'profiled']);
      debugPrint('platform ${Platform.operatingSystem}: tray shown ${shell.hasTray}, host in process $pid');

      // Connections a turn leaves open, so that their agents stay running.
      final held = <VerifiedAgent>[];

      Future<String> turn(String name, {bool hold = false}) async {
        final entry = host.listening.singleWhere((listening) => listening.name == name);
        final method = PeerMethod.parse(entry.method!);
        final client = await connectVerifiedAcp(
          chatEndpoints(webSocket: entry.url),
          expectedPeerId: host.peerId!,
          verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
        );
        try {
          final opened = await client.openSession(cwd: dir.path);
          final written = StringBuffer();
          final listening = client.updates.listen((update) => written.write(update.text ?? ''));
          await client.prompt(opened.id, 'Say hello from $name');
          await Future<void>.delayed(const Duration(milliseconds: 100));
          await listening.cancel();
          if (hold) {
            held.add(client);
            return written.toString();
          }
          await client.close();
          return written.toString();
        } on Object {
          await client.close();
          rethrow;
        }
      }

      // Turned away until the person pairs it, through the library.
      await expectLater(turn('scripted'), throwsA(anything));
      final method = PeerMethod.parse(host.listening.first.method!);
      await host.accept(await browser.localId(method));
      expect(await turn('scripted'), 'hello from scripted');

      // Closed to the tray, where there is one: the host goes on.
      await windowManager.close();
      if (shell.hasTray) {
        await until(() async => !await windowManager.isVisible(), 'the window to go to the tray');
      } else {
        await tester.pump(const Duration(seconds: 1));
      }
      expect(host.phase, HostPhase.running);
      expect(
        await turn('profiled', hold: true),
        'hello from profiled',
        reason: 'an entry with a profile starts through this executable, as its own launch command',
      );
      expect(await turn('scripted', hold: true), 'hello from scripted');

      final agents = await _processes(marker);
      expect(agents, isNotEmpty, reason: 'the agents are this host\'s children');
      final launchers = await _processes(canoaChildFlag);
      expect(launchers, isNotEmpty, reason: 'the profile went through this executable');
      debugPrint('window closed: agents $agents, launchers $launchers');

      // A second host on this home, in this process: it says who holds it and does not start.
      final second = HostSupervisor(tool);
      await second.apply(configPath: cfg.path);
      await until(() async => second.phase == HostPhase.held, 'the second host to refuse');
      expect(second.problem, contains('CanoaUI (process $pid)'));
      expect(host.phase, HostPhase.running, reason: 'the first goes on');

      // What Exit Canoa runs, up to destroying the window, which would end this test's process too.
      final log = host.log;
      await shell.beforeQuit!();
      expect(host.phase, HostPhase.stopped);
      expect(host.listening, isEmpty);
      for (final client in held) {
        await client.close();
      }
      await until(() async => (await _processes(marker)).isEmpty, 'every agent to end');
      await until(() async => (await _processes(canoaChildFlag)).isEmpty, 'every launcher to end');
      expect(log, contains('proven'), reason: 'the app reads the host\'s log');
      debugPrint('the host log, as the app read it:\n$log');

      // Everything was given up: the lock, and so the second can start when told.
      await second.restart();
      await until(() async => second.phase == HostPhase.running, 'the second host to start once told');
      await second.stop();
    },
  );
}
