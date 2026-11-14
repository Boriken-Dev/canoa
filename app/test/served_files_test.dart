// What a web page does with its own host, through the app's own client code over the control channel the page reaches after
// pairing: choose a folder in the user's home, start a session of the scripted agent in it, change its model and effort, install an
// overlay from a local catalog through the store's code, select it, and reopen the session from the host's list after a reload.
// The page's side of the key proof is the web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa, with its examples (stdio_agent, web_peer) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them; CANOA_TEST_METHOD (`noise` or `webcrypto`) is the method the host
// proves by, `noise` unless set. Without CANOA_TEST_HOST, or without git on PATH, the tests are skipped.
@Timeout(Duration(minutes: 4))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/ledger/host_ledger_backend.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/report.dart';
import 'package:canoa/served/control_overlays.dart';
import 'package:canoa/served/host_config.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/served/served_control.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_settings.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/store_fixtures.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

const _grants = [
  'ledger.read', 'ledger.write', 'folders', 'config.read', 'config.write', 'upload', 'host.restart', 'overlay.read', 'overlay.write',
];

/// `canoa serve` as a supervisor runs it: started again, on the same configuration and home, when it ends with the status a
/// restart asks for.
final class _Host {
  _Host(this.dir, this.config, this.home, this.method);

  final Directory dir;
  final File config;
  final String home;
  final String method;
  late Process process;
  final urls = <String, String>{};
  late String peer;
  final _output = <String>[];

  Map<String, String> get _environment => {
    'HOME': home,
    'USERPROFILE': home,
    'COQUI_AGENTIC_PEER_HOME': '${dir.path}/peers',
  };

  Future<void> start() async {
    process = await Process.start(_hostBinary!, ['serve', '--config', config.path, '--method', method], environment: _environment);
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final ready = Completer<void>();
    process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      _output.add(line);
      final entry = jsonDecode(line) as Map<String, dynamic>;
      if (entry['name'] is String) {
        urls[entry['name'] as String] = entry['url'] as String;
        peer = entry['peer'] as String;
      }
      if (entry['ready'] == true && !ready.isCompleted) ready.complete();
    });
    await ready.future.timeout(const Duration(seconds: 60));
  }

  /// Waits for the host to end by itself and starts it again; the status it ended with.
  Future<int> restarted() async {
    final status = await process.exitCode.timeout(const Duration(seconds: 60));
    await start();
    return status;
  }

  Future<ProcessResult> command(List<String> args) => Process.run(_hostBinary!, args, environment: _environment);

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
  }
}

String _listing(_Host host, PeerMethod method) => jsonEncode({
  'peer': host.peer,
  'method': method.id,
  'entries': [
    for (final MapEntry(:key, :value) in host.urls.entries) {'name': key, 'kind': key == '@control' ? 'control' : 'agent', 'url': value},
  ],
});

void main() {
  final skip = _hostBinary == null
      ? 'CANOA_TEST_HOST names a built canoa'
      : exampleMissing('web_peer') ?? exampleMissing('stdio_agent') ?? (hasGit ? null : 'git is not on PATH');
  late Directory dir;
  late _Host host;
  late WebPeerBridge browser;
  late PeerMethod method;
  late String home;
  late Repo team;
  late ServedHost served;
  late String browserId;

  Future<VerifiedPeer> prove(channel, expected) => provePeer(browser, channel, expected, method: method);

  final opened = <HostControl>[];

  /// A link to the control channel, closed when the test ends: the host holds only so many at once.
  HostControl control() {
    final made = servedControl(served, verify: prove);
    opened.add(made);
    return made;
  }

  Future<VerifiedAgent> connectScripted({Map<String, Object?> meta = const {}}) => connectVerifiedAcp(
    chatEndpoints(webSocket: host.urls['scripted']!),
    expectedPeerId: host.peer,
    verifyPeer: prove,
    initializeMeta: meta,
  );

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-served-');
    final root = dir.path.replaceAll(r'\', '/');
    home = '$root/home';
    Directory('$home/projects/app').createSync(recursive: true);
    team = Repo('$home/repos', 'team')
      ..write('catalog.toml', '[overlays.docs-tools]\nsource = "./overlays/docs-tools"\n\n[overlays.docs-tools.about]\nsummary = "Docs tools."\n')
      ..write('overlays/docs-tools/overlay.cfg', '[env]\nDOCS = "one"\n')
      ..commit('catalog');
    browser = await WebPeerBridge.start();
    method = PeerMethod.parse(Platform.environment['CANOA_TEST_METHOD'] ?? 'noise');
    browserId = await browser.localId(method);
    final agent = jsonEncode(exampleBinary('stdio_agent'));
    final config = File('$root/cfg/canoa.cfg')..createSync(recursive: true);
    config.writeAsStringSync('''
[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ${jsonEncode(_grants)} }

[host.agents.scripted]
program = $agent
args = ["--scripted"]
listen = "127.0.0.1:${await freeLoopbackPort()}"
working_roots = [${jsonEncode(home)}]
peers = [${jsonEncode(browserId)}]
own_profile = true
''');
    host = _Host(dir, config, home, method.id);
    await host.start();
    served = ServedHost.parse(_listing(host, method))!;
    // The page's key is turned away until the person accepts it on the host.
    await expectLater(HostFiles(control()).listFolders(), throwsA(anything));
    final accepted = await host.command(['accept', browserId]);
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
    await host.stop();
    await dir.delete(recursive: true);
  });

  test('the list the host serves names its control channel apart from its agents', () {
    expect(served.control?.url, host.urls['@control']);
    expect([for (final entry in served.entries) entry.name], ['scripted']);
  }, skip: skip);

  test('a folder is chosen in the home and the working roots, and nothing outside them is listed', () async {
    final files = HostFiles(control());
    final places = await files.listFolders();
    expect(places.path, isNull);
    // The host writes a path as the machine does: with backslashes on Windows.
    String same(String? path) => path!.replaceAll(r'\', '/');
    expect(places.entries.map((place) => same(place.path)), [home]);
    final inHome = await files.listFolders(path: places.entries.single.path);
    expect(same(inHome.path), home);
    expect(inHome.entries.map((entry) => entry.name), contains('projects'));
    final nested = await files.listFolders(path: inHome.entries.firstWhere((entry) => entry.name == 'projects').path);
    expect(nested.entries.map((entry) => entry.name), ['app']);
    expect(same(nested.parent), home);
    await expectLater(files.listFolders(path: dir.parent.path), throwsA(isA<ControlFailed>()));
    await expectLater(files.listFolders(path: '$home/../'), throwsA(isA<ControlFailed>()));
  }, skip: skip);

  test('a file is uploaded to the host within its limit, and removed', () async {
    final files = HostFiles(control());
    final sent = await files.upload('notes.txt', utf8.encode('hello'));
    expect(File(sent.path).readAsStringSync(), 'hello');
    // A file larger than one message goes in chunks and arrives whole.
    final large = List<int>.generate(2 * 1024 * 1024 + 5, (index) => index % 251);
    final progress = <int>[];
    final big = await files.upload('large.bin', large, progress: progress.add);
    expect(File(big.path).readAsBytesSync(), large);
    expect(progress.length, greaterThan(1));
    expect(progress.last, large.length);
    expect(await files.removeUpload(big.path), isTrue);
    final empty = await files.upload('empty.txt', const []);
    expect(File(empty.path).lengthSync(), 0);
    expect(await files.removeUpload(empty.path), isTrue);
    expect(sent.name, endsWith('notes.txt'));
    expect(await files.removeUpload(sent.path), isTrue);
    expect(File(sent.path).existsSync(), isFalse);
    await expectLater(files.upload('big.bin', List.filled(9 * 1024 * 1024, 1)), throwsA(isA<ControlFailed>()));
    final left = Directory('${host.dir.path}/peers/control/uploads');
    expect(left.existsSync() ? left.listSync(recursive: true).whereType<File>() : const <File>[], isEmpty,
        reason: 'a file the host refused part-way is not kept');
  }, skip: skip);

  test('a session starts in the chosen folder, answers, changes its model and effort, and is reopened from the host\'s list after a reload',
      () async {
    final control1 = control();
    final folder = '$home/projects/app';
    final client = await connectScripted();
    final opened = await client.openSession(cwd: folder);
    final written = StringBuffer();
    final listening = client.updates.listen((update) {
      if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
    });
    await client.prompt(opened.id, 'hello');
    await Future<void>.delayed(const Duration(milliseconds: 100));
    await listening.cancel();
    expect(written.toString(), isNotEmpty, reason: 'the scripted agent answered');

    // Model and effort are the agent's config options, changed while it runs.
    final options = opened.raw['configOptions'] as List<dynamic>;
    String? other(String id) {
      final option = options.cast<Map<String, dynamic>>().firstWhere((option) => option['id'] == id);
      final values = [for (final value in option['options'] as List<dynamic>) (value as Map<String, dynamic>)['value'] as String];
      return values.firstWhere((value) => value != option['currentValue']);
    }
    for (final id in ['model', 'effort']) {
      final value = other(id)!;
      final changed = await client.setConfigOption(opened.id, id, value);
      final now = (changed['configOptions'] as List<dynamic>).cast<Map<String, dynamic>>().firstWhere((option) => option['id'] == id);
      expect(now['currentValue'], value, reason: id);
    }

    // The session is kept in the host's list, in the host's home, and read again by a page that loads afterwards.
    final ledger = SessionLedger(HostLedgerBackend.over(control1));
    await ledger.load();
    ledger.record(
      peerId: host.peer, sessionId: opened.id, entry: 'scripted', place: SessionPlace.host, cwd: folder, title: 'Fix calc',
      endpoint: host.urls['scripted'], method: method.id,
    );
    await ledger.flush();
    await client.close();
    await control1.close();

    final reloaded = SessionLedger(HostLedgerBackend.over(control()));
    await reloaded.load();
    final kept = reloaded.find(host.peer, opened.id);
    expect(kept, isNotNull, reason: 'the host kept it');
    expect(kept!.cwd, folder);
    expect(kept.title, 'Fix calc');
    final again = await connectScripted();
    final reopened = await again.reopenSession(opened.id, cwd: kept.cwd!);
    expect(reopened.id, opened.id);
    await again.close();
  }, skip: skip);

  test('an overlay is installed from a local catalog through the store, selected, and reaches the next session', () async {
    final overlays = ControlOverlayManager(control());
    final files = HostFiles(control());
    final config = HostConfigStore(files);
    var current = await config.load();
    expect(current.overlays, isEmpty);

    final catalogs = CanoaConfig.parse('[catalogs]\nteam = "${team.location}@main#catalog.toml"\n');
    final listing = StoreListing([
      for (final MapEntry(key: label, value: source) in catalogs.catalogs.entries)
        await loadCatalog(overlays, label, source, const StoreSettings()),
    ]);
    final entry = listing.choose('team/docs-tools').entry;
    final preview = await previewOverlay(overlays, installName(entry), installSource(entry), os: 'linux', shared: const {});
    expect(preview.update.added, ['overlay.cfg']);
    expect(overlays.installed('docs-tools'), isNull, reason: 'a preview installs nothing');
    current = await installPreview(overlays, current, preview, catalog: 'team');
    final installed = overlays.installed('docs-tools')!;
    expect(installed.digest, preview.update.digest);
    expect(installed.catalog, 'team');
    // What the host wrote is what the host reads: the folder, with the digest the preview showed.
    expect(File('${host.config.parent.path}/overlays/docs-tools/overlay.cfg').readAsStringSync(), contains('DOCS'));

    // Selecting it is canoa.cfg's [overlays], written by table; the host reads it again for the next session, with no restart.
    expect(await config.save(current), isFalse, reason: 'no restart is needed for [overlays]');
    expect(File(host.config.path).readAsStringSync(), contains('docs-tools'));
    expect(File(host.config.path).readAsStringSync(), contains('[host.control]'), reason: 'the tables it does not edit stay');

    // The overlays folder is the host's: a page that loads afterwards reads what is installed and what is selected.
    final after = ControlOverlayManager(control());
    await after.refresh();
    expect(after.installed('docs-tools')?.digest, preview.update.digest);
    expect((await HostConfigStore(HostFiles(control())).load()).overlays.keys, ['docs-tools']);

    final client = await connectScripted(meta: {overlaysMeta: ['docs-tools']});
    final opened = await client.openSession(cwd: '$home/projects/app');
    final report = hostProfileOf((opened.raw['_meta'] as Map<String, dynamic>)[hostProfileMeta], fallback: 'scripted');
    expect(report?.report.items.map((item) => item.key), contains('env.DOCS'), reason: 'the overlay reached the session');
    await client.close();

    // And it goes the same way: removed through the store, and no longer selected.
    final withoutIt = await removeInstalled(after, current, 'docs-tools', installSource(entry));
    expect(withoutIt.overlays, isEmpty);
    expect(after.installed('docs-tools'), isNull);
  }, skip: skip);
}
