// main.dart's own main, as a person's start runs it, in the built app with nothing but its library: one step per run, each
// on a canoa.cfg and a home of its own, which the environment names since main.dart reads them there.
//
//   CANOA_TEST_LAUNCH_STEP=STEP CANOA_CONFIG=DIR/canoa.cfg COQUI_AGENTIC_PEER_HOME=DIR/home \
//     flutter test integration_test/launch_test.dart -d windows   (or -d linux)
//
// STEP is `first` (no canoa.cfg at all), `flag` (`--no-local-host`), `config` (`[desktop] local_host = false`) or `hosted`
// (an agent served, through a closed window). DIR is a new empty directory. `node` on PATH runs the scripted agent;
// CANOA_TEST_NODE names Node 22 or later for the browser's WebCrypto; CANOA_TEST_APP is this app's directory (default: the
// working directory). CANOA_HOST has to be unset, as it is for a person. It spends nothing.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/main.dart' as app;
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';

import '../test/fixtures/node_peer_bridge.dart';

final String _appDir = (Platform.environment['CANOA_TEST_APP'] ?? Directory.current.path).replaceAll(r'\', '/');

Future<bool> _answers(int port) async {
  try {
    final socket = await Socket.connect(InternetAddress.loopbackIPv4, port);
    socket.destroy();
    return true;
  } on SocketException {
    return false;
  }
}

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

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final environment = Platform.environment;
  final step = environment['CANOA_TEST_LAUNCH_STEP'];
  final cfg = environment['CANOA_CONFIG'];
  final home = environment['COQUI_AGENTIC_PEER_HOME'];
  final skip = step == null || cfg == null || home == null
      ? 'CANOA_TEST_LAUNCH_STEP, CANOA_CONFIG and COQUI_AGENTIC_PEER_HOME name a step and its files'
      : null;

  testWidgets('a person starting the app: step $step', skip: skip != null, (tester) async {
    expect(environment['CANOA_HOST'], isNull, reason: 'the route under test takes no program');
    final built = File(Platform.resolvedExecutable).parent;
    for (final name in ['canoa', 'canoa.exe', 'canoa-host', 'canoa-host.exe']) {
      expect(File('${built.path}${Platform.pathSeparator}$name').existsSync(), isFalse, reason: name);
    }
    final dir = File(cfg!).parent.path;
    final holderFile = File('$home/host.holder');
    Map<String, Object?>? holder() {
      try {
        return jsonDecode(holderFile.readAsStringSync()) as Map<String, Object?>;
      } on Object {
        return null;
      }
    }

    bool hostsItself() => holder()?['pid'] == pid && holder()?['name'] == 'CanoaUI';

    Future<void> until(Future<bool> Function() done, String what) async {
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!await done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    final (port, launchedPort) = (await freeLoopbackPort(), await freeLoopbackPort());
    final marker = 'canoa-launch-marker-${DateTime.now().microsecondsSinceEpoch}';
    final script = jsonEncode('$_appDir/integration_test/scripted_agent.mjs');
    final state = jsonEncode('--state=$dir/$marker.json');
    final entries = '''
[host.agents.scripted]
program = "node"
args = [$script, $state]
listen = "127.0.0.1:$port"

[host.agents.profiled]
program = "node"
args = [$script, $state]
listen = "127.0.0.1:$launchedPort"
profiles = ["plain"]

[host.profiles.plain]
context = { text = "A profile, so that this entry starts through the host's own launch command." }
''';

    switch (step) {
      case 'first':
        expect(File(cfg).existsSync(), isFalse, reason: 'a first launch: no canoa.cfg yet');
        await app.main(const []);
        await until(() async => hostsItself(), 'the app to take its home');
        expect(File(cfg).existsSync(), isFalse, reason: 'starting writes no canoa.cfg');
        debugPrint('first launch: ${holder()}');

      case 'flag' || 'config':
        await File(cfg).writeAsString(step == 'config' ? '[desktop]\nlocal_host = false\n\n$entries' : entries);
        await app.main(step == 'flag' ? const ['--no-local-host'] : const []);
        await tester.pump(const Duration(seconds: 3));
        expect(holder(), isNull);
        expect(File('$home/host.lock').existsSync() || Directory(home!).existsSync(), isFalse,
            reason: 'its home was never taken');
        expect(await _answers(port), isFalse, reason: 'nothing listens');
        debugPrint('left off by $step: no holder, nothing on $port');

      case 'hosted':
        await File(cfg).writeAsString(entries);
        await app.main(const []);
        await until(() async => hostsItself(), 'the app to take its home');
        await until(() => _answers(port), 'the entry to listen');
        await until(() => _answers(launchedPort), 'the launched entry to listen');

        final browser = await NodePeerBridge.start(script: '$_appDir/test/fixtures/node_peer.mjs');
        addTearDown(browser.kill);
        // Keys are managed from outside the running host, as `canoa accept` is.
        final keys = LibraryHostTool(CanoaNative.open(), home: home);
        final id = await keys.run(['id']);
        expect(id.exitCode, 0, reason: id.stderr);
        final peer = id.stdout.trim();
        final method = PeerMethod.parse(File('$home/method').readAsStringSync().trim());
        Future<VerifiedAgent> connect(int at) => connectVerifiedAcp(
          chatEndpoints(webSocket: 'ws://127.0.0.1:$at'),
          expectedPeerId: peer,
          verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
        );

        // Turned away until the person pairs it, as anyone connecting first is.
        await expectLater(connect(port).then((client) => client.close()), throwsA(anything));
        final accepted = await keys.run(['accept', '--', await browser.localId(method)]);
        expect(accepted.exitCode, 0, reason: accepted.stderr);
        Future<String> turn(VerifiedAgent client) async {
          final opened = await client.openSession(cwd: dir);
          final written = StringBuffer();
          final listening = client.updates.listen((update) => written.write(update.text ?? ''));
          await client.prompt(opened.id, 'Say hello from the launch test');
          await Future<void>.delayed(const Duration(milliseconds: 100));
          await listening.cancel();
          return written.toString();
        }

        final first = await connect(port);
        expect(await turn(first), 'hello from the launch test');
        await first.close();

        // Closed as a person closes it: to the tray, or minimized where there is none. The host goes on.
        await windowManager.close();
        await tester.pump(const Duration(seconds: 1));
        expect(hostsItself(), isTrue);
        final direct = await connect(port);
        final launched = await connect(launchedPort);
        expect(await turn(direct), 'hello from the launch test');
        expect(await turn(launched), 'hello from the launch test', reason: 'through this executable as `launch`');
        final agents = await _processes(marker);
        debugPrint('with the window closed: agents and the launcher of one $agents, visible ${await windowManager.isVisible()}');
        expect(agents, hasLength(greaterThanOrEqualTo(3)));
        await direct.close();
        await launched.close();
        final log = environment['CANOA_HOST_LOG'];
        if (log != null) debugPrint('the host log the app wrote:\n${File(log).readAsStringSync()}');

      default:
        fail('no step $step');
    }
    // The app's own frames, such as a scrollbar's, finish before the test takes its widgets away.
    await tester.pump(const Duration(seconds: 1));
  });
}
