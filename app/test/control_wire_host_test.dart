// The app's reading of three control answers, against the real host: `settings/sections`, `accounts/catalog` and `store/models`. A
// real `canoa serve` runs in a folder of this test, with a home, a data folder and vendor login folders of its own, and a control
// channel this page's key is granted; the app's own code (HostSections, ControlAccountsHost, TypeModels) reads what the host
// answers, so a field the host renames or reshapes fails here and not in a hand-written fixture.
//
// The page's side of the key proof is the web_peer example, since the Dart VM has no WebCrypto. CANOA_TEST_HOST names a built
// canoa, with its examples (stdio_agent, web_peer, foundry_standin) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them; CANOA_TEST_METHOD (`noise` or `webcrypto`) is the method the host
// proves by, `noise` unless set. Without CANOA_TEST_HOST the tests are skipped.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/acp/client.dart' show VerifiedPeer;
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/attention/host_notices.dart';
import 'package:canoa/attention/notifications_host.dart';
import 'package:canoa/attention/notifier.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool_io.dart' show freeLoopbackPort;
import 'package:canoa/served/served_control.dart';
import 'package:canoa/settings/advanced_config.dart';
import 'package:canoa/settings/host_sections.dart';
import 'package:canoa/store/store_models.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/served_process.dart';
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

/// What this page's key may do on the host: read its configuration and its Store, run a Store type's program, manage accounts, and
/// edit the host's own tables (`hosting`).
const _grants = ['config.read', 'config.write', 'store.read', 'store.programs', 'accounts', 'hosting', 'notify'];

/// The model the stand-in for the Foundry Local program lists.
const _model = 'qwen3-14b-generic-gpu:2';

void main() {
  final skip = _hostBinary == null
      ? 'CANOA_TEST_HOST names a built canoa'
      : exampleMissing('web_peer') ?? exampleMissing('stdio_agent') ?? exampleMissing('foundry_standin');
  late Directory dir;
  late WebPeerBridge browser;
  late PeerMethod method;
  late ServedProcess host;
  late File config;
  late HostControl control;

  Future<VerifiedPeer> prove(channel, expected) => provePeer(browser, channel, expected, method: method);

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-control-wire-');
    final root = dir.path.replaceAll(r'\', '/');
    for (final folder in ['host/cfg', 'host/home/.foundry', 'host/data', 'host/bin', 'host/claude', 'host/codex', 'host/pi']) {
      Directory('$root/$folder').createSync(recursive: true);
    }
    // The program of the Foundry Local type, found on the only PATH the host has, under its own name.
    File(exampleBinary('foundry_standin')!).copySync('$root/host/bin/foundry${Platform.isWindows ? '.exe' : ''}');
    method = PeerMethod.parse(Platform.environment['CANOA_TEST_METHOD'] ?? 'noise');
    browser = await WebPeerBridge.start();
    final browserId = await browser.localId(method);
    config = File('$root/host/cfg/canoa.cfg')
      ..writeAsStringSync('''
[host.control]
listen = "127.0.0.1:${await freeLoopbackPort()}"
grants = { ${jsonEncode(browserId)} = ${jsonEncode(_grants)} }

[host.agents.scripted]
program = ${jsonEncode(exampleBinary('stdio_agent'))}
args = ["--scripted"]
listen = "127.0.0.1:${await freeLoopbackPort()}"
peers = [${jsonEncode(browserId)}]
''');
    host = ServedProcess(
      binary: _hostBinary!,
      dir: Directory('$root/host'),
      config: config,
      method: method.id,
      // The vendors' own login folders and the data folder are this test's, so no login of the person's is read and no program
      // of theirs is found.
      environment: {
        'CLAUDE_CONFIG_DIR': '$root/host/claude',
        'CODEX_HOME': '$root/host/codex',
        'PI_CODING_AGENT_DIR': '$root/host/pi',
        'LOCALAPPDATA': '$root/host/data',
        'XDG_DATA_HOME': '$root/host/data',
        'PATH': '$root/host/bin${Platform.isWindows ? ';' : ':'}${Platform.environment['SystemRoot'] ?? '/usr/bin'}',
      },
    );
    await host.start();
    control = servedControl(ServedHost.parse(host.listing(method))!, verify: prove);
    // The page's key is turned away until the person accepts it on the host.
    await expectLater(control.ready(wait: false).then((client) => client.call('settings/sections', const {})), throwsA(anything));
    final accepted = await host.command(['accept', browserId]);
    expect(accepted.exitCode, 0, reason: '${accepted.stderr}');
  });

  tearDownAll(() async {
    if (skip != null) return;
    await control.close();
    browser.kill();
    await host.stop();
    for (var tries = 0; tries < 10; tries++) {
      try {
        dir.deleteSync(recursive: true);
        break;
      } on FileSystemException {
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
  });

  group('the real host wire', skip: skip, () {
  test('the sections the host lists are the pages the app draws itself, as the app reads them', () async {
    final sections = HostSections((method, params) async => (await control.ready(wait: false)).call(method, params));
    // Until it adopts the host's answer, the sections are the app's own. `load` keeps them when the host does not answer, so the
    // answer is asked for here, where a refusal fails the test.
    final before = sections.sections;
    expect(before, isNotEmpty);
    final answer = await (await control.ready(wait: false)).call('settings/sections', const {});
    expect(answer, isA<Map>().having((each) => each['sections'], 'sections', isNotEmpty));
    sections.adopt(answer);
    final ids = [for (final section in sections.sections) section.id];
    expect(ids.toSet(), hasLength(ids.length), reason: 'no page twice');
    expect(ids, containsAll([for (final section in HostSections.builtIn()) section.id]), reason: 'the host lists every page the app draws itself: $ids');
    for (final section in sections.sections) {
      expect(section.title, isNotEmpty, reason: section.id);
    }
    // The app's own listing is what the host lists of them: every field the app reads is the same.
    final listed = {for (final section in sections.sections) section.id: section};
    for (final own in before) {
      final host = listed[own.id]!;
      expect((host.title, host.origin, host.stored, host.under, host.group, host.icon, host.keywords, host.help),
          (own.title, own.origin, own.stored, own.under, own.group, own.icon, own.keywords, own.help),
          reason: own.id);
      expect(host.schema, own.schema, reason: '${own.id} schema');
      expect(host.form, own.form, reason: '${own.id} form');
    }
  });

  test("an account provider's catalog names its kinds and the ways to sign in, as the app's dialog reads them", () async {
    final accounts = ControlAccountsHost(control);
    final catalog = await accounts.catalog('claude_accounts');
    expect(catalog.provider, 'claude_accounts');
    expect(catalog.kinds, isNotEmpty);
    for (final kind in catalog.kinds) {
      expect(kind.id, isNotEmpty);
    }
    expect(catalog.logins, isNotEmpty, reason: 'a way to sign in');
    for (final login in catalog.logins) {
      expect(login.id, isNotEmpty);
    }
    await expectLater(accounts.catalog('nobody'), throwsA(anything), reason: 'a provider the host has none of');
  });

  test("a Store type's models are listed by the library of the type, with where each stands, what + does and the settings form", () async {
    final models = TypeModels(
      type: 'foundry',
      title: 'Models',
      control: () => control.ready(wait: false),
      open: control.open,
    );
    addTearDown(models.dispose);
    await models.refresh();
    expect(models.problem, isNull);
    expect(models.unavailable, isNull);
    expect(models.catalog, isNotEmpty);
    final listed = [for (final model in models.models) model.id];
    expect(listed, contains(_model));
    final row = models.models.firstWhere((model) => model.id == _model);
    expect(row.summary, isNotEmpty);
    expect(row.actions, isNotEmpty, reason: 'a model that is not downloaded can be');
    expect(row.actions.first.steps, isNotEmpty);
    expect(models.settings, isNotNull, reason: 'the type keeps a setting of a model');
    expect(models.settings!.schema, isNotEmpty);
  });

  test("the host's events and notifiers are read as the Notifications page reads them, a route is written through the host, and a notice is pushed", () async {
    final host = ControlNotificationsHost(control);
    final listed = await host.list();
    expect(listed.events.map((each) => each.id), ['permission', 'question', 'finished', 'failed', 'usage_warn', 'usage_stop', 'update_failed', 'host_restarted', 'host_updated']);
    expect(listed.events.firstWhere((each) => each.id == 'finished').notifiers, ['desktop']);
    expect(listed.events.firstWhere((each) => each.id == 'host_restarted').notifiers, isEmpty);
    expect(listed.notifiers.first.name, 'desktop');
    expect(listed.notifiers.first.removable, isFalse);

    await host.route('finished', const []);
    expect((await host.list()).events.firstWhere((each) => each.id == 'finished').notifiers, isEmpty);
    expect(config.readAsStringSync(), contains('finished = []'));
    await expectLater(host.route('finished', ['nobody']), throwsA(anything), reason: 'a name that is no notifier is refused by the host');
    await host.route('finished', ['desktop']);

    final shown = <String>[];
    final notifier = _Shown(shown);
    final notices = HostNotices(control: control, notifier: notifier, language: () => 'en', localSession: (_) => null);
    notices.start();
    await Future<void>.delayed(const Duration(milliseconds: 500));
    final result = await host.test('desktop');
    expect(result.ok, isTrue, reason: '${result.why}');
    await Future<void>.delayed(const Duration(milliseconds: 500));
    expect(shown, ['Canoa: Test']);
    await notices.stop();
  });

  test("the Advanced page's tables are listed, added, changed and removed through the host, and a refusal is the host's words", () async {
    final advanced = AdvancedConfig((method, params) async => (await control.ready(wait: false)).call(method, params));
    await advanced.load();
    expect(advanced.problem, isNull);
    AdvancedTable table(String id) => advanced.tables.firstWhere((each) => each.id == id);
    expect(advanced.tables.map((each) => each.id), containsAll(['host.mcp_list', 'host.tools', 'host.openers', 'harnesses']));
    expect(table('host.mcp_list').shape, AdvancedShape.flat);
    expect(table('host.mcp_list').schema, containsPair('properties', contains('query_seconds')));
    expect(table('host.mcp_list').own, isNull);

    // A table of settings: made, changed, and taken out; the rest of the file stays.
    expect(await advanced.set(['host', 'mcp_list'], {'query_seconds': 7}), isNull);
    expect(config.readAsStringSync(), allOf(contains('query_seconds = 7'), contains('[host.agents.scripted]')));
    expect(table('host.mcp_list').own, {'query_seconds': 7});
    expect(await advanced.set(['host', 'mcp_list'], {'query_seconds': 9, 'max_items_per_server': 50}), isNull);
    expect(table('host.mcp_list').own, {'query_seconds': 9, 'max_items_per_server': 50});

    // A table of names, each one text.
    expect(await advanced.set(['harnesses', 'my-acp'], 'My Harness'), isNull);
    expect(config.readAsStringSync(), contains('My Harness'));
    expect(table('harnesses').ownEntries['my-acp'], {'value': 'My Harness'});

    // What the host would not read is refused in its words, and nothing is written.
    final kept = config.readAsStringSync();
    final refused = await advanced.set(['host', 'mcp_list'], {'query_seconds': 0});
    expect(refused, isNotNull);
    expect(config.readAsStringSync(), kept);
    expect(await advanced.set(['host', 'agents', 'x'], {'program': 'x'}), isNotNull, reason: 'a table another page edits is not offered here');

    expect(await advanced.set(['host', 'mcp_list'], null), isNull);
    expect(await advanced.set(['harnesses', 'my-acp'], null), isNull);
    expect(table('host.mcp_list').own, isNull);
    expect(config.readAsStringSync(), allOf(isNot(contains('mcp_list')), isNot(contains('My Harness')), contains('[host.agents.scripted]')));
  });
  });
}

final class _Shown implements Notifier {
  _Shown(this.shown);

  final List<String> shown;

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async {}
  @override
  Future<bool> get available async => true;
  @override
  Future<bool> requestPermission() async => true;
  @override
  void watchAttention(void Function(bool attended) onChange) {}
  @override
  Future<void> show({required String title, required String body, required String sessionId}) async => shown.add('$title: $body');
}
