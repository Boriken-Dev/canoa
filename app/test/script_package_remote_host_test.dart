// A package of scripts installed through the Store onto a host the app reaches over the network, and not onto the one beside it:
// two real hosts run as processes of their own, each in a folder of this test, and the app's own Store code (the Extensions type's
// page, its plan and its confirm) is given the control channel of the remote one. The package is a folder of JavaScript modules in
// a git catalog: no build and no platform, the same bytes the Rust tests install. The host runs it with the QuickJS built into
// its program, so after it starts again the package's Store type is listed by the remote host and the near one has none of it.
//
// The page's side of the key proof is the web_peer example, since the Dart VM has no WebCrypto. CANOA_TEST_HOST names a built
// canoa, with its examples (stdio_agent, web_peer) in examples/ beside it, as `cargo build -p canoa-host --bin canoa --examples`
// leaves them; CANOA_TEST_METHOD (`noise` or `webcrypto`) is the method the hosts prove by, `noise` unless set. Without CANOA_TEST_HOST,
// or without git on PATH, the test is skipped.
@Timeout(Duration(minutes: 4))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart' show VerifiedPeer;
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool_io.dart' show freeLoopbackPort;
import 'package:canoa/profile/aos_support.dart' show treeOfFiles;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/served/served_control.dart';
import 'package:canoa/store/store_extension.dart' show ExtensionBody;
import 'package:canoa/store/store_extension_catalog.dart';
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart' show finish, settle, until;
import 'fixtures/examples.dart';
import 'fixtures/served_process.dart';
import 'fixtures/store_fixtures.dart' show Repo, hasGit;
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

/// What this page's key may do on a host: read and write the Store's catalogs, change the host's own configuration, and have it
/// start again.
const _grants = [
  'overlay.read',
  'overlay.write',
  'hosting',
  'config.read',
  'config.write',
  'host.restart',
  'store.read',
];

/// The modules of the SDK the package imports, copied beside it so the package is the whole of what is shipped.
const _sdk = ['store.js', 'profile.js', 'kinds.js', 'model.js'];

const _module = '''
import { StoreExtension } from './sdk/store.js';
import { EnvProfileItem } from './sdk/profile.js';

export class Notes extends StoreExtension {
  static title = 'Team notes';
  static publisher = 'Example publisher';
  static types = [{ id: 'notes', title: 'Notes', icon: 'extension' }];
}

export class Region extends EnvProfileItem {
  static item = 'region';
  static title = 'Region';
  static values = { type: 'object', properties: { name: { type: 'string', title: 'Name' } }, required: ['name'] };
  static variables = { name: ['REGION'] };
}
''';

const _manifest =
    'type = "js"\nname = "team_notes"\nkinds = ["store_extension", "profile_item"]\npaths = ["."]\n';
const _package =
    '{"name": "team_notes", "version": "1.0.0", "type": "module", "exports": "./team_notes.js"}\n';

/// The address git takes for a repository on this disk.
String _url(String location) =>
    location.startsWith('/') ? 'file://$location' : 'file:///$location';

void main() {
  final skip = _hostBinary == null
      ? 'CANOA_TEST_HOST names a built canoa'
      : exampleMissing('web_peer') ??
            exampleMissing('stdio_agent') ??
            (hasGit ? null : 'git is not on PATH');
  late Directory dir;
  late WebPeerBridge browser;
  late PeerMethod method;
  late String browserId;
  late ServedProcess near;
  late ServedProcess remote;
  late String pin;
  final opened = <HostControl>[];
  final started = <ServedProcess>[];

  Future<VerifiedPeer> prove(channel, expected) =>
      provePeer(browser, channel, expected, method: method);

  /// A link to a host's control channel, closed when the test ends.
  HostControl control(ServedProcess host) {
    final made = servedControl(
      ServedHost.parse(host.listing(method))!,
      verify: prove,
    );
    opened.add(made);
    return made;
  }

  /// A host serving the scripted agent and a control channel this page's key is granted, over a catalog of the package.
  Future<ServedProcess> startHost(String name, String catalog) async {
    final host = '${dir.path.replaceAll(r'\', '/')}/$name';
    Directory('$host/home').createSync(recursive: true);
    final config = File('$host/cfg/canoa.cfg')..createSync(recursive: true);
    final agent = jsonEncode(exampleBinary('stdio_agent'));
    config.writeAsStringSync('''
[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ${jsonEncode(_grants)} }

[host.agents.scripted]
program = $agent
args = ["--scripted"]
listen = "127.0.0.1:${await freeLoopbackPort()}"
working_roots = [${jsonEncode('$host/home')}]
peers = [${jsonEncode(browserId)}]

[store.extensions.catalogs]
team = ${jsonEncode('$catalog@main#extensions.toml')}
''');
    final served = ServedProcess(
      binary: _hostBinary!,
      dir: Directory(host),
      config: config,
      method: method.id,
    );
    await served.start();
    started.add(served);
    // The page's key is turned away until the person accepts it on the host.
    await expectLater(
      HostFiles(control(served)).listFolders(),
      throwsA(anything),
    );
    final accepted = await served.command(['accept', browserId]);
    expect(accepted.exitCode, 0, reason: '${accepted.stderr}');
    return served;
  }

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-remote-scripts-');
    final root = dir.path.replaceAll(r'\', '/');
    // The catalog: a repository that lists the package, which it holds as a folder. Its pin is the digest of that folder, which
    // a host recomputes over what it fetched.
    final sdk = Directory.current.path.replaceAll(r'\', '/');
    final files = <String, List<int>>{
      'team_notes.js': utf8.encode(_module),
      'team_notes.guatiao': utf8.encode(_manifest),
      'package.json': utf8.encode(_package),
      for (final name in _sdk)
        'sdk/$name': File(
          '$sdk/../bindings/js/canoa-sdk/src/$name',
        ).readAsBytesSync(),
    };
    pin = '${treeOfFiles(files).digest.digest}';
    final catalog = Repo('$root/repos', 'scripts');
    for (final MapEntry(:key, :value) in files.entries) {
      catalog.write('pkg/team_notes/$key', value);
    }
    catalog.write(
      'extensions.toml',
      '[extensions.team_notes]\nsource = "./pkg/team_notes"\npin = "$pin"\nruntime = "js"\n'
          'provider = "team_notes_notes"\nneeds = { kind = "store_extension", version = "1" }\n',
    );
    catalog.commit('the catalog');
    browser = await WebPeerBridge.start();
    method = PeerMethod.parse(
      Platform.environment['CANOA_TEST_METHOD'] ?? 'noise',
    );
    browserId = await browser.localId(method);
    // Both hosts are given the same catalog: the near one could have been installed on, and is not.
    near = await startHost('near', _url(catalog.location));
    remote = await startHost('remote', _url(catalog.location));
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    for (final host in started) {
      await host.stop();
    }
    await dir.delete(recursive: true);
  });

  testWidgets(
    'the Store installs a package of scripts onto the paired host it is given, which lists what the package adds once it starts again',
    (tester) async {
      try {
        final remoteControl = control(remote);
        var saved = 0;
        final extensions = StoreExtensions(
          control: () => remoteControl.ready(wait: false),
          open: remoteControl.open,
          saved: () async => saved++,
        );
        addTearDown(extensions.dispose);
        final type = extensions.types.firstWhere(
          (each) => each.id == extensionsTypeId,
        );
        await tester.binding.setSurfaceSize(const Size(900, 900));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: ExtensionBody(
                type: type,
                view: StoreView(extensionsTypeId),
              ),
            ),
          ),
        );
        final install = find.byKey(
          const Key('store-entry-install-team-team_notes'),
        );
        await until(
          tester,
          () => install.evaluate().isNotEmpty,
          "the catalog's package in the Store",
        );
        expect(
          find.byKey(const Key('store-entry-remove-team-team_notes')),
          findsNothing,
          reason: 'nothing is installed yet',
        );

        // The host's own plan, to confirm; nothing is fetched before the person does.
        await tester.tap(install);
        await until(
          tester,
          () => find.byKey(const Key('plan-dialog')).evaluate().isNotEmpty,
          "the host's plan",
        );
        expect(
          find.textContaining('A package of scripts is code the host runs'),
          findsOneWidget,
          reason: 'the notice says it is code',
        );
        expect(
          Directory('${remote.state}/extensions/team_notes').existsSync(),
          isFalse,
          reason: 'nothing is kept before the person confirms',
        );

        await tester.tap(find.byKey(const Key('plan-confirm')));
        await until(
          tester,
          () => find
              .byKey(const Key('extension-store-restart-dialog'))
              .evaluate()
              .isNotEmpty,
          'the install to end',
        );
        expect(find.byKey(const Key('extension-store-error')), findsNothing);
        expect(
          saved,
          1,
          reason: 'canoa.cfg changed through the host: the app reads it again',
        );
        await tester.tap(
          find.byKey(const Key('extension-store-restart-later')),
        );
        await settle(tester);
        await until(
          tester,
          () => find
              .byKey(const Key('store-entry-remove-team-team_notes'))
              .evaluate()
              .isNotEmpty,
          'the package to be named in canoa.cfg',
        );

        // The remote host kept the package, in its own folder, as the files and the digest the catalog named; the near one was
        // not touched, and neither host wrote the scripts that ship in it anywhere but its own state directory.
        final kept = Directory('${remote.state}/extensions/team_notes');
        expect(kept.existsSync(), isTrue);
        expect(
          File('${kept.path}/team_notes.guatiao').existsSync() &&
              File('${kept.path}/sdk/store.js').existsSync(),
          isTrue,
        );
        expect(
          Directory('${near.state}/extensions').existsSync(),
          isFalse,
          reason: 'the host beside the app was not installed on',
        );
        final recorded = File(remote.config.path).readAsStringSync();
        expect(
          recorded,
          contains('[host.extensions.team_notes]'),
          reason: 'the host wrote its own canoa.cfg',
        );
        expect(recorded, contains('bundled:js'));
        expect(recorded, contains(pin));
        expect(
          File(near.config.path).readAsStringSync(),
          isNot(contains('team_notes')),
        );
        for (final host in [near, remote]) {
          expect(
            Directory('${host.state}/bundled').existsSync(),
            isTrue,
            reason: 'the scripts that ship in a host are kept in its own home',
          );
        }

        // A host loads its extensions when it starts: the page asks the remote one to restart, as the app does, its supervisor (this
        // test) starts it again, and it then lists the package's type, which the near one does not. The answer is waited for on the
        // clock the widgets run on, since the page's link was made there: a call on that link from outside it is not delivered
        // until that clock next runs.
        Object? refused;
        var answered = false;
        unawaited(
          HostFiles(remoteControl).restart().then(
            (_) => answered = true,
            onError: (Object error) => refused = error,
          ),
        );
        await until(
          tester,
          () => answered || refused != null,
          'the host to answer the restart',
        );
        expect(
          refused,
          isNull,
          reason: 'a supervised host restarts when asked',
        );
        await tester.runAsync(remote.supervise);
        // The link ends with the host, which the page takes as a closed link; its next request dials the host again and finds it up.
        await settle(tester);
        await tester.runAsync(() => HostFiles(remoteControl).awaitHost());
        final again = control(remote);
        final after = StoreExtensions(
          control: () => again.ready(wait: false),
          open: again.open,
        );
        addTearDown(after.dispose);
        await tester.runAsync(() => after.ensure());
        expect(
          after.types.map((each) => each.id),
          contains('notes'),
          reason:
              "the package's type is in the Store of the host it was installed on",
        );
        expect(after.addedBy['team_notes'], ['Notes']);
        final nearControl = control(near);
        final beside = StoreExtensions(
          control: () => nearControl.ready(wait: false),
          open: nearControl.open,
        );
        addTearDown(beside.dispose);
        await tester.runAsync(() => beside.ensure());
        expect(beside.types.map((each) => each.id), isNot(contains('notes')));
        expect(beside.addedBy.keys, isNot(contains('team_notes')));
      } finally {
        // A link is closed with real input and output, which a test's own clock never lets finish.
        await tester.runAsync(() async {
          for (final each in opened) {
            await each.close().timeout(
              const Duration(seconds: 5),
              onTimeout: () {},
            );
          }
          opened.clear();
        });
        // What the app left timed, such as a link's settling, runs out before the test ends.
        await finish(tester);
      }
    },
    skip: skip != null,
  );
}
