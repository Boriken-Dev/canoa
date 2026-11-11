// Canoa's chat reaching an agent `canoa` serves on a Windows named pipe or a Unix socket, through the app's own connection code
// (connectVerifiedAcpOverPipe: the pipe channel, the peer proof, VerifiedAgent), against the real host binary. The browser's
// side of the proof is the web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa with its examples (stdio_agent, web_peer) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/acp/pipe_channel.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/chat/cache_lifetime.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

/// `canoa serve` with a scripted agent on a pipe, and another with a working root, in a directory of its own.
final class PipedHost {
  PipedHost._(this.process, this.dir, this.listen, this.peer, this.method, this.url, this.treesUrl, this.claudeUrl);

  final Process process;
  final Directory dir;

  /// What canoa.cfg's `listen` said.
  final String listen;
  final String peer;
  final PeerMethod method;

  /// What the host printed as the entry's url.
  final String url;

  /// The url of the entry that has a working root.
  final String treesUrl;

  /// The url of a Claude Code entry served by a stand-in that records what it was started with.
  final String claudeUrl;

  static Future<PipedHost> start() async {
    final dir = await Directory.systemTemp.createTemp('canoa-pipe-test');
    // A pipe's name is global on Windows; a socket's path is under the test's directory on Unix.
    final unique = '$pid-${Random().nextInt(1 << 30)}';
    final listen = Platform.isWindows ? 'pipe:canoa-dart-test-$unique' : 'unix:${dir.path}/host.sock';
    final claudeListen = Platform.isWindows ? 'pipe:canoa-dart-claude-$unique' : 'unix:${dir.path}/claude.sock';
    final standin = File('${dir.path}/claude${Platform.isWindows ? '.exe' : ''}');
    await File(exampleBinary('harness_standin')!).copy(standin.path);
    final treesListen = Platform.isWindows ? 'pipe:canoa-dart-trees-$unique' : 'unix:${dir.path}/trees.sock';
    final config = File('${dir.path}/canoa.cfg');
    await config.writeAsString('''
[host.agents.scripted]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
args = ${jsonEncode(scriptedArgs('${dir.path}/state'))}
listen = ${jsonEncode(listen)}

[host.agents.trees]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
listen = ${jsonEncode(treesListen)}
working_roots = [${jsonEncode(File('${dir.path}/src').absolute.path)}]

[host.agents.Claude]
harness = "claude"
executable = ${jsonEncode(standin.path)}
listen = ${jsonEncode(claudeListen)}
''');
    final process = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path],
      environment: {
        'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home',
        'STANDIN_RECORD': '${dir.path}/record.jsonl',
        'STANDIN_ENV': 'CLAUDE_CODE_PROMPT_CACHE_TTL',
      },
    );
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final lines = process.stdout.transform(utf8.decoder).transform(const LineSplitter());
    final urls = <String, Map<String, dynamic>>{};
    await for (final line in lines.take(3).timeout(const Duration(seconds: 30))) {
      final entry = jsonDecode(line) as Map<String, dynamic>;
      urls[entry['name'] as String] = entry;
    }
    final scripted = urls['scripted']!;
    return PipedHost._(
      process,
      dir,
      listen,
      scripted['peer'] as String,
      PeerMethod.parse(scripted['method'] as String),
      scripted['url'] as String,
      urls['trees']!['url'] as String,
      urls['Claude']!['url'] as String,
    );
  }

  Future<ProcessResult> command(List<String> args) =>
      Process.run(_hostBinary!, args, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'});

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
    await dir.delete(recursive: true);
  }
}

void main() {
  final skip = _hostBinary == null
      ? 'CANOA_TEST_HOST names a built canoa'
      : exampleMissing('stdio_agent') ?? exampleMissing('web_peer');
  late PipedHost host;
  late WebPeerBridge browser;

  setUpAll(() async {
    if (skip != null) return;
    host = await PipedHost.start();
    browser = await WebPeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    await host.stop();
  });

  Future<VerifiedAgent> connect({String? expected, AgentDecider? decider}) => connectVerifiedAcpOverPipe(
    host.url,
    expectedPeerId: expected ?? host.peer,
    verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: host.method),
    decider: decider,
  );

  test('the host says the pipe it listens on, which is not an address a page can dial', () {
    expect(host.url, host.listen);
    expect(isPipeUrl(host.url), isTrue);
    expect(isPipeUrl('ws://127.0.0.1:7801'), isFalse);
  }, skip: skip);

  test('a key is turned away over the pipe until the person accepts it on the host, then runs a turn', () async {
    await expectLater(connect(), throwsA(anything));
    final browserId = await browser.localId(host.method);
    final listed = await host.command(['peers', '--format', 'json']);
    expect(listed.exitCode, 0);
    final waiting = const LineSplitter()
        .convert(listed.stdout as String)
        .map((line) => jsonDecode(line) as Map<String, dynamic>)
        .where((peer) => peer['state'] == 'waiting')
        .map((peer) => peer['peer']);
    expect(waiting, contains(browserId), reason: 'the refused key waits for `canoa accept`');
    final accepted = await host.command(['accept', browserId]);
    expect(accepted.exitCode, 0, reason: accepted.stderr as String);

    final asked = <AcpPermissionRequest>[];
    final client = await connect(decider: (request, _) async {
      asked.add(request);
      return const AcpAllow();
    });
    try {
      final opened = await client.openSession(cwd: host.dir.path);
      final written = StringBuffer();
      final listening = client.updates.listen((update) {
        if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
      });
      await client.prompt(opened.id, 'Please tidy up the repository');
      await Future<void>.delayed(const Duration(milliseconds: 50));
      await listening.cancel();
      expect(written.toString(), contains('Renamed everywhere.'));
      expect(asked, isNotEmpty, reason: 'the scripted agent asked before it edited');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('a worktree is made and removed through the host over the pipe', () async {
    Future<ProcessResult> git(String dir, List<String> args) => Process.run('git',
        ['-C', dir, '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid', '-c', 'commit.gpgsign=false', ...args]);
    final repo = '${host.dir.path}/src/proj';
    await Directory(repo).create(recursive: true);
    await File('$repo/README').writeAsString('hello\n');
    for (final args in [['init', '-q'], ['add', '.'], ['commit', '-q', '-m', 'first']]) {
      expect((await git(repo, args)).exitCode, 0);
    }
    // The key is paired by the first test, which runs before this one.
    final worktrees = HostWorktrees((method, params) => hostWorktreeRequestOverPipe(
      host.treesUrl, expectedPeerId: host.peer, method: method, params: params,
      verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: host.method),
    ));
    final made = await worktrees.create(repo, 'canoa/first-a1b2c3');
    expect(made.path.replaceAll(r'\', '/'), endsWith('/src/proj.worktrees/first-a1b2c3'));
    await expectLater(worktrees.create(host.dir.path, 'canoa/outside'), throwsA(isA<WorktreeFailure>()),
        reason: 'the host directory is outside the root');
    expect((await worktrees.remove(made)).branchKept, isFalse);
  }, skip: skip ?? (Process.runSync('git', ['--version']).exitCode == 0 ? null : 'no git on PATH'));
  test('a session over the pipe carries the cache lifetime the app asked for', () async {
    for (final (lifetime, ttl) in [(CacheLifetime.short, '5m'), (CacheLifetime.long, '1h')]) {
      final record = File('${host.dir.path}/record.jsonl');
      if (record.existsSync()) record.deleteSync();
      final client = await connectVerifiedAcpOverPipe(
        host.claudeUrl,
        expectedPeerId: host.peer,
        verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: host.method),
        initializeMeta: {cacheLifetimeMeta: cacheLifetimeRequest(lifetime, chosen: true)},
      );
      try {
        await client.openSession(cwd: host.dir.path);
      } finally {
        await client.close();
      }
      final started = jsonDecode(record.readAsLinesSync().first) as Map<String, dynamic>;
      expect((started['env'] as Map)['CLAUDE_CODE_PROMPT_CACHE_TTL'], ttl, reason: '$lifetime');
    }
  }, skip: skip ?? exampleMissing('harness_standin'));
  test('a host that does not hold the pinned key is refused before any message', () async {
    final other = 'sha256:${'ab' * 32}';
    await expectLater(connect(expected: other), throwsA(anything));
  }, skip: skip);

  test('a pipe nobody listens on is reported, not waited for', () async {
    final missing = Platform.isWindows ? r'pipe:canoa-dart-test-nobody-here' : 'unix:/tmp/canoa-dart-test-nobody-here.sock';
    await expectLater(openPipeChannel(missing), throwsA(isA<TransportUnavailable>()));
  }, skip: skip);
}
