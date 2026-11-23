// A page the host served has no file system, so its chat has no scratch folder of its own to name: it starts as a Chat unless
// a workspace is chosen, and the host makes the chat's folder when the session opens (the same request the extension's own
// chat makes, marked as the person's own).
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/site.dart';
import 'package:canoa/workspace/site_meta.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';

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

void main() {
  testWidgets('a served page starts as a Chat with no workspace chosen, and Workspace is still offered', (tester) async {
    final host = ServedHost(
      peer: 'sha256:${'a' * 64}',
      method: PeerMethod.noise,
      entries: const [ServedEntry(name: 'Agent', kind: 'agent', url: 'ws://127.0.0.1:1/agent')],
      control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
    );
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    await pumpApp(tester, store, host: ServedSupervisor(host), served: ServedApp.over(host, HostControl.dialing(() async => _Control())));
    await newSession(tester);
    final draft = store.selected!;
    expect(draft.kind, SessionKind.chat);
    expect(draft.cwd, isNull);
    expect(find.byKey(const Key('new-session-kind')), findsOneWidget);
    expect(find.byKey(const Key('new-session-folder')), findsNothing, reason: 'a chat names no workspace');
    await chooseWorkspace(tester);
    expect(draft.kind, SessionKind.workspace);
    expect(find.byKey(const Key('new-session-folder')), findsOneWidget);
    await finish(tester);
  });

  test('the page asks the host for the chat folder as its own session of a site name the host accepts', () {
    expect(siteFqdn('https://$servedSite/'), servedSite, reason: 'a name the host takes as one safe folder name');
    final meta = siteSetup(servedSite).meta![siteMeta]! as Map;
    expect(meta, {'site': servedSite, 'own': true});
    expect(madeFolder({'_meta': {siteMeta: {'site': servedSite, 'id': 'abc', 'folder': '/docs/Canoa/scratch/app/abc'}}}),
        (id: 'abc', folder: '/docs/Canoa/scratch/app/abc'));
    expect(madeFolder({'_meta': {siteMeta: {'site': servedSite}}}), isNull, reason: 'a host that made none is an error, not a guess');
  });
}
