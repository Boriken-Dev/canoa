// The Terminal's shell on a real `canoa serve`, through the app's own code: a page's key proves itself, the host grants it
// `terminal`, a shell opens in a folder of the person's, what is typed comes back through the emulator, a resize reaches it and
// closing ends it. The page's side of the key proof is the web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa, with its examples (web_peer) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. Without it the test is skipped.
@Timeout(Duration(minutes: 3))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart' show VerifiedPeer;
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_grants.dart';
import 'package:canoa/desktop/host_tool_io.dart' show freeLoopbackPort;
import 'package:canoa/served/served_control.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : exampleMissing('web_peer');
  late Directory dir;
  late Process host;
  late WebPeerBridge browser;
  late ServedHost served;
  late String home;
  late PeerMethod method;
  final opened = <HostControl>[];

  Future<VerifiedPeer> prove(channel, expected) => provePeer(browser, channel, expected, method: method);

  HostControl control() {
    final made = servedControl(served, verify: prove);
    opened.add(made);
    return made;
  }

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-terminal-');
    final root = dir.path.replaceAll(r'\', '/');
    home = '$root/home';
    Directory(home).createSync(recursive: true);
    browser = await WebPeerBridge.start();
    method = PeerMethod.parse(Platform.environment['CANOA_TEST_METHOD'] ?? 'noise');
    final browserId = await browser.localId(method);
    final config = File('$root/cfg/canoa.cfg')..createSync(recursive: true);
    config.writeAsStringSync('''
[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ["terminal", "folders"] }
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
    served = ServedHost.parse(
      jsonEncode({
        'peer': peer,
        'method': method.id,
        'entries': [
          for (final MapEntry(:key, :value) in urls.entries) {'name': key, 'kind': key == '@control' ? 'control' : 'agent', 'url': value},
        ],
      }),
    )!;
    // The page's key is turned away until the person accepts it on the host.
    await expectLater(control().ready().then((client) => client.call('host/grants')), throwsA(anything));
    final accepted = await Process.run(_hostBinary!, ['accept', browserId], environment: environment);
    expect(accepted.exitCode, 0, reason: '${accepted.stderr}');
  });

  tearDown(() async {
    for (final each in opened) {
      await each.close();
    }
    opened.clear();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    host.kill();
    await host.exitCode;
    await dir.delete(recursive: true);
  });

  test('the host says the key holds terminal', () async {
    final grants = HostGrants(() => control().ready());
    await grants.refresh();
    expect(grants.may(HostGrants.terminal), isTrue);
  }, skip: skip);

  test('a shell opens in the folder given, echoes what is typed through the emulator, takes a resize and ends on exit', () async {
    final pane = TerminalPane(host: ControlTerminals(control()), cwd: home);
    addTearDown(pane.close);
    await pane.start();
    expect(pane.state, PaneState.running, reason: '${pane.problem}');

    pane.terminal.textInput('echo canoa-marker-xxx\r');
    final deadline = DateTime.now().add(const Duration(seconds: 40));
    // The shell echoes the line typed and then prints it: the marker is on the screen at least twice once it has run.
    bool ran() => 'canoa-marker-xxx'.allMatches(pane.terminal.buffer.getText()).length >= 2;
    while (!ran() && DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
    expect(ran(), isTrue, reason: pane.terminal.buffer.getText());

    pane.terminal.resize(120, 40);
    await Future<void>.delayed(const Duration(milliseconds: 300));
    expect(pane.state, PaneState.running, reason: 'a resize the host took leaves the shell as it was');

    pane.terminal.textInput('exit\r');
    final ended = Completer<void>();
    void watch() {
      if (pane.state == PaneState.exited && !ended.isCompleted) ended.complete();
    }

    pane.addListener(watch);
    watch();
    await ended.future.timeout(const Duration(seconds: 30));
    expect(pane.exitCode, 0);
  }, skip: skip);

  test('a shell is opened with one the host listed, by its id, and one it did not list is refused in its words', () async {
    final terminals = ControlTerminals(control());
    final listed = await terminals.shells();
    // Whatever this machine has: the host lists what it finds, and the list may be empty.
    if (listed.isNotEmpty) {
      final chosen = listed.first;
      final pane = TerminalPane(host: terminals, cwd: home, shell: chosen.id);
      addTearDown(pane.close);
      await pane.start();
      expect(pane.state, PaneState.running, reason: '${chosen.id}: ${pane.problem}');
    }
    final unknown = TerminalPane(host: terminals, cwd: home, shell: 'no-such-shell-3117');
    addTearDown(unknown.close);
    await unknown.start();
    expect(unknown.state, PaneState.failed);
    expect(unknown.problem, contains('shell/list'));
  }, skip: skip);

  test('a folder that does not exist is refused in the host\'s words, and the pane says so', () async {
    final pane = TerminalPane(host: ControlTerminals(control()), cwd: '$home/not-there');
    addTearDown(pane.close);
    await pane.start();
    expect(pane.state, PaneState.failed);
    expect(pane.problem, isNotEmpty);
    expect(pane.problem, isNot('The host could not be reached.'));
  }, skip: skip);
}
