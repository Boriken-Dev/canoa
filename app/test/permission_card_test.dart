// What reaches the permission card from the store and what comes back: the question and the reason a request carries, the edit's
// diff or the command inline, the keys Canoa binds, and each answer reaching the agent. The card's own layout is tested in
// coqui_acp_chat_ui.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// An agent in memory, speaking v2 where it asks: a request carries the reason the agent gives.
final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store) async {
    client = VerifiedAgent.fromChild(
      label: 'Claude',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        if (message['method'] == 'initialize') {
          reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (message['method'] == 'session/new') {
          reply(message['id'], {'sessionId': 's1'});
        } else if (message['id'] != null && message['method'] != null && message['method'] != 'session/prompt') {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Claude', agent: 'Claude', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void ask(int id, {required Map<String, Object?> toolCall, String? description, List<Map<String, Object?>>? options}) =>
      incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'toolCall': {'toolCallId': 't$id', ...toolCall},
          'description': ?description,
          'options': options ?? _options,
        },
      }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

const _options = [
  {'optionId': 'allow-always', 'name': 'Always allow', 'kind': 'allow_always'},
  {'optionId': 'allow-once', 'name': 'Allow once', 'kind': 'allow_once'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

const _edit = {
  'title': 'Edit calc.js',
  'kind': 'edit',
  'rawInput': {'file_path': 'src/calc.js'},
  'content': [
    {'type': 'diff', 'path': 'src/calc.js', 'oldText': 'export function addNumbers(a, b) {\n  return a - b;\n}', 'newText': 'export function addNumbers(a, b) {\n  return a + b;\n}'},
  ],
};

const _command = {
  'title': 'Run npm test',
  'kind': 'execute',
  'rawInput': {'command': 'npm test -- --run src/calc.test.js'},
};

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  await tester.pump();
}

Future<(_Agent, ChatSession)> _waiting(
  WidgetTester tester, {
  Size size = const Size(1100, 900),
  Map<String, Object?> toolCall = _edit,
  String? description = 'Path is outside allowed working directories',
  List<Map<String, Object?>>? options,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store));
  await tester.pump();
  await tester.enterText(find.byType(TextField).last, 'Fix the sums');
  await tester.tap(find.byTooltip('Send'));
  await _settle(tester);
  agent.ask(7, toolCall: toolCall, description: description, options: options);
  await _settle(tester);
  return (agent, session);
}

Future<void> _openMore(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('permission-more')));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/permission_card_test.dart')));

  testWidgets('a chip says it needs the person, above a title that asks the question', (tester) async {
    await _waiting(tester);
    final chip = find.byKey(const Key('needs-input'));
    expect(find.descendant(of: chip, matching: find.text('Needs your input')), findsOneWidget);
    final title = find.text('Allow Claude to edit calc.js?');
    expect(title, findsOneWidget);
    expect(tester.getTopLeft(chip).dy, lessThan(tester.getTopLeft(title).dy));
  });

  testWidgets('the agent\'s reason is the line under the title', (tester) async {
    await _waiting(tester);
    final reason = find.text('Path is outside allowed working directories');
    expect(reason, findsOneWidget);
    expect(tester.getTopLeft(reason).dy, greaterThan(tester.getBottomLeft(find.text('Allow Claude to edit calc.js?')).dy - 1));
  });

  testWidgets('an edit shows its diff inline, across the card\'s width', (tester) async {
    await _waiting(tester);
    final diff = find.byType(DiffView);
    expect(diff, findsOneWidget);
    expect(find.descendant(of: diff, matching: find.textContaining('return a + b;')), findsOneWidget);
    expect(find.descendant(of: diff, matching: find.textContaining('return a - b;')), findsOneWidget);
    final card = find.byKey(const Key('permission-card'));
    expect(tester.getSize(diff).width, greaterThan(tester.getSize(card).width - 60), reason: 'full width, not a narrow box');
  });

  testWidgets('a command shows inline, across the card\'s width', (tester) async {
    await _waiting(tester, toolCall: _command, description: null);
    expect(find.text('Allow Claude to run npm test?'), findsOneWidget);
    final command = find.byKey(const Key('permission-detail'));
    expect(find.descendant(of: command, matching: find.textContaining('npm test -- --run src/calc.test.js')), findsOneWidget);
    expect(tester.getSize(command).width, greaterThan(tester.getSize(find.byKey(const Key('permission-card'))).width - 40));
  });

  testWidgets('the primary answers always, Deny refuses, the menu\'s entry allows once', (tester) async {
    final (agent, session) = await _waiting(tester);
    await tester.tap(find.byKey(const Key('permission-primary')));
    await _settle(tester);
    expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-always'}});
    expect(session.pendingPermissions, isEmpty);

    agent.ask(8, toolCall: _command, options: _options);
    await _settle(tester);
    await _openMore(tester);
    await tester.tap(find.text('Allow once'));
    await _settle(tester);
    await _settle(tester);
    await _settle(tester);
    expect(agent.answerTo(8)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});

    agent.ask(9, toolCall: _command, options: _options);
    await _settle(tester);
    await tester.tap(find.byKey(const Key('permission-deny')));
    await _settle(tester);
    expect(agent.answerTo(9)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
  });

  testWidgets('the key hints name the chords that answer: Deny, and Allow once in the menu', (tester) async {
    await _waiting(tester);
    expect(find.descendant(of: find.byKey(const Key('permission-deny')), matching: find.text('Ctrl+Shift+R')), findsOneWidget);
    await _openMore(tester);
    expect(find.descendant(of: find.byKey(const Key('answer-allow_once')), matching: find.text('Ctrl+Shift+A')), findsOneWidget);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the card draws at $label width', (tester) async {
      await _waiting(tester, size: size);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/permission_card_$label.png'));
    });
  }
}
