// The Hosts section's probe against a real `canoa serve --web`: the list it serves at /canoa.json, read as a saved connection reads
// it. A saved connection that names the host's id is online; one that names another is not shown as the host the person saved.
//
// CANOA_TEST_HOST names a built canoa, with the stdio_agent example in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/connection.dart';
import 'package:canoa/shell/hosts/host_status.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

Connection _saved(String address, String peer, {ConnectionKind kind = ConnectionKind.canoa}) => Connection(
  name: 'Real',
  kind: kind,
  address: address,
  peer: peer,
  method: 'noise',
  transport: ConnectionTransport.websocket,
);

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : exampleMissing('stdio_agent');
  late Directory dir;
  late Process process;
  late String web;
  late String peer;

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-probe-');
    final root = dir.path.replaceAll(r'\', '/');
    Directory('$root/site').createSync();
    File('$root/site/index.html').writeAsStringSync('<!doctype html><title>Canoa</title>');
    final webPort = await freePort();
    final agentPort = await freePort();
    final config = File('$root/canoa.cfg')
      ..writeAsStringSync('''
[host.agents.scripted]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
args = ["--scripted"]
listen = "127.0.0.1:$agentPort"
''');
    process = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path, '--method', 'noise', '--web', '$root/site', '--web-listen', '127.0.0.1:$webPort'],
      environment: {'HOME': '$root/home', 'USERPROFILE': '$root/home', 'COQUI_AGENTIC_PEER_HOME': '$root/peers'},
    );
    process.stderr.drain<void>();
    final ready = Completer<void>();
    process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final entry = jsonDecode(line) as Map<String, dynamic>;
      if (entry['peer'] is String) peer = entry['peer'] as String;
      if (entry['web'] is String) web = entry['web'] as String;
      if (entry['ready'] == true && !ready.isCompleted) ready.complete();
    });
    await ready.future.timeout(const Duration(seconds: 60));
  });

  tearDownAll(() async {
    if (skip != null) return;
    process.kill();
    await process.exitCode;
    await dir.delete(recursive: true);
  });

  test('a real host\'s list is read: online under its own id, with its agent and no control channel it does not listen for', () async {
    final found = await probeConnection(_saved(web, peer));
    expect(found.state, HostState.online, reason: '${found.detail}');
    expect(found.peer, peer);
    expect(found.method, 'noise');
    expect([for (final agent in found.agents) agent.name], ['scripted']);
  }, skip: skip);

  test('a connection that pins another id is not the host the person saved, and one that points at no host is not reachable', () async {
    final other = 'sha256:${'cd' * 32}';
    final changed = await probeConnection(_saved(web, other));
    expect(changed.state, HostState.changed);
    expect(changed.peer, peer, reason: 'what it says of itself is shown, never trusted');
    final nobody = await probeConnection(_saved('http://127.0.0.1:${await freePort()}/', peer));
    expect(nobody.state, HostState.offline);
  }, skip: skip);
}

Future<int> freePort() async {
  final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final port = socket.port;
  await socket.close();
  return port;
}
