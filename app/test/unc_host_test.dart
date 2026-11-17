// A workspace on a Windows share, through the app's own code against a real `canoa serve`: the folder is listed and checked, an `@`
// offers its names, a file is read and an upload kept there, and a terminal opens in it. The share is this machine's own
// administrative share (`\\localhost\C$\...`), so nothing leaves it; the tests are skipped elsewhere.
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
import 'package:canoa/desktop/host_tool_io.dart' show freeLoopbackPort;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/served/served_control.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/workspace/host_folders.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

/// [local] (a folder on a drive) as the administrative share names it: `C:\x` is `\\localhost\C$\x`.
String _share(String local) => '\\\\localhost\\${local[0]}\$${local.substring(2)}';

void main() {
  final skip = !Platform.isWindows
      ? 'a Windows share'
      : _hostBinary == null
      ? 'CANOA_TEST_HOST names a built canoa'
      : exampleMissing('web_peer');
  late Directory dir;
  late Process host;
  late WebPeerBridge browser;
  late ServedHost served;
  late PeerMethod method;
  late String work; // the workspace, written as the share names it
  final opened = <HostControl>[];

  Future<VerifiedPeer> prove(channel, expected) => provePeer(browser, channel, expected, method: method);

  HostControl control() {
    final made = servedControl(served, verify: prove);
    opened.add(made);
    return made;
  }

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-unc-');
    final root = dir.path.replaceAll(r'\', '/');
    final home = '$root/home';
    Directory(home).createSync(recursive: true);
    final local = Directory('${dir.path}\\work')..createSync(recursive: true);
    File('${local.path}\\notes.txt').writeAsStringSync('hello');
    Directory('${local.path}\\src').createSync();
    File('${local.path}\\src\\main.dart').writeAsStringSync('void main() {}');
    Directory('${local.path}\\.git').createSync();
    work = _share(local.path);
    browser = await WebPeerBridge.start();
    method = PeerMethod.parse(Platform.environment['CANOA_TEST_METHOD'] ?? 'noise');
    final browserId = await browser.localId(method);
    final config = File('$root/cfg/canoa.cfg')..createSync(recursive: true);
    config.writeAsStringSync('''
[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ["terminal", "folders", "files", "upload"] }
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

  test('the folder browser lists a share folder and names every entry as the share does', () async {
    final files = HostFiles(control());
    final listing = await files.listFolders(path: work, files: true);
    expect(listing.path, work);
    expect({for (final entry in listing.entries) entry.name}, containsAll(['notes.txt', 'src']));
    for (final entry in listing.entries) {
      expect(entry.path, startsWith(work), reason: 'written plain, never as \\\\?\\UNC\\');
    }
    final verbatim = r'\\?\UNC\' + work.substring(2);
    expect((await files.listFolders(path: verbatim)).path, work);
    expect(listing.parent, startsWith(r'\\localhost\C$'));
  }, skip: skip);

  test('the Files panel reads a file on the share', () async {
    final read = await HostFiles(control()).readFile('$work\\notes.txt');
    expect(utf8.decode(read.bytes!), 'hello');
  }, skip: skip);

  test('the folder is found to be a repository, and an @ offers its names and resolves one', () async {
    final folders = HostFolders(control());
    addTearDown(folders.dispose);
    final said = (await folders.info([work]))[work];
    expect(said, isNotNull);
    expect(said!.exists, isTrue);
    expect(said.repository, isTrue);
    expect(await folders.isRepository(work), isTrue);
    expect(await folders.mentions(work), containsAll(['notes.txt', 'src/main.dart']));
    final inside = await folders.resolve(work, 'src/main.dart');
    expect(inside, isNotNull);
    expect(inside!.path, startsWith(work));
  }, skip: skip);

  test('an upload is kept and removed again', () async {
    final files = HostFiles(control());
    final put = await files.upload('report.txt', utf8.encode('upload'));
    expect(File(put.path).readAsStringSync(), 'upload');
    expect(await files.removeUpload(put.path), isTrue);
  }, skip: skip);

  test('a terminal opens in the share folder and the shell is there', () async {
    final pane = TerminalPane(host: ControlTerminals(control()), cwd: work);
    addTearDown(pane.close);
    await pane.start();
    expect(pane.state, PaneState.running, reason: '${pane.problem}');
    final name = dir.path.split(r'\').last;
    // The shell prints its own folder with the marker split so the echoed command line does not match.
    pane.terminal.textInput('"canoa_"+"here_"+(Split-Path -Leaf (Split-Path -Parent (Get-Location).Path))\r');
    final deadline = DateTime.now().add(const Duration(seconds: 40));
    bool shown() => pane.terminal.buffer.getText().contains('canoa_here_$name');
    while (!shown() && DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
    expect(shown(), isTrue, reason: pane.terminal.buffer.getText());
  }, skip: skip);
}
