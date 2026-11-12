// The host's question of which MCP servers a session may start: the wire answers it with the person's choice, says in
// `initialize` that it can, and a client without an approver is neither asked nor answers.
import 'dart:convert';

import 'package:canoa/acp/usage_wire.dart';
import 'package:canoa/profile/approval_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

String _ask(Object id) => jsonEncode({
  'jsonrpc': '2.0',
  'id': id,
  'method': approveMethod,
  'params': {
    'profile': 'default',
    'harness': 'Claude',
    'servers': [
      {'name': 'team', 'layer': 'overlay:team', 'digest': 'sha256:${'a' * 64}'},
      {'name': 7},
    ],
  },
});

void main() {
  test('the wire answers the host with the names the approver allows, and keeps the request from the client', () async {
    final sent = <String>[];
    ProfileApproval? seen;
    final wire = UsageWire((message) async => sent.add(message), approver: (ask) async {
      seen = ask;
      return ['team'];
    });
    expect(wire.answersApproval(_ask('canoa-approve-1')), isTrue);
    await Future<void>.delayed(Duration.zero);
    expect(seen!.servers.map((server) => server.name), ['team'], reason: 'a malformed server is left out');
    final answer = jsonDecode(sent.single) as Map<String, dynamic>;
    expect(answer['id'], 'canoa-approve-1');
    expect(answer['result'], {'allow': ['team']});
  });

  test('an approver that fails allows nothing', () async {
    final sent = <String>[];
    final wire = UsageWire((message) async => sent.add(message), approver: (ask) async => throw StateError('no window'));
    expect(wire.answersApproval(_ask(3)), isTrue);
    await Future<void>.delayed(Duration.zero);
    expect((jsonDecode(sent.single) as Map)['result'], {'allow': <String>[]});
  });

  test('without an approver the request is not taken and initialize does not say the client answers', () {
    final wire = UsageWire((message) async {});
    expect(wire.answersApproval(_ask(1)), isFalse);
    final plain = jsonDecode(wire.outgoing(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}})));
    expect((plain['params'] as Map).containsKey('_meta'), isFalse);
    final answering = UsageWire((message) async {}, approver: (ask) async => const []);
    final said = jsonDecode(answering.outgoing(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}})));
    expect(said['params']['_meta'][approveMeta], isTrue);
  });

  testWidgets('the dialog starts only the servers switched on, and can keep the choice', (tester) async {
    ApprovalChoice? chosen;
    final ask = (
      profile: 'default',
      harness: 'Claude',
      servers: [(name: 'team', layer: 'overlay:team', digest: 'sha256:${'a' * 64}')],
    );
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () async => chosen = await showDialog<ApprovalChoice>(
            context: context,
            builder: (_) => ProfileApprovalDialog(ask: ask, agent: 'Claude'),
          ),
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('approve-start')));
    await tester.pumpAndSettle();
    expect(chosen!.allow, isEmpty, reason: 'off until switched on');
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('approve-team')));
    await tester.tap(find.byKey(const Key('approve-remember')));
    await tester.tap(find.byKey(const Key('approve-start')));
    await tester.pumpAndSettle();
    expect(chosen!.allow, ['team']);
    expect(chosen!.remember, isTrue);
  });
}
