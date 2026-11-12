// The host's question of which MCP servers and overlay hooks a session may start: the wire answers it with the person's choice,
// says in `initialize` that it can, and a client without an approver is neither asked nor answers.
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
    'hooks': [
      {
        'overlay': 'team',
        'digest': 'sha256:${'b' * 64}',
        'changed': true,
        'failClosed': true,
        'events': [
          {'event': 'session-start', 'run': 'hook.py', 'timeout': 30, 'onFailure': 'refuse', 'match': ['new'], 'fires': true},
          {'event': 'cd', 'run': 'hook.py', 'onFailure': 'report', 'fires': false},
          {'event': 7},
        ],
      },
      {'overlay': 3},
    ],
  },
});

void main() {
  test('the wire answers the host with the names the approver allows, and keeps the request from the client', () async {
    final sent = <String>[];
    ProfileApproval? seen;
    final wire = UsageWire((message) async => sent.add(message), approver: (ask) async {
      seen = ask;
      return (servers: ['team'], hooks: ['team']);
    });
    expect(wire.answersApproval(_ask('canoa-approve-1')), isTrue);
    await Future<void>.delayed(Duration.zero);
    expect(seen!.servers.map((server) => server.name), ['team'], reason: 'a malformed server is left out');
    expect(seen!.hooks.map((hook) => hook.overlay), ['team'], reason: 'a malformed overlay is left out');
    final hook = seen!.hooks.single;
    expect(hook.digest, 'sha256:${'b' * 64}');
    expect(hook.changed && hook.failClosed, isTrue);
    expect(hook.events.map((each) => each.event), ['session-start', 'cd'], reason: 'a malformed event is left out');
    expect(hook.events.first.match, ['new']);
    expect(hook.events.first.onFailure, 'refuse');
    expect(hook.events.first.timeout, 30);
    expect(hook.events.last.fires, isFalse);
    final answer = jsonDecode(sent.single) as Map<String, dynamic>;
    expect(answer['id'], 'canoa-approve-1');
    expect(answer['result'], {'allow': ['team'], 'allowHooks': ['team']});
  });

  test('an approver that fails allows nothing', () async {
    final sent = <String>[];
    final wire = UsageWire((message) async => sent.add(message), approver: (ask) async => throw StateError('no window'));
    expect(wire.answersApproval(_ask(3)), isTrue);
    await Future<void>.delayed(Duration.zero);
    expect((jsonDecode(sent.single) as Map)['result'], {'allow': <String>[], 'allowHooks': <String>[]});
  });

  test('without an approver the request is not taken and initialize does not say the client answers', () {
    final wire = UsageWire((message) async {});
    expect(wire.answersApproval(_ask(1)), isFalse);
    final plain = jsonDecode(wire.outgoing(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}})));
    expect((plain['params'] as Map).containsKey('_meta'), isFalse);
    final answering = UsageWire((message) async {}, approver: (ask) async => noApproval);
    final said = jsonDecode(answering.outgoing(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}})));
    expect(said['params']['_meta'][approveMeta], isTrue);
  });

  testWidgets('a question left open closes by itself with nothing allowed and nothing to keep', (tester) async {
    var closed = false;
    ApprovalChoice? chosen = const ApprovalChoice(['x']);
    final ask = (
      profile: 'default',
      harness: 'Claude',
      servers: [(name: 'team', layer: 'overlay:team', digest: 'sha256:${'a' * 64}')],
      hooks: <ApprovalHook>[],
    );
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () async {
            chosen = await showDialog<ApprovalChoice>(
              context: context,
              builder: (_) => ProfileApprovalDialog(ask: ask, agent: 'Claude', expires: const Duration(seconds: 5)),
            );
            closed = true;
          },
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    expect(closed, isTrue);
    expect(chosen, isNull, reason: 'no answer: main allows nothing and writes no canoa.cfg');
  });

  testWidgets('the dialog starts only the servers switched on, and can keep the choice', (tester) async {
    ApprovalChoice? chosen;
    final ask = (
      profile: 'default',
      harness: 'Claude',
      servers: [(name: 'team', layer: 'overlay:team', digest: 'sha256:${'a' * 64}')],
      hooks: <ApprovalHook>[],
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

  testWidgets('the dialog lists an overlay\'s hooks beside the servers, off until switched on, and keeps them by digest', (tester) async {
    ApprovalChoice? chosen;
    final digest = 'sha256:${'c' * 64}';
    final ask = (
      profile: 'default',
      harness: 'Claude',
      servers: <ApprovalServer>[],
      hooks: <ApprovalHook>[
        (
          overlay: 'team',
          digest: digest,
          changed: true,
          failClosed: true,
          events: <ApprovalHookEvent>[
            (event: 'prompt', run: 'hook.py', timeout: 5, onFailure: 'refuse', match: <String>[], sets: <String>['X'], fires: true),
            (event: 'cd', run: 'hook.py', timeout: null, onFailure: 'report', match: <String>[], sets: <String>[], fires: false),
          ],
        ),
      ],
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
    expect(find.text('Claude asks to start hooks'), findsOneWidget);
    expect(find.textContaining('prompt: hook.py, 5 s at most, refuses on failure, may set X'), findsOneWidget);
    expect(find.textContaining('never runs'), findsOneWidget, reason: 'an event Canoa does not see is said so');
    expect(find.textContaining('another version'), findsOneWidget);
    expect(find.textContaining(digest), findsOneWidget);
    await tester.tap(find.byKey(const Key('approve-start')));
    await tester.pumpAndSettle();
    expect(chosen!.hooks, isEmpty, reason: 'off until switched on');
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    // The dialog scrolls: what a tap needs is on the screen.
    await tester.ensureVisible(find.byKey(const Key('approve-hooks-team')));
    await tester.tap(find.byKey(const Key('approve-hooks-team')));
    await tester.ensureVisible(find.byKey(const Key('approve-remember')));
    await tester.tap(find.byKey(const Key('approve-remember')));
    await tester.tap(find.byKey(const Key('approve-start')));
    await tester.pumpAndSettle();
    expect(chosen!.hooks, ['team']);
    expect(chosen!.allow, isEmpty);
    expect(chosen!.remember, isTrue);
  });
}
