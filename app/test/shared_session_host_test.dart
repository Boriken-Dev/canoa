// Two app clients on one session the real `canoa` keeps for several clients (an agent entry with `shared = true`), each with a key of
// its own: the second joins with the conversation replayed, they see each other attached, what one says reaches the other, a
// permission request answered by one closes on the other, and one that comes back is replayed only what it missed.
//
// CANOA_TEST_HOST names a built canoa, with its examples (stdio_agent) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the test is skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path => File(path).absolute.path,
  null => null,
};
final String? _method = switch (Platform.environment['CANOA_TEST_METHOD']) {
  final String method when method.isNotEmpty => method,
  _ => null,
};

String _example(String name) {
  final examples = File(_hostBinary!).parent.uri.resolve('examples/').toFilePath();
  return '$examples${Platform.isWindows ? '$name.exe' : name}';
}

/// One app: its key, its conversation list and what it shows.
final class _App {
  _App(this.bridge) : ledger = SessionLedger(MemoryLedgerBackend()) {
    store = SessionStore(ledger: ledger);
  }

  final WebPeerBridge bridge;
  final SessionLedger ledger;
  late final SessionStore store;

  List<String> said(ChatSession session) => [
    for (final message in session.messages)
      if (message.role == MessageRole.user || message.role == MessageRole.agent) message.text,
  ];
}

Future<void> _until(bool Function() done, String what) async {
  final limit = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(limit)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : null;

  test('two app clients take part in one shared session of the real host', () async {
    final dir = await Directory.systemTemp.createTemp('canoa-shared-app-test');
    final config = File('${dir.path}/canoa.cfg');
    await config.writeAsString('''
[host.agents.keep]
program = ${jsonEncode(_example('stdio_agent'))}
shared = true
''');
    final process = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path, if (_method case final method?) ...['--method', method]],
      environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
    );
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final bridges = <WebPeerBridge>[];
    final clients = <VerifiedAgent>[];
    try {
      final first = await process.stdout
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .first
          .timeout(const Duration(seconds: 30));
      final served = jsonDecode(first) as Map<String, dynamic>;
      final url = served['url'] as String, peer = served['peer'] as String;
      final method = PeerMethod.parse(served['method'] as String);
      final one = _App(await WebPeerBridge.start()), two = _App(await WebPeerBridge.start());
      bridges.addAll([one.bridge, two.bridge]);

      Future<VerifiedAgent> connect(_App app) async {
        final client = await connectVerifiedAcp(
          chatEndpoints(webSocket: url),
          expectedPeerId: peer,
          verifyPeer: (channel, expected) => provePeer(app.bridge, channel, expected, method: method),
          decider: app.store.askPermission,
        );
        clients.add(client);
        return client;
      }

      Future<ProcessResult> command(List<String> args) =>
          Process.run(_hostBinary!, args, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'});

      // Each key is turned away until the person accepts it on the host, which notes it only once it has been turned away.
      for (final app in [one, two]) {
        final id = await app.bridge.localId(method);
        var accepted = false;
        for (var tries = 0; tries < 40 && !accepted; tries++) {
          try {
            await (await connect(app)).close();
            accepted = true;
          } on Object {
            accepted = (await command(['accept', id])).exitCode == 0;
            if (!accepted) await Future<void>.delayed(const Duration(milliseconds: 250));
          }
        }
        expect(accepted, isTrue, reason: 'the host lists ${(await command(['peers', '--format', 'json'])).stdout}');
      }

      // The first opens a session and says something in it.
      final clientOne = await connect(one);
      final opened = await clientOne.openSession(cwd: dir.path);
      final remote = opened.id;
      final sessionOne = one.store.addProviderSession(
        title: 'Shared', agent: 'keep', peerId: clientOne.peerId, remoteSessionId: remote, cwd: dir.path,
      );
      one.store.attachVerifiedClient(sessionOne.id, clientOne);
      await one.store.sendTextTo(sessionOne, 'one');
      await _until(() => one.said(sessionOne).contains('echo: one'), "the agent's answer");

      // The second joins it from its own list, and is replayed all of it.
      final clientTwo = await connect(two);
      final entryTwo = two.ledger.record(
        peerId: clientTwo.peerId, sessionId: remote, entry: 'keep', place: SessionPlace.host, reopenBy: 'session/resume', cwd: dir.path,
        title: 'Shared',
      )!;
      final sessionTwo = await two.store.reopen(entryTwo, clientTwo, attach: true);
      expect(two.said(sessionTwo), ['one', 'echo: one']);
      expect(entryTwo.cursor, isNotNull, reason: 'it keeps where in the host\'s log it is');

      // Each sees the other attached, and not itself.
      await _until(() => sessionOne.others.length == 1 && sessionTwo.others.length == 1, 'each to see the other');
      expect(sessionOne.others.single.id, isNot(sessionTwo.others.single.id));

      // What one says reaches the other as it happens.
      await two.store.sendTextTo(sessionTwo, 'two');
      await _until(() => one.said(sessionOne).contains('echo: two'), "the second's turn to reach the first");
      expect(one.said(sessionOne), ['one', 'echo: one', 'two', 'echo: two']);

      // A permission request goes to both, and the one answered by the second closes on the first.
      final asking = one.store.sendTextTo(sessionOne, 'ask first');
      await _until(() => sessionOne.pendingPermissions.isNotEmpty && sessionTwo.pendingPermissions.isNotEmpty, 'the request on both');
      two.store.answerPermission(sessionTwo, allow: true);
      await _until(() => sessionOne.pendingPermissions.isEmpty, 'the request to close on the first');
      await asking;
      await _until(() => one.said(sessionOne).contains('the person chose allow'), 'the turn to go on');
      expect(
        sessionOne.messages.map((message) => message.text),
        contains(contains('answered "Write a file"')),
        reason: 'the first is told it was answered elsewhere',
      );

      // The second's link ends; what is said meanwhile is replayed to it when it comes back, and nothing it had is repeated.
      await _until(() => two.said(sessionTwo).contains('the person chose allow'), 'the second to see the turn end');
      final before = two.said(sessionTwo);
      await clientTwo.close();
      await one.store.sendTextTo(sessionOne, 'three');
      final back = await connect(two);
      final again = await two.store.reopen(entryTwo, back, attach: true);
      expect(identical(again, sessionTwo), isTrue);
      expect(two.said(sessionTwo), [...before, 'three', 'echo: three']);
    } finally {
      for (final client in clients) {
        await client.close().catchError((Object _) {});
      }
      for (final bridge in bridges) {
        bridge.kill();
      }
      process.kill();
      await process.exitCode;
      await dir.delete(recursive: true);
    }
  }, skip: skip);
}
