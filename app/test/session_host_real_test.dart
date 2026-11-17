// A second `canoa serve` home as a saved host a new session is aimed at: the app's own code reads its folders over the control
// channel (the calls the folder browser and the check before Send make), has it make a chat's folder, and starts its scripted
// agent in a workspace picked there. The page's side of the key proof is the web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa, with its examples (web_peer, stdio_agent) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the tests are skipped. Only temporary homes are used.
@Timeout(Duration(minutes: 3))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/host_tool_io.dart' show freeLoopbackPort;
import 'package:canoa/served/served_control.dart';
import 'package:canoa/workspace/remote_host.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : exampleMissing('web_peer') ?? exampleMissing('stdio_agent');
  late Directory dir;
  late Process host;
  late WebPeerBridge browser;
  late RemoteHost remote;
  late String home;
  late String agentUrl;
  late PeerMethod method;

  Future<VerifiedPeer> prove(channel, expected) => provePeer(browser, channel, expected, method: method);

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-second-home-');
    final root = dir.path.replaceAll(r'\', '/');
    home = '$root/home';
    Directory('$home/work/app').createSync(recursive: true);
    browser = await WebPeerBridge.start();
    method = PeerMethod.parse(Platform.environment['CANOA_TEST_METHOD'] ?? 'noise');
    final browserId = await browser.localId(method);
    final config = File('$root/cfg/canoa.cfg')..createSync(recursive: true);
    config.writeAsStringSync('''
[sessions]
workspace = "$home/scratch/\${session}"

[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ["folders"] }

[host.agents.scripted]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
args = ["--scripted"]
''');
    final environment = {'HOME': home, 'USERPROFILE': home, 'COQUI_AGENTIC_PEER_HOME': '$root/peers'};
    host = await Process.start(_hostBinary!, ['serve', '--config', config.path, '--method', method.id], environment: environment);
    host.stderr.transform(utf8.decoder).listen(stderr.write);
    final urls = <String, String>{};
    var peer = '';
    final ready = Completer<void>();
    host.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final entry = jsonDecode(line) as Map<String, dynamic>;
      if (entry['name'] is String) {
        urls[entry['name'] as String] = entry['url'] as String;
        peer = entry['peer'] as String;
      }
      if (entry['ready'] == true && !ready.isCompleted) ready.complete();
    });
    await ready.future.timeout(const Duration(seconds: 60));
    agentUrl = urls['scripted']!;
    final served = ServedHost.parse(
      jsonEncode({
        'peer': peer,
        'method': method.id,
        'os': Platform.isWindows ? 'windows' : 'linux',
        'entries': [
          for (final MapEntry(:key, :value) in urls.entries) {'name': key, 'kind': key == '@control' ? 'control' : 'agent', 'url': value},
        ],
      }),
    )!;
    remote = RemoteHost(
      Connection(name: 'Second', kind: ConnectionKind.canoa, address: 'http://127.0.0.1/', peer: peer, method: method.id),
      served,
      servedControl(served, verify: prove),
    );
    // The key is turned away until the person accepts it on the host.
    await expectLater(remote.control.ready().then((client) => client.call('fs/list', const {})), throwsA(anything));
    final accepted = await Process.run(_hostBinary!, ['accept', browserId], environment: environment);
    expect(accepted.exitCode, 0, reason: '${accepted.stderr}');
  });

  tearDownAll(() async {
    if (skip != null) return;
    await remote.control.close();
    browser.kill();
    host.kill();
    await host.exitCode;
    await dir.delete(recursive: true);
  });

  test('the browser lists the other host\'s own places and folders, and the check before Send refuses what it cannot open', () async {
    final places = await remote.files.listFolders();
    expect(places.path, isNull);
    final under = await remote.files.listFolders(path: '$home/work');
    expect([for (final entry in under.entries) entry.name], contains('app'));
    expect(await remote.problemWith('$home/work/app'), isNull);
    expect(await remote.problemWith('$home/work/none'), contains('does not exist'));
    // A path of the other system is turned away before the host is asked.
    final other = Platform.isWindows ? '/srv/app' : r'C:\work\app';
    expect(await remote.problemWith(other), contains('not a path on Second'));
  }, skip: skip);

  test('the other host makes a chat\'s folder, and its scripted agent starts in a workspace picked there', () async {
    final made = await remote.folders.makeScratch((session: 'abc', source: 'scripted', handler: 'scripted', created: DateTime.now()));
    expect(made.made, isTrue);
    expect(made.path.replaceAll(r'\', '/'), contains('/scratch/abc'));
    expect(Directory(made.path).existsSync(), isTrue, reason: 'the host made it on its own disk');

    final client = await connectVerifiedAcp(
      chatEndpoints(webSocket: agentUrl),
      expectedPeerId: remote.served.peer,
      verifyPeer: prove,
    );
    try {
      final workspace = '$home/work/app';
      final opened = await client.openSession(cwd: workspace);
      final written = StringBuffer();
      final listening = client.updates.listen((update) {
        if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
      });
      await client.prompt(opened.id, 'hello');
      await Future<void>.delayed(const Duration(milliseconds: 100));
      await listening.cancel();
      expect(written.toString(), isNotEmpty, reason: 'the scripted agent answered the prompt');
    } finally {
      await client.close();
    }
  }, skip: skip);
}
