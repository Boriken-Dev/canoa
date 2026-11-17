// A page a host served holds no host in its process: every session it starts or opens again, with an account or without,
// reaches the host over the entry's endpoint, and none goes through the in-process link only a desktop has.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/accounts/account_models.dart' show accountMeta;
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_transport_web/coqui_transport_web.dart' show WebEndpoints;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/options_agent.dart';

/// A control channel that grants the ledger and nothing else.
final class _Control implements HostLink {
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final result = switch (request['method']) {
      'host/grants' => {'grants': ['ledger.read', 'ledger.write']},
      'ledger/list' => {'records': <Object?>[]},
      _ => null,
    };
    _out.add(jsonEncode(result == null
        ? {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32001, 'message': 'no grant'}}
        : {'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() => _out.close();
}

const _config = CanoaConfig(
  agents: [],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['one', 'two']},
    },
  },
);

typedef _Reached = ({String url, Map<String, Object?> meta});

/// The served page on a host listing Claude, whose endpoint is [reached] when the app connects to it.
Future<(SessionStore, OptionsAgent, List<_Reached>)> _page(WidgetTester tester, {bool accounts = true, SessionLedger? ledger}) async {
  final host = ServedHost(
    peer: 'sha256:${'a' * 64}',
    method: PeerMethod.noise,
    entries: const [ServedEntry(name: 'Claude', kind: 'agent', url: 'ws://127.0.0.1:1/claude', harness: 'claude')],
    control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
  );
  final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
  final reached = <_Reached>[];
  final store = SessionStore(ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));
  await pumpApp(
    tester,
    store,
    config: _config,
    host: ServedSupervisor(host),
    served: ServedApp.over(host, HostControl.dialing(() async => _Control())),
    accountsHost: accounts ? FakeAccountsHost(accounts: [account('one'), account('two')]) : null,
    connectEndpoint: (WebEndpoints endpoints, {required String pinnedPeerId, required PeerMethod method, Duration timeout = const Duration(seconds: 10),
        AgentDecider? decider, AgentElicitor? elicitor, AgentSecrets? secrets, Map<String, Object?> initializeMeta = const {}}) async {
      reached.add((url: endpoints.webSocket.toString(), meta: initializeMeta));
      final link = agent.openLink('over the endpoint');
      final client = VerifiedAgent.fromChild(
        label: 'Claude', messages: link.messages, send: link.send, close: link.close,
        decider: decider, elicitor: elicitor, initializeMeta: initializeMeta,
      );
      await client.initialize();
      return client;
    },
  );
  return (store, agent, reached);
}

void main() {
  testWidgets('a served page\'s new chat with an account starts over the host\'s endpoint, with the account in initialize', (tester) async {
    final (store, agent, reached) = await _page(tester);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(reached, hasLength(1), reason: 'the chat started, and over its endpoint');
    expect(reached.single.url, 'ws://127.0.0.1:1/claude');
    expect(reached.single.meta[accountMeta], 'one', reason: 'the first account the entry offers');
    expect(store.selected!.place, SessionPlace.host);
    expect(store.selected!.account, 'one');
    expect(agent.methods, contains('session/prompt'));
    await finish(tester);
  });

  testWidgets('a conversation the ledger notes as begun on the host\'s own computer opens again over the endpoint', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.record(
      peerId: 'local:Claude', sessionId: 'o1', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/load',
      cwd: '/work/repo', title: 'An earlier one', account: 'one',
    );
    final (store, agent, reached) = await _page(tester, ledger: ledger);
    await ledger.flush();
    await tester.ensureVisible(find.byTooltip('Reopen'));
    await tester.tap(find.byTooltip('Reopen'));
    await settle(tester, 150);
    expect(reached, hasLength(1), reason: 'reached through the host\'s endpoint, not an in-process link');
    expect(reached.single.url, 'ws://127.0.0.1:1/claude');
    expect(reached.single.meta[accountMeta], 'one');
    expect(agent.methods, contains('session/load'));
    expect(store.sessions.where((each) => each.remoteSessionId == 'o1'), hasLength(1));
    await finish(tester);
  });

  testWidgets('a served page\'s new chat with no account starts over the endpoint too', (tester) async {
    final (store, _, reached) = await _page(tester, accounts: false);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Say hello');
    await settle(tester, 150);
    expect(reached, hasLength(1));
    expect(store.selected!.place, SessionPlace.host);
    await finish(tester);
  });
}
