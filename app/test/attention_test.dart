// A session that needs the person, while they look elsewhere: a notification naming it and the kind, never what the agent sent,
// and a mark on its row.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/attention/notifier.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Notifier implements Notifier {
  final shown = <({String title, String body, String sessionId})>[];
  bool looking = true;
  void Function(String)? open;

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async => open = onOpen;
  @override
  Future<bool> get available async => true;
  @override
  Future<bool> requestPermission() async => true;
  @override
  Future<bool> attended() async => looking;
  @override
  Future<void> show({required String title, required String body, required String sessionId}) async =>
      shown.add((title: title, body: body, sessionId: sessionId));
}

/// An agent in memory, speaking v2, that asks and ends turns when a test says.
final class _Agent {
  final incoming = StreamController<String>();
  late final VerifiedAgent client;

  Future<ChatSession> start(SessionStore store, String title) async {
    client = VerifiedAgent.fromChild(
      label: title,
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
          'initialize' => {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}},
          'session/new' => {'sessionId': 's1'},
          _ => <String, Object?>{},
        }}));
      },
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: title, agent: title, peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    return session;
  }

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body},
  }));

  void turn() {
    update({'sessionUpdate': 'state_update', 'state': 'running'});
    update({'sessionUpdate': 'state_update', 'state': 'idle', 'stopReason': 'end_turn'});
  }

  void ask() => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': 5, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'title': 'Run rm -rf ~/secrets',
      'subject': {'type': 'command', 'command': 'rm -rf ~/secrets'},
      'options': [{'optionId': 'y', 'name': 'Yes', 'kind': 'allow_once'}, {'optionId': 'n', 'name': 'No', 'kind': 'reject_once'}],
    },
  }));
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump();
}

/// Two conversations, Front chosen and Back behind it, shown in the app with [notifier].
Future<(_Agent, ChatSession, ChatSession, SessionStore)> _two(WidgetTester tester, _Notifier notifier,
    {CanoaConfig config = const CanoaConfig()}) async {
  await tester.binding.setSurfaceSize(const Size(1100, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore(includePreview: false);
  final back = _Agent();
  final behind = await tester.runAsync(() => back.start(store, 'Back')) as ChatSession;
  final front = await tester.runAsync(() => _Agent().start(store, 'Front')) as ChatSession;
  store.select(front.id);
  await tester.pumpWidget(CanoaApp(store: store, config: config, notifier: notifier));
  await tester.pump();
  return (back, behind, front, store);
}

Finder _row(String title) => find.ancestor(of: find.text(title), matching: find.byType(ListTile)).first;

void main() {
  testWidgets('a request in a conversation behind is notified by its name and kind, never by what it would do, and marked', (tester) async {
    final notifier = _Notifier();
    final (back, behind, _, _) = await _two(tester, notifier);
    back.ask();
    await _settle(tester);
    expect(notifier.shown, hasLength(1));
    final shown = notifier.shown.single;
    expect(shown.title, 'Back');
    expect(shown.body, 'Back asks leave to use a tool');
    expect(shown.sessionId, behind.id);
    expect('${shown.title} ${shown.body}', isNot(contains('rm')));
    expect(find.descendant(of: _row('Back'), matching: find.byTooltip('Needs you')), findsOneWidget);
  });

  testWidgets('a finished turn behind is notified and marked new until the conversation is chosen', (tester) async {
    final notifier = _Notifier();
    final (back, behind, _, store) = await _two(tester, notifier);
    back.turn();
    await _settle(tester);
    expect(notifier.shown.single.body, 'Back finished its turn');
    expect(behind.unread, isTrue);
    expect(find.descendant(of: _row('Back'), matching: find.byTooltip('New since you looked')), findsOneWidget);
    await tester.tap(find.text('Back').first);
    await tester.pump();
    expect(behind.unread, isFalse);
    expect(find.byTooltip('New since you looked'), findsNothing);
    expect(store.selectedId, behind.id);
  });

  testWidgets('the conversation the person is looking at is not notified, unless the window is away', (tester) async {
    final notifier = _Notifier();
    final (back, behind, _, store) = await _two(tester, notifier);
    store.select(behind.id);
    await tester.pump();
    back.turn();
    await _settle(tester);
    expect(notifier.shown, isEmpty);
    notifier.looking = false;
    back.turn();
    await _settle(tester);
    expect(notifier.shown, hasLength(1));
    expect(behind.unread, isFalse, reason: 'it is the conversation chosen');
  });

  testWidgets('each kind can be turned off', (tester) async {
    final notifier = _Notifier();
    final (back, _, _, _) = await _two(tester, notifier,
        config: const CanoaConfig(notifications: NotificationSettings(permission: false, finished: false)));
    back.ask();
    back.turn();
    await _settle(tester);
    expect(notifier.shown, isEmpty);
    expect(find.descendant(of: _row('Back'), matching: find.byTooltip('Needs you')), findsOneWidget,
        reason: 'the row still says so');
  });

  testWidgets('choosing a notification opens its conversation', (tester) async {
    final notifier = _Notifier();
    final (back, behind, _, store) = await _two(tester, notifier);
    back.turn();
    await _settle(tester);
    notifier.open!(notifier.shown.single.sessionId);
    await tester.pump();
    expect(store.selectedId, behind.id);
  });

  testWidgets('a working conversation behind shows that it works', (tester) async {
    final (back, _, _, _) = await _two(tester, _Notifier());
    back.update({'sessionUpdate': 'state_update', 'state': 'running'});
    await _settle(tester);
    expect(find.descendant(of: _row('Back'), matching: find.byTooltip('Working')), findsOneWidget);
  });

  test('canoa.cfg keeps each kind, on by default, and refuses what is not yes or no', () {
    const fallback = CanoaConfig();
    expect([fallback.notifications.permission, fallback.notifications.question, fallback.notifications.finished], [true, true, true]);
    final read = CanoaConfig.parse('[notifications]\nfinished = false\n');
    expect(read.notifications.finished, isFalse);
    expect(CanoaConfig.parse(read.encode()).notifications.finished, isFalse);
    expect(CanoaConfig.parse(read.encode()).notifications.permission, isTrue);
    expect(() => CanoaConfig.parse('[notifications]\nfinished = "no"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[notifications]\nerrors = true\n'), throwsFormatException);
  });

  testWidgets('Settings turns a kind off', (tester) async {
    await tester.binding.setSurfaceSize(const Size(900, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    CanoaConfig? saved;
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () async => saved = await showDialog<CanoaConfig>(
        context: context,
        builder: (_) => const SettingsDialog(config: CanoaConfig()),
      ),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Notifications'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(SwitchListTile, 'It finished its turn'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(saved?.notifications.finished, isFalse);
    expect(saved?.notifications.permission, isTrue);
  });

  test('the words name the session and the kind, one line each', () {
    final session = ChatSession(id: 'l', title: 'Fix\nit \u202Enow', agent: 'Claude', status: SessionStatus.connected);
    final question = attentionText(session, AttentionKind.question);
    expect(question.title, 'Fix it  now');
    expect(question.body, 'Claude has a question for you');
  });
}
