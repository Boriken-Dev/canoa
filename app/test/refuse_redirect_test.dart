// A permission card waiting: the composer says a message declines and redirects, sending refuses once and sends the text, and a plain
// Deny stops the turn.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _redirectHint = 'Send a message to decline and redirect the agent';

/// An agent in memory, speaking v1: a prompt is never answered, so the turn stays open while the request waits.
final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store) async {
    client = VerifiedAgent.fromChild(
      label: 'Scripted',
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
    final session = store.addProviderSession(title: 'Scripted', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void ask(int id) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': 't$id', 'title': 'Edit calc.js', 'kind': 'edit', 'rawInput': {'file_path': 'calc.js'}},
      'options': [
        {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
        {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
      ],
    },
  }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
  List<Map<String, dynamic>> calls(String method) => [for (final message in sent) if (message['method'] == method) message];
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  await tester.pump();
}

Finder get _field => find.byType(TextField).last;

Future<(_Agent, ChatSession, SessionStore)> _waiting(WidgetTester tester, {Size size = const Size(1100, 900)}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store));
  await tester.pump();
  // A first message leaves the turn open, as it is when an agent asks for leave.
  await tester.enterText(_field, 'Fix the sums');
  await tester.tap(find.byTooltip('Send'));
  await _settle(tester);
  agent.ask(7);
  await _settle(tester);
  return (agent, session, store);
}

String? _hintOf(WidgetTester tester) => tester.widget<TextField>(_field).decoration?.hintText;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/refuse_redirect_test.dart')));

  testWidgets('while a card waits the composer says a message declines and redirects', (tester) async {
    final (_, session, _) = await _waiting(tester);
    expect(session.pendingPermissions, hasLength(1));
    expect(_hintOf(tester), _redirectHint);
  });

  testWidgets('the placeholder is the usual one again once the card is answered', (tester) async {
    final (agent, session, _) = await _waiting(tester);
    await tester.tap(find.byKey(const Key('permission-primary')));
    await _settle(tester);
    expect(session.pendingPermissions, isEmpty);
    expect(_hintOf(tester), isNot(_redirectHint));
    expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
  });

  testWidgets('sending refuses once, with the agent\'s own refusal, and the text goes to the agent', (tester) async {
    final (agent, session, store) = await _waiting(tester);
    await tester.enterText(_field, 'Edit calc.ts instead');
    await tester.tap(find.byTooltip('Queue'));
    await _settle(tester);
    expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
    expect(session.pendingPermissions, isEmpty);
    expect(agent.calls('session/cancel'), isEmpty, reason: 'a redirect does not stop the turn');
    expect(session.queue.map((message) => message.text), ['Edit calc.ts instead'], reason: 'the turn is still open, so it waits its end');
    // The turn ends, and the message goes.
    final prompt = agent.calls('session/prompt').first;
    agent.reply(prompt['id'], {'stopReason': 'end_turn'});
    await _settle(tester);
    await _settle(tester);
    final prompts = agent.calls('session/prompt');
    expect(prompts, hasLength(2));
    expect(jsonEncode(prompts.last['params']), contains('Edit calc.ts instead'));
    expect(store.selected!.queue, isEmpty);
  });

  testWidgets('a plain Deny answers the refusal and stops the turn', (tester) async {
    final (agent, session, _) = await _waiting(tester);
    await tester.tap(find.byKey(const Key('permission-deny')));
    await _settle(tester);
    expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
    expect(agent.calls('session/cancel'), hasLength(1));
    expect(session.pendingPermissions, isEmpty);
  });

  testWidgets('an empty message sends nothing and leaves the card waiting', (tester) async {
    final (agent, session, _) = await _waiting(tester);
    await tester.tap(find.byTooltip('Queue'));
    await _settle(tester);
    expect(session.pendingPermissions, hasLength(1));
    expect(agent.answerTo(7), isNull);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the waiting card and the redirect placeholder draw at $label width', (tester) async {
      await _waiting(tester, size: size);
      expect(tester.takeException(), isNull);
      final hint = _hintOf(tester)!;
      expect(hint, label == 'phone' ? 'Message to decline and redirect' : _redirectHint);
      expect(tester.getSize(find.text(hint)).height, lessThan(30), reason: 'one line of placeholder at $label width');
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/refuse_redirect_$label.png'));
    });
  }
}
