// The Hosts section: this computer first, then the connections saved in canoa.cfg, each with its kind and where it stands; adding,
// renaming and removing one; what a direct endpoint does not offer; what removing names; and what a search nearby fills in.
import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/shell/hosts/host_status.dart';
import 'package:canoa/shell/hosts/hosts_controller.dart';
import 'package:canoa/shell/hosts/hosts_section.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

final _deskId = 'sha256:${'a' * 64}';
final _agentId = 'sha256:${'b' * 64}';
final _desk = Connection(name: 'Desk', kind: ConnectionKind.canoa, address: 'https://desk.example:7800/', peer: _deskId, method: 'noise');
final _agent = Connection(name: 'One agent', kind: ConnectionKind.acp, address: 'wss://agent.example/acp', peer: _agentId, method: 'webcrypto');

HostStatus _online(Connection connection) => HostStatus(
  HostState.online,
  peer: connection.peer,
  method: connection.method,
  agents: const [HostAgent('Claude', 'wss://desk.example:7801/claude')],
  capabilities: const ['Management (sessions, folders, files)'],
);

String _listed({String? peer}) => jsonEncode({
  'peer': peer ?? _deskId,
  'method': 'noise',
  'entries': [
    {'name': 'claude', 'kind': 'agent', 'url': 'wss://desk.example:7801/claude', 'title': 'Claude for review'},
    {'name': 'control', 'kind': 'control', 'url': 'wss://desk.example:7800/control'},
  ],
});

const _local = HostSummary(
  state: 'Running',
  peer: 'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
  method: 'noise',
  agents: ['Claude', 'Codex'],
  capabilities: ['Session list', 'Folder browsing', 'Git worktrees'],
);

/// The Hosts section as a window shows it: the index at the left and the overview beside it.
Future<(HostsController, List<CanoaConfig>)> _show(
  WidgetTester tester, {
  CanoaConfig config = const CanoaConfig(),
  ConnectionProbe? probe,
  HostSummary? local = _local,
  String? servedName,
  bool canSave = true,
  Size size = const Size(1100, 800),
  bool overviewOnly = false,
  List<ChatSession> sessions = const [],
  bool Function(HostItem item)? hasShell,
  void Function(HostItem item)? openShell,
  void Function(String id)? onHostGone,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final saved = <CanoaConfig>[];
  final controller = HostsController(
    layout: ShellLayout(backend: MemoryLayoutBackend()),
    save: (changed) async => saved.add(changed),
    sessions: () => sessions,
    probe: probe ?? (connection) async => _online(connection),
    local: local == null ? null : () => local,
    servedName: servedName,
    servedSummary: servedName == null ? null : () => const HostSummary(state: 'Serving this page', agents: ['Claude'], capabilities: ['Session list']),
    mayWrite: () => canSave,
    hasShell: hasShell,
    openShell: openShell,
    onHostGone: onHostGone,
  )..adopt(config);
  addTearDown(controller.dispose);
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: overviewOnly
          ? HostsOverview(controller: controller, localDetail: (_) => const Text('Agents this computer hosts'))
          : Row(children: [
              SizedBox(width: 300, child: HostsIndex(controller: controller)),
              Expanded(child: HostsOverview(controller: controller, localDetail: (_) => const Text('Agents this computer hosts'))),
            ]),
    ),
  ));
  await tester.pumpAndSettle();
  return (controller, saved);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/hosts_test.dart')));

  group('[connections.NAME] in canoa.cfg', () {
    test('survives a round trip, with a name that needs quoting, and holds no secret', () {
      final config = const CanoaConfig()
          .withConnection(Connection(name: 'Work "laptop"', kind: ConnectionKind.canoa, address: 'https://desk:7800/', peer: _deskId, method: 'noise'))
          .withConnection(_agent);
      final text = config.encode();
      final read = CanoaConfig.parse(text);
      expect([for (final connection in read.connections) connection.name], ['One agent', 'Work "laptop"']);
      final laptop = read.connections.last;
      expect((laptop.kind, laptop.address, laptop.peer, laptop.method), (ConnectionKind.canoa, 'https://desk:7800/', _deskId, 'noise'));
      expect(text, isNot(contains('password')));
      expect(text, isNot(contains('secret')));
    });

    test('a direct WebTransport endpoint keeps its transport, and a WebSocket one writes none', () {
      final webTransport = Connection(
        name: 'Fast', kind: ConnectionKind.acp, address: 'https://agent.example:4433/acp', peer: _agentId, method: 'noise',
        transport: ConnectionTransport.webtransport,
      );
      final text = const CanoaConfig().withConnection(webTransport).withConnection(_agent).encode();
      final read = CanoaConfig.parse(text);
      expect(read.connections.firstWhere((connection) => connection.name == 'Fast').transport, ConnectionTransport.webtransport);
      expect(read.connections.firstWhere((connection) => connection.name == 'One agent').transport, ConnectionTransport.websocket);
      expect(RegExp(r'^transport =', multiLine: true).allMatches(text).length, 1);
    });

    test('is refused when it is not right, and an address with a password is never saved', () {
      String table(String body) => '[connections."X"]\n$body';
      final peer = 'peer = "$_deskId"\nmethod = "noise"\n';
      expect(() => CanoaConfig.parse(table('kind = "other"\naddress = "https://a:1/"\n$peer')), throwsFormatException);
      expect(() => CanoaConfig.parse(table('kind = "canoa"\naddress = "ftp://a/"\n$peer')), throwsFormatException);
      expect(() => CanoaConfig.parse(table('kind = "acp"\naddress = "https://a:1/"\n$peer')), throwsFormatException,
          reason: 'a direct agent by WebSocket is ws or wss');
      expect(() => CanoaConfig.parse(table('kind = "canoa"\naddress = "https://user:pw@a:1/"\n$peer')), throwsFormatException);
      expect(() => CanoaConfig.parse(table('kind = "canoa"\naddress = "https://a:1/"\npeer = "sha256:short"\nmethod = "noise"\n')), throwsFormatException);
      expect(() => CanoaConfig.parse(table('kind = "canoa"\naddress = "https://a:1/"\npeer = "$_deskId"\nmethod = "rsa"\n')), throwsFormatException);
      expect(() => CanoaConfig.parse(table('kind = "canoa"\naddress = "https://a:1/"\n${peer}key = "x"\n')), throwsFormatException,
          reason: 'there is no place for a key');
    });

    test('a name is saved once', () {
      final config = const CanoaConfig().withConnection(_desk);
      expect(() => config.withConnection(_desk), throwsFormatException);
      expect(config.withConnection(_desk.renamed('Desk 2'), replacing: 'Desk').connections.single.name, 'Desk 2');
      expect(config.withoutConnection('Desk').connections, isEmpty);
    });
  });

  group('a look at a connection', () {
    http.Client listing(String body, {int status = 200}) => MockClient((request) async {
      expect(request.url, Uri.parse('https://desk.example:7800/canoa.json'));
      return http.Response(body, status);
    });

    test('a Canoa host is read from its list: its agents, and a control channel only when it lists one', () async {
      final found = await probeConnection(_desk, client: listing(_listed()));
      expect(found.state, HostState.online);
      expect([for (final agent in found.agents) agent.name], ['Claude for review']);
      expect(found.capabilities, ['Management (sessions, folders, files)']);
    });

    test('a list with no control channel claims no capability', () async {
      final bare = jsonEncode({'peer': _deskId, 'method': 'noise', 'entries': [{'name': 'a', 'kind': 'agent', 'url': 'wss://desk.example:1/a'}]});
      final found = await probeConnection(_desk, client: listing(bare));
      expect(found.capabilities, isEmpty);
    });

    test('a host that says another id is not the host that was saved', () async {
      final found = await probeConnection(_desk, client: listing(_listed(peer: 'sha256:${'d' * 64}')));
      expect(found.state, HostState.changed);
      expect(found.agents, isEmpty, reason: 'nothing it lists is trusted');
    });

    test('a host that does not answer, answers elsewhere or is not a Canoa host is not reachable', () async {
      expect((await probeConnection(_desk, client: listing('missing', status: 404))).state, HostState.offline);
      expect((await probeConnection(_desk, client: listing('<html>'))).state, HostState.offline);
      final failing = MockClient((request) async => throw http.ClientException('refused'));
      expect((await probeConnection(_desk, client: failing)).state, HostState.offline);
    });
  });

  group('the section', () {
    testWidgets('Local is first, then each saved connection with its kind and where it stands', (tester) async {
      final (controller, _) = await _show(tester, config: const CanoaConfig().withConnection(_agent).withConnection(_desk));
      expect([for (final item in controller.items) item.name], ['Local', 'Desk', 'One agent']);
      final rows = [for (final item in controller.items) tester.getTopLeft(find.byKey(ValueKey('host-row ${item.id}'))).dy];
      expect(rows, orderedEquals([...rows]..sort()));
      expect(find.text('This computer'), findsOneWidget);
      expect(find.text('Canoa host'), findsOneWidget);
      expect(find.text('Direct ACP endpoint'), findsOneWidget);
      expect(controller.statusOf(controller.items[1]).state, HostState.online, reason: 'a Canoa host is read when the section opens');
      expect(controller.statusOf(controller.items[2]).state, HostState.unchecked, reason: 'a direct agent is connected to only when asked');
      expect(find.byTooltip('Online'), findsOneWidget);
      expect(find.byTooltip('Not checked'), findsOneWidget);
    });

    testWidgets('Local\'s overview is this computer\'s host: state, id, served agents, what it grants, and Hosting', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_desk));
      expect(find.byKey(const ValueKey('host-state')), findsOneWidget);
      expect(find.text('Running'), findsOneWidget);
      expect(find.textContaining('sha256:cccc'), findsOneWidget);
      expect(find.byKey(const ValueKey('host-agents')), findsOneWidget);
      expect(find.text('Codex'), findsOneWidget);
      expect(find.text('Folder browsing'), findsOneWidget);
      expect(find.text('Agents this computer hosts'), findsOneWidget, reason: 'Hosting is shown inside it');
    });

    testWidgets('a saved Canoa host shows its id as pinned, the agents it lists and what it says it can do', (tester) async {
      final (controller, _) = await _show(tester, config: const CanoaConfig().withConnection(_desk));
      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      expect(controller.selected?.name, 'Desk');
      expect(find.text('Online'), findsOneWidget);
      expect(find.textContaining(_deskId), findsOneWidget);
      expect(find.textContaining('Pinned.'), findsOneWidget);
      expect(find.text('Claude'), findsOneWidget);
      expect(find.text('Management (sessions, folders, files)'), findsOneWidget);
    });

    testWidgets('a direct ACP endpoint shows no folder browsing, files or management, and is not checked until asked', (tester) async {
      final probed = <String>[];
      final (controller, _) = await _show(
        tester,
        config: const CanoaConfig().withConnection(_agent),
        probe: (connection) async {
          probed.add(connection.name);
          return _online(connection);
        },
      );
      expect(probed, isEmpty, reason: 'opening the section connects to no agent');
      await tester.tap(find.byKey(const ValueKey('host-row connection:One agent')));
      await tester.pumpAndSettle();
      final capabilities = find.byKey(const ValueKey('host-capabilities'));
      expect(find.descendant(of: find.byType(ListView), matching: find.textContaining('no folder browsing, files or management')), findsOneWidget);
      expect(capabilities, findsOneWidget);
      expect(find.text('Folder browsing'), findsNothing);
      expect(find.text('Browse'), findsNothing);
      expect(find.textContaining('lists none'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('host-reconnect')));
      await tester.pumpAndSettle();
      expect(probed, ['One agent']);
      expect(controller.statusOf(controller.selected!).state, HostState.online);
    });

    testWidgets('a host that says another id says so, and is not shown as online', (tester) async {
      await _show(
        tester,
        config: const CanoaConfig().withConnection(_desk),
        probe: (connection) async => const HostStatus(HostState.changed, detail: 'It says its id is something else.', peer: 'sha256:other'),
      );
      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      expect(find.text('Another key'), findsOneWidget);
      expect(find.text('Online'), findsNothing);
      expect(find.textContaining('will not connect'), findsOneWidget);
    });

    testWidgets('adding, renaming and removing write canoa.cfg and read it back', (tester) async {
      final (controller, saved) = await _show(tester);
      await controller.add(_desk);
      await tester.pumpAndSettle();
      expect(CanoaConfig.parse(saved.last.encode()).connections.single.name, 'Desk');

      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('host-rename')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('rename-field')), 'Desk upstairs');
      await tester.tap(find.byKey(const ValueKey('rename-save')));
      await tester.pumpAndSettle();
      var read = CanoaConfig.parse(saved.last.encode());
      expect([for (final connection in read.connections) connection.name], ['Desk upstairs']);
      expect(read.connections.single.peer, _deskId, reason: 'a rename changes the name and nothing else');
      expect(controller.selected?.name, 'Desk upstairs', reason: 'the row stays chosen under its new name');

      await tester.tap(find.byKey(const ValueKey('host-remove')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('remove-confirm')));
      await tester.pumpAndSettle();
      read = CanoaConfig.parse(saved.last.encode());
      expect(read.connections, isEmpty);
      expect(controller.selected?.name, 'Local');
    });

    testWidgets('a name taken is refused in the rename dialog', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_desk).withConnection(_agent));
      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('host-rename')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('rename-field')), 'One agent');
      await tester.tap(find.byKey(const ValueKey('rename-save')));
      await tester.pumpAndSettle();
      expect(find.text('A connection named One agent is already saved.'), findsOneWidget);
    });

    testWidgets('a row with a shell to offer has an Open a shell button that names its host; one without has none', (tester) async {
      final opened = <String>[];
      await _show(
        tester,
        config: const CanoaConfig().withConnection(_desk).withConnection(_agent),
        hasShell: (item) => item.id != 'connection:One agent',
        openShell: (item) => opened.add(item.id),
      );
      expect(find.byKey(const ValueKey('host-shell local')), findsOneWidget);
      expect(find.byKey(const ValueKey('host-shell connection:Desk')), findsOneWidget);
      expect(find.byKey(const ValueKey('host-shell connection:One agent')), findsNothing, reason: 'a direct endpoint offers no shell');
      expect(find.byTooltip('Open a shell on Desk'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('host-shell connection:Desk')));
      await tester.pumpAndSettle();
      expect(opened, ['connection:Desk'], reason: 'the button does not also choose the row');
    });

    testWidgets('no shortcut where the app has no Terminal section', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_desk));
      expect(find.byTooltip('Open a shell on Desk'), findsNothing);
    });

    testWidgets('removing or renaming a host says its id is gone, so its shells can end', (tester) async {
      final gone = <String>[];
      final (controller, _) = await _show(tester, config: const CanoaConfig().withConnection(_desk), onHostGone: gone.add);
      await controller.rename(_desk, 'Desk upstairs');
      expect(gone, ['connection:Desk']);
      await controller.remove(controller.config.connections.single);
      expect(gone, ['connection:Desk', 'connection:Desk upstairs']);
    });

    testWidgets('removing one names the sessions on it, and says nothing remote is stopped', (tester) async {
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      addTearDown(store.dispose);
      store.addRemoteSession(
        title: 'Review the diff', agent: 'a', peerId: _agentId, remoteSessionId: 'r1', endpoint: Uri.parse('wss://agent.example/acp'),
        place: SessionPlace.peer, method: 'webcrypto',
      );
      store.addRemoteSession(
        title: 'Elsewhere', agent: 'a', peerId: _deskId, remoteSessionId: 'r2', endpoint: Uri.parse('wss://desk.example:7801/claude'),
        place: SessionPlace.peer, method: 'noise',
      );
      final (controller, saved) = await _show(tester, config: const CanoaConfig().withConnection(_agent), sessions: store.sessions);
      await tester.tap(find.byKey(const ValueKey('host-row connection:One agent')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('host-remove')));
      await tester.pumpAndSettle();
      final listed = find.byKey(const ValueKey('remove-sessions'));
      expect(find.descendant(of: listed, matching: find.text('Review the diff')), findsOneWidget);
      expect(find.descendant(of: listed, matching: find.text('Elsewhere')), findsNothing, reason: 'only the sessions on this connection');
      expect(find.textContaining('Nothing is stopped on the other end'), findsOneWidget);
      expect(saved, isEmpty, reason: 'nothing is removed until it is confirmed');
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(controller.config.connections, hasLength(1));
    });

    testWidgets('on a page the host served, the serving host is named as such and is never Local', (tester) async {
      final (controller, _) = await _show(tester, local: null, servedName: 'desk.example:7800', canSave: false);
      expect([for (final item in controller.items) item.name], ['desk.example:7800']);
      expect(find.text('Local'), findsNothing);
      expect(find.text('Serves this page'), findsOneWidget);
      expect(find.textContaining('it is not Local'), findsOneWidget);
      expect(tester.widget<IconButton>(find.byKey(const ValueKey('hosts-add'))).onPressed, isNull, reason: 'its host has not given this key config.write');
    });

    testWidgets('on a page the host served with config.write, a connection is added, renamed and removed', (tester) async {
      final (controller, saved) = await _show(tester, local: null, servedName: 'desk.example:7800');
      expect(tester.widget<IconButton>(find.byKey(const ValueKey('hosts-add'))).onPressed, isNotNull);
      await controller.add(_desk);
      await tester.pumpAndSettle();
      expect(CanoaConfig.parse(saved.last.encode()).connections.single.name, 'Desk');
      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('host-rename')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('rename-field')), 'Desk upstairs');
      await tester.tap(find.byKey(const ValueKey('rename-save')));
      await tester.pumpAndSettle();
      expect([for (final connection in CanoaConfig.parse(saved.last.encode()).connections) connection.name], ['Desk upstairs']);
      await tester.tap(find.byKey(const ValueKey('host-remove')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('remove-confirm')));
      await tester.pumpAndSettle();
      expect(CanoaConfig.parse(saved.last.encode()).connections, isEmpty);
      expect(controller.items.map((item) => item.name), ['desk.example:7800'], reason: 'the serving host stays listed');
    });

    testWidgets('a page without config.write is refused in the controller too', (tester) async {
      final (controller, saved) = await _show(tester, local: null, servedName: 'desk.example:7800', canSave: false);
      await expectLater(controller.add(_desk), throwsA(isA<FormatException>()));
      expect(saved, isEmpty);
    });

    testWidgets('with nothing listed it says so and offers Add', (tester) async {
      await _show(tester, local: null);
      expect(find.byKey(const ValueKey('host-empty')), findsOneWidget);
      expect(find.text('Add connection'), findsOneWidget);
    });
  });

  group('looking nearby, through the host', () {
    test('reads each candidate as a claim, with the reason a search could not run', () async {
      final link = _Link((method, params) {
        expect(method, 'discover/nearby');
        expect(params['seconds'], 3);
        return {
          'candidates': [
            {'label': 'Claude on the desk', 'url': 'ws://192.168.1.5:7801', 'peer': _agentId, 'method': 'noise', 'offers': ['agent']},
            {'label': 'no address'},
          ],
          'networkError': 'no multicast here',
        };
      });
      final found = await nearbyOf(ControlClient(link));
      expect([for (final candidate in found.candidates) (candidate.label, candidate.url, candidate.peer, candidate.method)], [
        ('Claude on the desk', 'ws://192.168.1.5:7801', _agentId, 'noise'),
      ]);
      expect(found.problem, contains('no multicast here'));
    });

    test('a host that does not grant it says so', () async {
      final link = _Link((method, params) => throw const ControlFailed(-32001, 'no discover grant'));
      expect((await nearbyOf(ControlClient(link))).problem, 'This host does not let this app look nearby.');
    });
  });

  group('in the app', () {
    testWidgets('the selector offers Hosts, which holds what Connect and Hosting were', (tester) async {
      final written = await pumpApp(
        tester,
        SessionStore(),
        config: const CanoaConfig().withConnection(_desk),
        probe: (connection) async => _online(connection),
      );
      expect(find.byTooltip('Hosts'), findsOneWidget);
      expect(find.text('Connect'), findsNothing);
      await tester.tap(find.byTooltip('Hosts'));
      await frames(tester);
      expect(find.byKey(const ValueKey('host-row connection:Desk')), findsOneWidget);
      expect(find.text('Local'), findsWidgets, reason: 'the app\'s own host is Local, listed first');
      expect(written, isEmpty);

      await tester.tap(find.byKey(const ValueKey('hosts-add')));
      await frames(tester);
      expect(find.text('Add connection'), findsWidgets);
    });
  });

  group('goldens', () {
    testWidgets('Local at desktop width', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_agent).withConnection(_desk));
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/hosts_local_desktop.png'));
    });

    testWidgets('a saved Canoa host at desktop width', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_agent).withConnection(_desk));
      await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
      await tester.pumpAndSettle();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/hosts_connection_desktop.png'));
    });

    testWidgets('a direct endpoint at desktop width', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_agent).withConnection(_desk));
      await tester.tap(find.byKey(const ValueKey('host-row connection:One agent')));
      await tester.pumpAndSettle();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/hosts_endpoint_desktop.png'));
    });

    testWidgets('Local at phone width', (tester) async {
      await _show(tester, config: const CanoaConfig().withConnection(_desk), size: const Size(390, 844), overviewOnly: true);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/hosts_local_phone.png'));
    });
  });
}

/// A control link that answers each request from [answer], or fails it with the [ControlFailed] it throws.
final class _Link implements HostLink {
  _Link(this._answer);

  final Object? Function(String method, Map<String, Object?> params) _answer;
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    try {
      final result = _answer(request['method']! as String, (request['params'] as Map?)?.cast<String, Object?>() ?? const {});
      _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
    } on ControlFailed catch (error) {
      _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': error.code, 'message': error.message}}));
    }
  }

  @override
  Future<void> close() => _out.close();
}
