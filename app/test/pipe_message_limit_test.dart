// The longest message the chat's pipe channel takes follows the host's `[host] max_message_bytes`, which the host says in its answer
// to `initialize`: a prompt over the host's default limit is refused naming the setting, and one the raised setting allows reaches
// the agent and comes back, against the real host binary and its echoing `stdio_agent`.
//
// CANOA_TEST_HOST names a built canoa with its examples (stdio_agent, web_peer) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the tests are skipped.
@Timeout(Duration(minutes: 5))
library;

import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

const int _mebibyte = 1024 * 1024;

/// `canoa serve` with the echoing agent on a pipe, with `[host] max_message_bytes` set when [limit] is given, and the key of
/// [browser] accepted.
final class LimitedHost {
  LimitedHost._(this.process, this.dir, this.url, this.peer, this.method);

  final Process process;
  final Directory dir;
  final String url;
  final String peer;
  final PeerMethod method;

  static Future<LimitedHost> start(WebPeerBridge browser, {int? limit}) async {
    final dir = await Directory.systemTemp.createTemp('canoa-limit-test');
    final unique = '$pid-${Random().nextInt(1 << 30)}';
    final listen = Platform.isWindows ? 'pipe:canoa-dart-limit-$unique' : 'unix:${dir.path}/host.sock';
    final config = File('${dir.path}/canoa.cfg');
    await config.writeAsString('''
${limit == null ? '' : '[host]\nmax_message_bytes = $limit\n'}
[host.agents.fixture]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
listen = ${jsonEncode(listen)}
''');
    final home = '${dir.path}/home';
    final process = await Process.start(_hostBinary!, ['serve', '--config', config.path], environment: {'COQUI_AGENTIC_PEER_HOME': home});
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final lines = process.stdout.transform(utf8.decoder).transform(const LineSplitter());
    final entry = jsonDecode(await lines.first.timeout(const Duration(seconds: 30))) as Map<String, dynamic>;
    final host = LimitedHost._(
      process,
      dir,
      entry['url'] as String,
      entry['peer'] as String,
      PeerMethod.parse(entry['method'] as String),
    );
    // The first attempt of an unpaired key is turned away and waits for the person to accept it on the host.
    await host.connect(browser).then<void>((client) => client.close(), onError: (Object _) {});
    final accepted = await Process.run(_hostBinary!, ['accept', await browser.localId(host.method)], environment: {'COQUI_AGENTIC_PEER_HOME': home});
    if (accepted.exitCode != 0) throw StateError('accept failed: ${accepted.stderr}');
    return host;
  }

  Future<VerifiedAgent> connect(WebPeerBridge browser) => connectVerifiedAcpOverPipe(
    url,
    expectedPeerId: peer,
    verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
  );

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
    await dir.delete(recursive: true);
  }
}

/// What the agent said in the turn that answers [text].
Future<String> _said(VerifiedAgent client, String session, String text) async {
  final written = StringBuffer();
  final listening = client.updates.listen((update) {
    if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
  });
  try {
    await client.prompt(session, text);
    await Future<void>.delayed(const Duration(milliseconds: 100));
  } finally {
    await listening.cancel();
  }
  return written.toString();
}

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : exampleMissing('stdio_agent') ?? exampleMissing('web_peer');
  late WebPeerBridge browser;

  setUpAll(() async {
    if (skip != null) return;
    browser = await WebPeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
  });

  test('a prompt over the host\'s default limit is refused naming the setting, and the connection goes on', () async {
    final host = await LimitedHost.start(browser);
    try {
      final client = await host.connect(browser);
      try {
        expect(client.identity.maxMessageBytes, 16 * _mebibyte);
        final opened = await client.openSession(cwd: host.dir.path);
        final failed = client.events.where((event) => event is AcpFailed).cast<AcpFailed>().first;
        expect(await client.prompt(opened.id, 'a' * (20 * _mebibyte)), isNull);
        expect((await failed.timeout(const Duration(seconds: 10))).message, contains('max_message_bytes'));
        expect(await _said(client, opened.id, 'hi'), 'echo: hi');
      } finally {
        await client.close();
      }
    } finally {
      await host.stop();
    }
  }, skip: skip);

  test('a prompt over the default is taken when the host\'s setting is raised, and comes back whole', () async {
    final size = 20 * _mebibyte;
    final host = await LimitedHost.start(browser, limit: 32 * _mebibyte);
    try {
      final client = await host.connect(browser);
      try {
        expect(client.identity.maxMessageBytes, 32 * _mebibyte);
        final opened = await client.openSession(cwd: host.dir.path);
        final said = await _said(client, opened.id, 'a' * size);
        expect(said.length, 'echo: '.length + size);
      } finally {
        await client.close();
      }
    } finally {
      await host.stop();
    }
  }, skip: skip);
}
