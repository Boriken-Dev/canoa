// Canoa's chat reaching agents a real coqui-acp-bridge serves under each method, through the app's own connection code: the
// PIN pairing and peer proof the web build runs, its sealed channel and VerifiedAgent. The browser's WebCrypto is
// web/canoa_peer_bridge.mjs run by Node, since the Dart VM has none.
//
// CANOA_TEST_BRIDGE names a built coqui-acp-bridge; CANOA_TEST_HOST a built canoa, whose stdio_agent example (in
// examples/ beside it) is the agent served. CANOA_TEST_NODE names Node 22 or later (default `node`). Without both the tests
// are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/node_peer_bridge.dart';

final String? _bridge = Platform.environment['CANOA_TEST_BRIDGE'];
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
  final skip = _bridge == null || _host == null ? 'CANOA_TEST_BRIDGE and CANOA_TEST_HOST name built programs' : null;
  late Directory dir;
  late NodePeerBridge browser;
  final started = <Process>[];

  Map<String, String> home() => {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'};

  Future<Process> run(List<String> args) async {
    final process = await Process.start(_bridge!, args, environment: home());
    started.add(process);
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    return process;
  }

  /// `pair-accept --method noise`: its address, the PIN it shows, its id, and the process.
  Future<(String, String, String, Process)> pairAccept() async {
    final port = await _freePort();
    final process = await run(['pair-accept', '--listen', '127.0.0.1:$port', '--method', 'noise', '--timeout-seconds', '60']);
    final lines = process.stdout.transform(utf8.decoder).transform(const LineSplitter()).take(2);
    // It shows the PIN once it listens. A probe connection would count as a failed attempt.
    final [pin, peer] = await lines.toList().timeout(const Duration(seconds: 30));
    return ('ws://127.0.0.1:$port', pin.trim(), peer.trim(), process);
  }

  /// `serve` on a free port with [args], answering its URL.
  Future<String> serve(List<String> args) async {
    final port = await _freePort();
    await run(['serve', ...args, '--listen', '127.0.0.1:$port', '--', _agent]);
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
    browser = await NodePeerBridge.start();
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
    final keygen = await Process.run(_bridge!, ['keygen', '${dir.path}/webcrypto.pem', '--method', 'webcrypto'], environment: home());
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
