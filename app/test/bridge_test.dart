// Canoa's chat reaching agents a real `canoa serve --program` serves under each method, through the app's own connection code: the
// PIN pairing and peer proof the web build runs, its sealed channel and VerifiedAgent. The browser's side of the proof is
// canoa-host's web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa, whose stdio_agent example (in examples/ beside it) is the agent served. Without it the
// tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:coqui_transport_web/coqui_transport_web.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/web_peer_bridge.dart';

final String? _host = Platform.environment['CANOA_TEST_HOST'];

String get _agent {
  final examples = File(_host!).absolute.parent.uri.resolve('examples/').toFilePath();
  return '$examples${Platform.isWindows ? 'stdio_agent.exe' : 'stdio_agent'}';
}

Future<int> _freePort() async {
  final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final port = socket.port;
  await socket.close();
  return port;
}

/// Waits until something accepts connections on [port].
Future<void> _listening(int port) async {
  final deadline = DateTime.now().add(const Duration(seconds: 20));
  for (;;) {
    try {
      (await Socket.connect(InternetAddress.loopbackIPv4, port)).destroy();
      return;
    } on SocketException {
      if (DateTime.now().isAfter(deadline)) rethrow;
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
  }
}

void main() {
  final skip = _host == null ? 'CANOA_TEST_HOST names a built canoa' : null;
  late Directory dir;
  late WebPeerBridge browser;
  final started = <Process>[];

  // The host reads no canoa.cfg of the person's who runs the test: it is given an empty one of its own.
  Map<String, String> home() {
    final config = File('${dir.path}/empty.cfg');
    if (!config.existsSync()) config.writeAsStringSync('');
    return {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home', 'CANOA_CONFIG': config.path};
  }

  Future<Process> run(List<String> args) async {
    final process = await Process.start(_host!, args, environment: home());
    started.add(process);
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    return process;
  }

  /// `canoa pair --method noise`: its address, the PIN it shows, its id, and the process.
  Future<(String, String, String, Process)> pairAccept() async {
    final port = await _freePort();
    final process = await run(['pair', '--listen', '127.0.0.1:$port', '--method', 'noise', '--timeout-seconds', '60']);
    // It shows the PIN once it listens, as one JSON line. A probe connection would count as a failed attempt.
    final first = Completer<String>();
    process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      if (!first.isCompleted) first.complete(line);
    });
    final line = await first.future.timeout(const Duration(seconds: 30));
    final shown = jsonDecode(line) as Map<String, dynamic>;
    return ('ws://127.0.0.1:$port', shown['pin'] as String, shown['peer'] as String, process);
  }

  /// `serve --program` on a free port with [args], answering its URL.
  Future<String> serve(List<String> args) async {
    final port = await _freePort();
    await run(['serve', '--program', ...args, '--listen', '127.0.0.1:$port', '--', _agent]);
    await _listening(port);
    return 'ws://127.0.0.1:$port';
  }

  /// The app's own pairing: the channel it dials, then the PIN exchange the page runs.
  Future<String> pair(String url, String pin) async {
    final source = await connectWebChannel(chatEndpoints(webSocket: url, preferWebTransport: false));
    return pairByPin(browser, StagedChannel(source), pin);
  }

  Future<VerifiedAgent> connect(String url, String peer, PeerMethod method) => connectVerifiedAcp(
    chatEndpoints(webSocket: url),
    expectedPeerId: peer,
    verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
  );

  Future<String> turn(VerifiedAgent client, String text) async {
    final opened = await client.openSession(cwd: dir.path);
    final written = StringBuffer();
    final listening = client.updates.listen((update) {
      if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
    });
    await client.prompt(opened.id, text);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    await listening.cancel();
    return written.toString();
  }

  setUp(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-bridge-test');
    browser = await WebPeerBridge.start();
  });

  tearDown(() async {
    if (skip != null) return;
    browser.kill();
    for (final process in started) {
      process.kill();
      await process.exitCode;
    }
    started.clear();
    await dir.delete(recursive: true);
  });

  test('the app pairs by PIN with a Noise bridge, and its chat reaches the agent that bridge serves', () async {
    final (pairUrl, pin, bridgePeer, pairing) = await pairAccept();
    expect(await pair(pairUrl, pin), bridgePeer, reason: 'the id the pairing proved is the bridge\'s');
    expect(await pairing.exitCode.timeout(const Duration(seconds: 10)), 0);
    final url = await serve(['--paired', '--method', 'noise']);
    final client = await connect(url, bridgePeer, PeerMethod.noise);
    try {
      expect(await turn(client, 'hello'), 'echo: hello');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('a wrong PIN pairs nobody, says it was refused, and the bridge serves nothing to the key', () async {
    final (pairUrl, pin, bridgePeer, pairing) = await pairAccept();
    final wrong = pin == '000000' ? '000001' : '000000';
    await expectLater(pair(pairUrl, wrong), throwsA(anything));
    expect(await pairing.exitCode.timeout(const Duration(seconds: 10)), isNot(0));
    final url = await serve(['--paired', '--method', 'noise']);
    await expectLater(connect(url, bridgePeer, PeerMethod.noise), throwsA(anything));
  }, skip: skip);

  test('the chat reaches a WebCrypto bridge that allows the browser\'s WebCrypto key, which is not its Noise key', () async {
    final keygen = await Process.run(_host!, ['id', '--identity', '${dir.path}/webcrypto.pem', '--method', 'webcrypto'], environment: home());
    expect(keygen.exitCode, 0, reason: '${keygen.stderr}');
    final bridgePeer = (keygen.stdout as String).trim();
    final own = await browser.localId(PeerMethod.webcrypto);
    expect(own, isNot(await browser.localId(PeerMethod.noise)));
    final url = await serve(['--identity', '${dir.path}/webcrypto.pem', '--allow', own]);
    final client = await connect(url, bridgePeer, PeerMethod.webcrypto);
    try {
      expect(await turn(client, 'hello'), 'echo: hello');
    } finally {
      await client.close();
    }
    await expectLater(connect(url, bridgePeer, PeerMethod.noise), throwsA(anything),
        reason: 'a Noise proof of a WebCrypto agent fails, and nothing tries the other method');
  }, skip: skip);

  test('an agent stored under the wrong method is refused, and the other method is never tried', () async {
    final (pairUrl, pin, bridgePeer, _) = await pairAccept();
    await pair(pairUrl, pin);
    final url = await serve(['--paired', '--method', 'noise']);
    await expectLater(connect(url, bridgePeer, PeerMethod.webcrypto), throwsA(anything));
  }, skip: skip);
}
