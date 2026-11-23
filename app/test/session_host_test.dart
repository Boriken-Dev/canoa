// A new session aimed at another host: the host chip, the folder browser that lists through that host, a path checked by it before
// Send, recent folders kept apart by host, and a chat whose folder that host makes.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/accounts/account_models.dart' show accountMeta;
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/chat/inspector.dart' show machineOf;
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/folder_dialog.dart' show breadcrumbs;
import 'package:canoa/profile/report.dart' show loadoutMeta;
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/workspace/remote_host.dart';
import 'package:coqui_transport_web/coqui_transport_web.dart' show WebEndpoints;
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

final _peer = 'sha256:${'b' * 64}';

/// The control channel of a Linux host with `/srv/app` and `/srv/locked` (which exists and cannot be listed).
final class _Far implements HostLink {
  final _out = StreamController<String>();
  final asked = <String>[];

  static Map<String, Object?> _entry(String name, String path) => {'name': name, 'path': path, 'dir': true};

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method'] as String;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    asked.add('$method ${params['path'] ?? ''}'.trim());
    Object? result;
    Map<String, Object?>? error;
    switch (method) {
      case 'fs/list':
        final path = params['path'] as String?;
        if (path == null) {
          result = {'path': null, 'entries': [_entry('srv', '/srv')]};
        } else if (path == '/srv') {
          result = {'path': '/srv', 'parent': null, 'entries': [_entry('app', '/srv/app'), _entry('locked', '/srv/locked')]};
        } else if (path == '/srv/app') {
          result = {'path': '/srv/app', 'parent': '/srv', 'entries': <Object?>[]};
        } else {
          error = {'code': -32003, 'message': 'permission denied'};
        }
      case 'fs/info':
        result = {
          'folders': [
            for (final path in (params['paths'] as List).cast<String>())
              {'path': path, 'exists': path.startsWith('/srv'), 'folder': path.startsWith('/srv'), 'repository': false},
          ],
        };
      case 'fs/mentions':
        result = {'entries': ['notes.txt', 'src/']};
      case 'fs/resolve':
        final name = params['name'] as String;
        result = {'path': '${params['path']}/$name', 'folder': name.endsWith('/'), 'size': 11};
      case 'fs/scratch':
        result = {'path': '/srv/scratch/${params['session']}', 'made': true};
      case 'fs/scratch_remove':
        result = {'removed': true};
      case 'accounts/providers':
        result = {
          'providers': [
            {'id': 'claude_accounts', 'title': 'Claude', 'harness': 'claude'},
          ],
        };
      case 'accounts/list':
        result = {
          'accounts': [
            for (final name in ['far-work', 'far-other', 'far-key'])
              {'name': name, 'provider': 'claude_accounts', 'agent': 'claude', 'fits': ['claude'], 'kind': 'claude-login', 'id': 'id-$name', 'state': 'signed in'},
          ],
        };
      case 'accounts/entries':
        result = {
          'entries': [
            // far-key is signed in but the agent does not accept what it provides, so the host leaves it out of `usable`.
            {'name': 'Claude', 'harness': 'claude', 'accounts': ['far-work'], 'usable': ['far-work', 'far-other'], 'accepts': ['claude-code']},
          ],
        };
      default:
        error = {'code': -32001, 'message': 'no grant'};
    }
    _out.add(jsonEncode({
      'jsonrpc': '2.0',
      'id': request['id'],
      if (error != null) 'error': error else 'result': result,
    }));
  }

  @override
  Future<void> close() => _out.close();
}

final _far = Connection(
  name: 'Far',
  kind: ConnectionKind.canoa,
  address: 'http://far.test:8080/',
  peer: _peer,
  method: 'noise',
);

RemoteHost _remote(_Far control, {String os = 'linux', List<ServedEntry>? entries, Connection? connection}) => RemoteHost(
  connection ?? _far,
  ServedHost(
    peer: _far.peer,
    method: PeerMethod.noise,
    os: os,
    entries: entries ?? const [ServedEntry(name: 'scripted', kind: 'agent', url: 'wss://far.test:9/scripted')],
    control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://far.test:9/control'),
  ),
  HostControl.dialing(() async => control),
);

const _claudeEntry = ServedEntry(name: 'Claude', kind: 'agent', url: 'wss://far.test:9/Claude', harness: 'claude');

typedef _Reached = ({String url, String peer});

Future<(SessionStore, OptionsAgent, List<_Reached>, _Far, SessionLedger)> _page(
  WidgetTester tester, {
  List<ServedEntry>? entries,
  Connection? connection,
  CanoaConfig? config,
}) async {
  final agent = OptionsAgent();
  final reached = <_Reached>[];
  final far = _Far();
  final ledger = SessionLedger(MemoryLedgerBackend());
  final store = SessionStore(ledger: ledger);
  await pumpApp(
    tester,
    store,
    config: (config ?? const CanoaConfig(agents: [scripted])).withConnection(connection ?? _far),
    pickFolder: () async => '/home/me/local',
    openRemote: (opened) async => _remote(far, entries: entries, connection: connection),
    connectEndpoint: (WebEndpoints endpoints, {required String pinnedPeerId, required PeerMethod method, Duration timeout = const Duration(seconds: 10),
        AgentDecider? decider, AgentElicitor? elicitor, AgentSecrets? secrets, Map<String, Object?> initializeMeta = const {}}) async {
      reached.add((url: endpoints.webSocket.toString(), peer: pinnedPeerId));
      final link = agent.openLink('far');
      final client = VerifiedAgent.fromChild(
        label: 'scripted', messages: link.messages, send: link.send, close: link.close,
        decider: decider, elicitor: elicitor, initializeMeta: initializeMeta,
      );
      await client.initialize();
      return client;
    },
  );
  return (store, agent, reached, far, ledger);
}

Future<void> _aimAtFar(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('new-session-host')));
  await frames(tester);
  await tester.tap(find.byKey(const ValueKey('host-choice Far')));
  await settle(tester);
}

void main() {
  test('breadcrumbs name each folder above a path as its own host writes it', () {
    expect([for (final each in breadcrumbs('/srv/app')) each.path], ['/', '/srv', '/srv/app']);
    expect([for (final each in breadcrumbs(r'C:\work\app')) each.path], [r'C:\', r'C:\work', r'C:\work\app']);
    expect([for (final each in breadcrumbs(r'\\box\share\x')) each.path], [r'\\box', r'\\box\share', r'\\box\share\x']);
  });

  test('a path is judged by the system the host runs, and a host that does not say is given no judgment', () {
    expect(pathFitsSystem('linux', '/srv/app'), isTrue);
    expect(pathFitsSystem('linux', r'C:\work\app'), isFalse);
    expect(pathFitsSystem('linux', r'\\server\share'), isFalse);
    expect(pathFitsSystem('windows', r'C:\work\app'), isTrue);
    expect(pathFitsSystem('windows', '/srv/app'), isFalse);
    expect(pathFitsSystem(null, r'C:\x'), isTrue);
  });

  test('the host refuses a path that is for another system, is not there, or cannot be read, in its own words', () async {
    final host = _remote(_Far());
    expect(await host.problemWith('/srv/app'), isNull);
    expect(await host.problemWith(r'C:\work\app'), contains('not a path on Far'));
    expect(await host.problemWith('/etc/none'), contains('does not exist'));
    expect(await host.problemWith('/srv/locked'), contains('permission denied'));
    host.dispose();
  });

  testWidgets('a draft aimed at another host browses that host\'s folders, is checked there and starts its agent at its address', (tester) async {
    final (store, agent, reached, far, _) = await _page(tester);
    await newSession(tester);
    await chooseWorkspace(tester);
    await _aimAtFar(tester);
    expect(store.selected!.host, 'Far');
    expect(store.selected!.source!.label, 'scripted', reason: 'the agents it offers are the host\'s own');
    await openFolder(tester);
    await settle(tester);
    expect(find.byKey(const Key('host-folder-dialog')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv/app')));
    await settle(tester);
    await tester.tap(find.byKey(const Key('host-folder-choose')));
    await settle(tester);
    expect(store.selected!.cwd, '/srv/app');
    expect(far.asked, containsAll(['fs/list', 'fs/list /srv', 'fs/info']));
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(reached.single.url, 'wss://far.test:9/scripted');
    expect(reached.single.peer, _peer);
    expect(store.selected!.place, SessionPlace.peer);
    expect(store.selected!.cwd, '/srv/app');
    expect(agent.methods, contains('session/prompt'));
    await finish(tester);
  });

  testWidgets('an @ in a session on another host lists that host\'s folder, and a pick goes as a link to a path on that host', (tester) async {
    final (store, agent, _, far, _) = await _page(tester);
    await newSession(tester);
    await chooseWorkspace(tester);
    await _aimAtFar(tester);
    await openFolder(tester);
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv/app')));
    await settle(tester);
    await tester.tap(find.byKey(const Key('host-folder-choose')));
    await settle(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(store.selected!.place, SessionPlace.peer);
    final field = find.byType(TextField).last;
    await tester.enterText(field, 'read @no');
    await settle(tester);
    expect(far.asked, contains('fs/mentions /srv/app'), reason: 'the host lists its own folder');
    await tester.tap(find.byKey(const ValueKey('mention notes.txt')));
    await settle(tester);
    expect(store.selected!.attachments.single.onAgentHost, isTrue);
    final before = agent.methods.where((method) => method == 'session/prompt').length;
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => agent.methods.where((method) => method == 'session/prompt').length > before, 'the agent to have it');
    final prompt = agent.last('session/prompt')['params']['prompt'] as List<dynamic>;
    expect(prompt.map((block) => block['type']), ['text', 'resource_link'], reason: 'the file is on the agent\'s host: it is not read here');
    expect(prompt[1]['uri'], endsWith('/srv/app/notes.txt'));
    await finish(tester);
  });
  testWidgets('an unreadable path is refused before anything starts, and a Windows path is not taken for a Linux host', (tester) async {
    final (store, _, reached, _, _) = await _page(tester);
    await newSession(tester);
    await chooseWorkspace(tester);
    await _aimAtFar(tester);
    await openFolder(tester);
    await settle(tester);
    await tester.tap(find.byKey(const Key('host-folder-edit')));
    await frames(tester);
    await tester.enterText(find.byKey(const Key('host-folder-typed')), r'C:\work\app');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    expect(find.byKey(const Key('host-folder-problem')), findsOneWidget);
    expect(find.textContaining('not a path on Far'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const Key('host-folder-choose'))).onPressed, isNull, reason: 'nothing is chosen at the starting places');
    await tester.enterText(find.byKey(const Key('host-folder-typed')), '/srv/locked');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    expect(find.textContaining('permission denied'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await settle(tester);
    expect(store.selected!.cwd, isNull);
    expect(reached, isEmpty);
    await finish(tester);
  });

  testWidgets('recent folders are kept apart by host', (tester) async {
    final (store, _, _, _, ledger) = await _page(tester);
    ledger.record(peerId: 'local:scripted', sessionId: 'l1', entry: 'scripted', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/home/me/local', title: 'Here');
    ledger.record(peerId: _far.peer, sessionId: 'r1', entry: 'scripted', place: SessionPlace.peer, reopenBy: 'session/load', cwd: '/srv/app', title: 'There');
    ledger.record(peerId: _far.peer, sessionId: 'r2', entry: 'scripted', place: SessionPlace.peer, reopenBy: 'session/load', cwd: r'C:\stale', title: 'Wrong system');
    await ledger.flush();
    await newSession(tester);
    await chooseWorkspace(tester);
    expect(find.byKey(const ValueKey('recent-folder /home/me/local')), findsOneWidget);
    expect(find.byKey(const ValueKey('recent-folder /srv/app')), findsNothing);
    await _aimAtFar(tester);
    expect(find.byKey(const ValueKey('recent-folder /srv/app')), findsOneWidget);
    expect(find.byKey(const ValueKey('recent-folder /home/me/local')), findsNothing);
    expect(find.byKey(const ValueKey(r'recent-folder C:\stale')), findsNothing);
    expect(store.selected!.host, 'Far');
    await finish(tester);
  });

  testWidgets('a chat on another host gets its folder from that host', (tester) async {
    final (store, _, reached, far, _) = await _page(tester);
    await newSession(tester);
    await _aimAtFar(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(far.asked.where((each) => each.startsWith('fs/scratch')), isNotEmpty);
    expect(reached, hasLength(1));
    expect(store.selected!.cwd, startsWith('/srv/scratch/'));
    expect(store.selected!.kind, SessionKind.chat);
    await finish(tester);
  });

  testWidgets('a draft aimed at another host lists that host\'s accounts the agent accepts, and starts on the one chosen with its loadout', (tester) async {
    final (store, agent, reached, far, _) = await _page(
      tester,
      entries: const [_claudeEntry],
      config: CanoaConfig.parse('[loadouts.main]\noverlays = []\n'),
    );
    await newSession(tester);
    await chooseWorkspace(tester);
    await _aimAtFar(tester);
    expect(store.selected!.source!.label, 'Claude');
    expect(far.asked, containsAll(['accounts/list', 'accounts/entries']), reason: 'the chosen host is asked for its accounts');
    store.selected!.loadout = 'main';
    await tester.tap(find.byKey(const Key('option-account')));
    await frames(tester);
    expect(find.byKey(const Key('account-choice far-work')), findsOneWidget);
    expect(find.byKey(const Key('account-choice far-other')), findsOneWidget);
    expect(find.byKey(const Key('account-choice far-key')), findsNothing, reason: 'the host says the agent cannot use it');
    await tester.tap(find.byKey(const Key('account-choice far-other')));
    await frames(tester);
    await closePicker(tester);
    await openFolder(tester);
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('host-folder /srv/app')));
    await settle(tester);
    await tester.tap(find.byKey(const Key('host-folder-choose')));
    await settle(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(reached.single.url, 'wss://far.test:9/Claude');
    final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
    final meta = (initialize['params'] as Map)['_meta'] as Map;
    expect(meta[accountMeta], 'far-other');
    expect(meta[loadoutMeta], 'main');
    await finish(tester);
  });

  testWidgets('a draft that chose no account starts on the first the other host offers', (tester) async {
    final (_, agent, _, _, _) = await _page(tester, entries: const [_claudeEntry]);
    await newSession(tester);
    await _aimAtFar(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
    expect(((initialize['params'] as Map)['_meta'] as Map)[accountMeta], 'far-work');
    await finish(tester);
  });

  test('what a source\'s provider listed is kept per entry and host, and the local key is the entry\'s name', () async {
    const here = AgentSource.hosted('Claude');
    const far = AgentSource.hosted('Claude', host: 'Far');
    const near = AgentSource.hosted('Claude', host: 'Near');
    expect(sourceKey(here), 'Claude');
    expect(sourceKey(far), 'Claude@Far');
    expect({here, far, near}, hasLength(3));
    final store = SessionStore();
    store.choicesLoader = (source) async => SourceChoices(known: true, models: [ModelChoice(id: 'm-${source.host}', name: 'Model of ${source.host}')]);
    for (final source in [here, far, near]) {
      await store.askChoicesAgain(source);
    }
    expect(store.choicesOf(here)!.models.single.id, 'm-null');
    expect(store.choicesOf(far)!.models.single.id, 'm-Far');
    expect(store.choicesOf(near)!.models.single.id, 'm-Near');
    store.dispose();
  });

  test('a conversation reopened from the ledger on a saved host shows the saved name, not its address', () async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final entry = ledger.record(
      peerId: 'local:Scripted', sessionId: 'o1', entry: 'Scripted', place: SessionPlace.peer, reopenBy: 'session/load',
      endpoint: 'wss://far.test:9/scripted', method: 'noise', cwd: '/srv/app', title: 'An earlier one',
    )!;
    final store = SessionStore(ledger: ledger);
    final client = await OptionsAgent().start(scripted);
    final reopened = await store.reopen(entry, client, host: savedNameOfPeer([_far], _far.peer));
    expect(machineOf(reopened).value, 'Far', reason: 'the address is far.test');
    expect(savedNameOfPeer([_far], 'sha256:${'c' * 64}'), isNull);
    expect(savedNameOfPeer([_far], _far.peer.toUpperCase().replaceFirst('SHA256', 'sha256')), 'Far');
    final unsaved = await SessionStore(ledger: ledger).reopen(entry, await OptionsAgent().start(scripted));
    expect(machineOf(unsaved).value, 'far.test', reason: 'a host that is not saved is named by its address');
    store.dispose();
  });
}
