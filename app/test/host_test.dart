// Canoa's chat reaching the agents `canoa-host` serves, through the app's own connection code: connectVerifiedAcp, the
// peer proof the web build runs, its sealed channel and VerifiedAgent, against the real host binary. The browser's WebCrypto
// is web/canoa_peer_bridge.mjs run by Node, since the Dart VM has none.
//
// CANOA_TEST_HOST names a built canoa-host, with its examples (stdio_agent, echo_provider) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa-host --examples` leaves them. CANOA_TEST_NODE names Node 22 or later (default
// `node`). Without CANOA_TEST_HOST the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:flutter_test/flutter_test.dart';

final String? _hostBinary = Platform.environment['CANOA_TEST_HOST'];
final String _node = Platform.environment['CANOA_TEST_NODE'] ?? 'node';

/// The page's peer bridge in a Node process: one key for the life of the process.
final class NodePeerBridge implements PeerBridge {
  NodePeerBridge._(this._process) {
    _process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final answer = jsonDecode(line) as Map<String, dynamic>;
      final waiting = _waiting.remove(answer['id']);
      if (waiting == null) return;
      if (answer.containsKey('error')) {
        waiting.completeError(StateError(answer['error'] as String));
      } else {
        waiting.complete(answer['result']);
      }
    });
    _process.stderr.transform(utf8.decoder).listen(stderr.write);
  }

  static Future<NodePeerBridge> start() async =>
      NodePeerBridge._(await Process.start(_node, ['test/fixtures/node_peer.mjs']));

  final Process _process;
  final _waiting = <int, Completer<Object?>>{};
  var _next = 0;

  Future<Object?> _call(String op, List<Object?> args) {
    final id = _next++;
    final answer = Completer<Object?>();
    _waiting[id] = answer;
    _process.stdin.writeln(jsonEncode({'id': id, 'op': op, 'args': args}));
    return answer.future;
  }

  Uint8List _bytes(Object? text) => base64Decode(text as String);

  Future<String> localId() async => await _call('localId', []) as String;
  @override
  Future<String> begin(String expectedPeerId) async => await _call('begin', [expectedPeerId]) as String;
  @override
  Future<String> answer(String token, String challenge) async => await _call('answer', [token, challenge]) as String;
  @override
  Future<String> finish(String token, String accepted) async => await _call('finish', [token, accepted]) as String;
  @override
  void abort(String token) => unawaited(_call('abort', [token]));
  @override
  Future<Uint8List> seal(String session, Uint8List message) async =>
      _bytes(await _call('seal', [session, base64Encode(message)]));
  @override
  Future<Uint8List?> open(String session, Uint8List record) async {
    final opened = await _call('open', [session, base64Encode(record)]);
    return opened == null ? null : _bytes(opened);
  }

  @override
  Future<Uint8List> closeRecord(String session) async => _bytes(await _call('closeRecord', [session]));
  @override
  void closeSession(String session) => unawaited(_call('closeSession', [session]));

  void kill() => _process.kill();
}

/// `canoa-host serve` on a Canoa.cfg in a directory of its own.
final class HostUnderTest {
  HostUnderTest._(this.process, this.dir, this.urls, this.peer);

  final Process process;
  final Directory dir;
  final Map<String, String> urls;
  final String peer;

  static String _example(String name, {required bool library}) {
    final examples = File(_hostBinary!).parent.uri.resolve('examples/').toFilePath();
    final file = library
        ? (Platform.isWindows ? '$name.dll' : Platform.isMacOS ? 'lib$name.dylib' : 'lib$name.so')
        : (Platform.isWindows ? '$name.exe' : name);
    return '$examples$file';
  }

  static Future<HostUnderTest> start() async {
    final dir = await Directory.systemTemp.createTemp('canoa-host-app-test');
    final config = File('${dir.path}/Canoa.cfg');
    await config.writeAsString('''
[appearance]
look = "bubbles"

[host.agents.fixture]
program = ${jsonEncode(_example('stdio_agent', library: false))}

[host.providers.echo]
library = ${jsonEncode(_example('echo_provider', library: true))}
provider = "echo_provider"
''');
    final process = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path],
      environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
    );
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final lines = process.stdout.transform(utf8.decoder).transform(const LineSplitter());
    final urls = <String, String>{};
    String? peer;
    await for (final line in lines.take(2).timeout(const Duration(seconds: 30))) {
      final entry = jsonDecode(line) as Map<String, dynamic>;
      urls[entry['name'] as String] = entry['url'] as String;
      peer = entry['peer'] as String;
    }
    return HostUnderTest._(process, dir, urls, peer!);
  }

  Future<ProcessResult> command(List<String> args) => Process.run(
    _hostBinary!,
    args,
    environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
  );

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
    await dir.delete(recursive: true);
  }
}

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa-host' : null;
  late HostUnderTest host;
  late NodePeerBridge browser;

  setUpAll(() async {
    if (skip != null) return;
    host = await HostUnderTest.start();
    browser = await NodePeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    await host.stop();
  });

  Future<VerifiedAgent> connect(String entry, {AcpDecider? decider}) => connectVerifiedAcp(
    chatEndpoints(webSocket: host.urls[entry]!),
    expectedPeerId: host.peer,
    verifyPeer: (channel, expected) => provePeer(browser, channel, expected),
    decider: decider,
  );

  /// What the agent wrote during one turn.
  Future<String> turn(VerifiedAgent client, String session, String text) async {
    final written = StringBuffer();
    final listening = client.updates.listen((update) {
      if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
    });
    await client.prompt(session, text);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    await listening.cancel();
    return written.toString();
  }

  test('a browser key is turned away until the person accepts it on the host', () async {
    await expectLater(connect('fixture'), throwsA(anything));
    final browserId = await browser.localId();
    final listed = await host.command(['peers', '--format', 'json']);
    expect(listed.exitCode, 0);
    final waiting = const LineSplitter()
        .convert(listed.stdout as String)
        .map((line) => jsonDecode(line) as Map<String, dynamic>)
        .where((peer) => peer['state'] == 'waiting')
        .map((peer) => peer['peer']);
    expect(waiting, contains(browserId));
    final accepted = await host.command(['accept', browserId]);
    expect(accepted.exitCode, 0, reason: accepted.stderr as String);
  }, skip: skip);

  test('the chat reaches a stdio ACP agent, and a permission it asks reaches the person', () async {
    final asked = <AcpPermissionRequest>[];
    final client = await connect('fixture', decider: (request) async {
      asked.add(request);
      return const AcpAllow();
    });
    try {
      final opened = await client.openSession(cwd: host.dir.path);
      expect(await turn(client, opened.id, 'hello'), 'echo: hello');
      expect(await turn(client, opened.id, 'ask to write'), 'the person chose allow');
      expect(asked.single.title, 'Write a file');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('the chat reaches a Guatiao provider', () async {
    final client = await connect('echo');
    try {
      final opened = await client.openSession(cwd: host.dir.path);
      expect(await turn(client, opened.id, 'hello'), 'echo: hello');
      expect(await turn(client, opened.id, 'again'), 'echo: again');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('a key the host removes loses its link and cannot come back', () async {
    final client = await connect('echo');
    final removed = await host.command(['revoke', await browser.localId()]);
    expect(removed.exitCode, 0);
    try {
      final deadline = DateTime.now().add(const Duration(seconds: 5));
      while (!client.isClosed && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      expect(client.isClosed, isTrue, reason: 'the host ends the link of a removed key');
      await expectLater(client.openSession(cwd: host.dir.path), throwsA(anything));
    } finally {
      await client.close();
    }
    await expectLater(connect('echo'), throwsA(anything));
  }, skip: skip);
}
