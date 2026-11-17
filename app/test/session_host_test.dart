// A new session aimed at another host: the host chip, the folder browser that lists through that host, a path checked by it before
// Send, recent folders kept apart by host, and a chat whose folder that host makes.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/folder_dialog.dart' show breadcrumbs;
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/remote_host.dart';
import 'package:coqui_web/coqui_web.dart' show WebEndpoints;
import 'package:flutter/material.dart';
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
      case 'fs/scratch':
        result = {'path': '/srv/scratch/${params['session']}', 'made': true};
      case 'fs/scratch_remove':
        result = {'removed': true};
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

RemoteHost _remote(_Far control, {String os = 'linux'}) => RemoteHost(
  _far,
  ServedHost(
    peer: _far.peer,
    method: PeerMethod.noise,
    os: os,
    entries: const [ServedEntry(name: 'scripted', kind: 'agent', url: 'wss://far.test:9/scripted')],
    control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://far.test:9/control'),
  ),
  HostControl.dialing(() async => control),
);

typedef _Reached = ({String url, String peer});

Future<(SessionStore, OptionsAgent, List<_Reached>, _Far, SessionLedger)> _page(WidgetTester tester) async {
  final agent = OptionsAgent();
  final reached = <_Reached>[];
  final far = _Far();
  final ledger = SessionLedger(MemoryLedgerBackend());
  final store = SessionStore(ledger: ledger);
  await pumpApp(
    tester,
    store,
    config: const CanoaConfig(agents: [scripted]).withConnection(_far),
    pickFolder: () async => '/home/me/local',
    openRemote: (connection) async => _remote(far),
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
}
