// Canoa's chat reaching the agents `canoa-host` serves, through the app's own connection code: connectVerifiedAcp, the
// peer proof the web build runs, its sealed channel and VerifiedAgent, against the real host binary. The browser's WebCrypto
// is web/canoa_peer_bridge.mjs run by Node, since the Dart VM has none.
//
// CANOA_TEST_HOST names a built canoa-host, with its examples (stdio_agent, echo_provider) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa-host --examples` leaves them. CANOA_TEST_NODE names Node 22 or later (default
// `node`). Without CANOA_TEST_HOST the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/node_peer_bridge.dart';
import 'fixtures/webcrypto_host.dart';

// Absolute, since the provider's library is named in a Canoa.cfg written to a temporary directory and read from there.
final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path => File(path).absolute.path,
  null => null,
};

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
    // The app's browser proof is WebCrypto only, and a new home would prove by Noise.
    final process = await Process.start(
      _hostBinary!,
      ['serve', '--method', 'webcrypto', '--config', config.path],
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

/// Whether process [pid] runs, asked without signalling it: on Windows any signal ends a process.
Future<bool> _running(int pid) async {
  if (Platform.isWindows) {
    final listed = await Process.run('tasklist', ['/FI', 'PID eq $pid', '/NH', '/FO', 'CSV']);
    return (listed.stdout as String).contains('"$pid"');
  }
  return (await Process.run('kill', ['-0', '$pid'])).exitCode == 0;
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

  group('the app running the host', () {
    late Directory dir;
    late HostSupervisor supervisor;
    late File config;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-supervised');
      config = File('${dir.path}/Canoa.cfg');
      await config.writeAsString('''
[host.agents.fixture]
program = ${jsonEncode(HostUnderTest._example('stdio_agent', library: false))}
''');
      // The app's browser proof is WebCrypto only, and a new home would prove by Noise.
      supervisor = HostSupervisor(
        WebCryptoHostTool(ProcessHostTool(_hostBinary!, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'})),
        firstPause: const Duration(milliseconds: 200),
      );
    });

    tearDown(() async {
      await supervisor.stop();
      supervisor.dispose();
      await dir.delete(recursive: true);
    });

    Future<void> until(bool Function() done, String what) async {
      final deadline = DateTime.now().add(const Duration(seconds: 20));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }

    test('serves Canoa.cfg, and a key the person pairs in the app reaches its agent', () async {
      await supervisor.apply(configPath: config.path, hasEntries: true);
      await until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final url = supervisor.listening.single.url;
      final client = connectVerifiedAcp(
        chatEndpoints(webSocket: url),
        expectedPeerId: supervisor.peerId!,
        verifyPeer: (channel, expected) => provePeer(browser, channel, expected),
      );
      await expectLater(client, throwsA(anything), reason: 'an unpaired key is turned away');
      final browserId = await browser.localId();
      final waiting = await supervisor.keys();
      expect(waiting.where((key) => !key.accepted).map((key) => key.id), contains(browserId));
      await supervisor.accept(browserId);
      expect((await supervisor.keys()).firstWhere((key) => key.id == browserId).accepted, isTrue);
      final agent = await connectVerifiedAcp(
        chatEndpoints(webSocket: url),
        expectedPeerId: supervisor.peerId!,
        verifyPeer: (channel, expected) => provePeer(browser, channel, expected),
      );
      try {
        final opened = await agent.openSession(cwd: dir.path);
        expect(await turn(agent, opened.id, 'hello'), 'echo: hello');
      } finally {
        await agent.close();
      }
      await supervisor.revoke(browserId);
      expect((await supervisor.keys()).map((key) => key.id), isNot(contains(browserId)));
    }, skip: skip);

    test('a host that dies is started again, and stopping ends it for good', () async {
      await supervisor.apply(configPath: config.path, hasEntries: true);
      await until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final first = supervisor.pid!;
      Process.killPid(first, ProcessSignal.sigkill);
      await until(() => supervisor.phase == HostPhase.waiting, 'the supervisor to see it end');
      await until(() => supervisor.phase == HostPhase.running, 'the host to run again');
      final second = supervisor.pid!;
      expect(second, isNot(first));
      await supervisor.stop();
      expect(supervisor.phase, HostPhase.stopped);
      expect(supervisor.pid, isNull);
      await Future<void>.delayed(const Duration(seconds: 1));
      expect(supervisor.phase, HostPhase.stopped, reason: 'nothing starts it again');
      expect(await _running(second), isFalse, reason: 'the host has ended');
    }, skip: skip);

    test('a host whose app dies without stopping it ends too', () async {
      // CANOA_TEST_DART names the Dart that runs the dying app, `dart` by default.
      final ran = await Process.run(Platform.environment['CANOA_TEST_DART'] ?? 'dart', [
        'run',
        'test/fixtures/dying_parent.dart',
        _hostBinary!,
        config.path,
        '${dir.path}/home',
      ], runInShell: Platform.isWindows);
      expect(ran.exitCode, 0, reason: '${ran.stderr}');
      final pid = int.parse((ran.stdout as String).trim().split('\n').last.trim());
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (await _running(pid) && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
      final alive = await _running(pid);
      if (alive) Process.killPid(pid, ProcessSignal.sigkill);
      expect(alive, isFalse, reason: 'the host outlived the app that started it');
    }, skip: skip);

    test('a Canoa.cfg the host refuses is said in its words and not retried', () async {
      await config.writeAsString('[host.agents.fixture]\nprogram = "x"\nlisten = "0.0.0.0:7999"\n');
      expect(await supervisor.check(config.path), isNotNull);
      await supervisor.apply(configPath: config.path, hasEntries: true);
      await until(() => supervisor.phase == HostPhase.failed, 'the host to refuse it');
      expect(supervisor.problem, contains('loopback'));
    }, skip: skip);
  });

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
