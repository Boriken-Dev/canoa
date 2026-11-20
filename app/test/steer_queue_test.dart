// Steering and queueing while a turn runs: Enter and Alt+Enter by the `[sessions] while_working` setting, the composer's Steer
// and Queue buttons, a steer that cannot be sent coming back to the draft, queued messages joined, moved and taken back, and the
// queue going as one message by `[sessions] queue_delivery`. Against an in-memory ACP v2 agent that answers what a test says.
import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory;

import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// An agent speaking ACP v2 in memory: it answers each request with `{}` (a prompt with a message id), or with an error for a
/// method a test lists in [refuse].
final class _Agent {
  _Agent(SessionStore store, {Set<String> refuse = const {}}) : refuse = {...refuse} {
    client = VerifiedAgent.fromChild(
      label: 'V2',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'], method = message['method'];
        if (method == null || id == null && !this.refuse.contains(method)) return;
        if (this.refuse.contains(method)) {
          // A notification cannot be answered with an error: the agent is not reachable for it.
          if (id == null) throw StateError('not now');
          incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32000, 'message': 'not now'}}));
          return;
        }
        incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (method) {
          'initialize' => {'protocolVersion': 2, 'capabilities': {'session': {'prompt': {'embeddedContext': true}}}},
          'session/new' => {'sessionId': 's1'},
          'session/prompt' => {'messageId': 'm${sent.length}'},
          _ => <String, Object?>{},
        }}));
      },
    );
  }

  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  final Set<String> refuse;
  late final VerifiedAgent client;

  List<String> get prompts => [
    for (final message in sent)
      if (message['method'] == 'session/prompt') ((message['params']['prompt'] as List).first['text'] ?? '') as String,
  ];
  int get cancels => sent.where((message) => message['method'] == 'session/cancel').length;

  void update(Map<String, Object?> body) =>
      incoming.add(jsonEncode({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body}}));
  void running() => update({'sessionUpdate': 'state_update', 'state': 'running'});
  void idle(String reason) => update({'sessionUpdate': 'state_update', 'state': 'idle', 'stopReason': reason});
}

/// The app on a store with one session open on an agent whose turn is running.
Future<(_Agent, ChatSession, SessionStore)> _working(
  WidgetTester tester, {
  CanoaConfig config = const CanoaConfig(),
  Set<String> refuse = const {},
}) async {
  await tester.binding.setSurfaceSize(const Size(1100, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  final agent = _Agent(store, refuse: refuse);
  await tester.runAsync(() async {
    await agent.client.initialize();
    await agent.client.openSession(cwd: '/work');
  });
  final session = store.addProviderSession(title: 'V2', agent: 'V2', peerId: agent.client.peerId, remoteSessionId: 's1');
  store.attachVerifiedClient(session.id, agent.client, record: false);
  store.select(session.id);
  await tester.pumpWidget(CanoaApp(store: store, config: config));
  agent.running();
  await _settle(tester);
  return (agent, session, store);
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

Finder get _composer => find.byWidgetPredicate((widget) => widget is TextField && (widget.decoration?.hintText ?? '').startsWith('Message'));

String _textOf(WidgetTester tester) => tester.widget<TextField>(_composer).controller!.text;

/// Enter in the message: the keyboard's send action.
Future<void> _enter(WidgetTester tester, String text) async {
  await tester.enterText(_composer, text);
  await tester.testTextInput.receiveAction(TextInputAction.send);
  await _settle(tester);
}

/// Alt+Enter in the message.
Future<void> _altEnter(WidgetTester tester, String text) async {
  await tester.enterText(_composer, text);
  await tester.tap(_composer);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
  await _settle(tester);
}

CanoaConfig _steering(WhileWorking whileWorking, {QueueDelivery delivery = QueueDelivery.merge}) =>
    CanoaConfig(sessions: LedgerSettings(whileWorking: whileWorking, queueDelivery: delivery));

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/steer_queue_test.dart')));

  group('the box while a turn runs', () {
    for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
      testWidgets('has Stop beside the microphone and a spinner at the end of the line, at $label width', (tester) async {
        await _working(tester);
        await tester.binding.setSurfaceSize(size);
        await _settle(tester);
        final box = tester.getRect(find.byKey(const Key('composer')));
        expect(find.byKey(const Key('composer-stop')), findsOneWidget);
        expect(find.byKey(const Key('dictate')), findsOneWidget, reason: 'the microphone stays while nothing is typed');
        expect(find.byKey(const Key('composer-queue')), findsNothing, reason: 'nothing typed to queue');
        expect(find.byKey(const Key('composer-steer')), findsNothing);
        expect(find.byTooltip('Send'), findsNothing);
        expect(box.contains(tester.getCenter(find.byKey(const Key('composer-stop')))), isTrue);
        expect(tester.getTopLeft(find.byKey(const Key('composer-stop'))).dx, lessThan(tester.getTopLeft(find.byKey(const Key('dictate'))).dx));
        expect(tester.getTopLeft(find.byKey(const Key('composer-busy'))).dy, greaterThanOrEqualTo(box.bottom), reason: 'under the box');
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/composer_working_$label.png'));
      });
    }

    testWidgets('typing puts Queue and Steer where the microphone was, and the spinner goes when the turn ends', (tester) async {
      final (agent, _, _) = await _working(tester);
      expect(find.byKey(const Key('composer-busy')), findsOneWidget);
      await tester.enterText(_composer, 'next');
      await tester.pump();
      expect(find.byKey(const Key('dictate')), findsNothing);
      expect(find.byKey(const Key('composer-queue')), findsOneWidget);
      expect(find.byKey(const Key('composer-steer')), findsOneWidget);
      expect(find.byKey(const Key('composer-stop')), findsOneWidget);
      agent.idle('end_turn');
      await _settle(tester);
      expect(find.byKey(const Key('composer-busy')), findsNothing);
      expect(find.byKey(const Key('composer-stop')), findsNothing);
      expect(find.byTooltip('Send'), findsOneWidget, reason: 'the typed message is still there');
    });
  });

  group('the keys', () {
    testWidgets('Enter queues and Alt+Enter steers by default; Steer stops the turn and the message goes first', (tester) async {
      final (agent, session, _) = await _working(tester);
      await _enter(tester, 'queued one');
      expect(session.queue.single.text, 'queued one');
      expect(agent.cancels, 0, reason: 'a queued message stops nothing');
      await _altEnter(tester, 'change course');
      expect(agent.cancels, 1);
      expect(session.queue.map((message) => message.text), ['change course', 'queued one']);
      agent.idle('cancelled');
      await _settle(tester);
      expect(agent.prompts, ['change course'], reason: 'the steer goes first and alone');
      expect(session.queue.single.text, 'queued one');
      expect(find.byType(SnackBar), findsNothing);
    });

    testWidgets('with while_working = steer, Enter steers and Alt+Enter queues, and the tooltips say so', (tester) async {
      final (agent, session, _) = await _working(tester, config: _steering(WhileWorking.steer));
      expect(find.byKey(const Key('composer-steer')), findsNothing, reason: 'nothing is typed to steer with');
      await tester.enterText(_composer, 'x');
      await tester.pump();
      expect(find.byTooltip('Steer: stop the turn and send this first (Enter)'), findsOneWidget);
      expect(find.byTooltip('Queue: send this when the turn ends (Alt+Enter)'), findsOneWidget);
      await _altEnter(tester, 'later');
      expect(session.queue.single.text, 'later');
      expect(agent.cancels, 0);
      await _enter(tester, 'now');
      expect(agent.cancels, 1);
      expect(session.queue.map((message) => message.text), ['now', 'later']);
    });

    testWidgets('the buttons do the same as the keys', (tester) async {
      final (agent, session, _) = await _working(tester);
      await tester.enterText(_composer, 'queued');
      await tester.pump();
      await tester.tap(find.byKey(const Key('composer-queue')));
      await _settle(tester);
      await tester.enterText(_composer, 'steering');
      await tester.pump();
      await tester.tap(find.byKey(const Key('composer-steer')));
      await _settle(tester);
      expect(agent.cancels, 1);
      expect(session.queue.map((message) => message.text), ['steering', 'queued']);
    });
  });

  group('a steer that cannot be sent', () {
    testWidgets('because the agent could not be asked to stop keeps the draft, its text and its attachment', (tester) async {
      final (agent, session, store) = await _working(tester, refuse: {'session/cancel'});
      expect(store.addAttachment(session, 'notes.txt', Uint8List.fromList(utf8.encode('some notes')), mimeType: 'text/plain'), isNull);
      await _altEnter(tester, 'change course');
      expect(find.textContaining('Not steered'), findsOneWidget);
      expect(_textOf(tester), 'change course');
      expect(session.attachments.map((item) => item.name), ['notes.txt']);
      expect(session.queue, isEmpty);
      expect(agent.prompts, isEmpty);
    });

    testWidgets('because the agent refused it once the turn ended gives the draft back, attachment included', (tester) async {
      final (agent, session, store) = await _working(tester, refuse: {'session/prompt'});
      expect(store.addAttachment(session, 'notes.txt', Uint8List.fromList(utf8.encode('some notes')), mimeType: 'text/plain'), isNull);
      await _altEnter(tester, 'change course');
      expect(_textOf(tester), '', reason: 'the draft is kept until the agent has the message');
      expect(session.attachments, isEmpty);
      agent.idle('cancelled');
      await _settle(tester);
      expect(find.textContaining('Not steered'), findsOneWidget);
      expect(_textOf(tester), 'change course');
      expect(session.attachments.map((item) => item.name), ['notes.txt']);
      expect(session.queue, isEmpty);
    });
  });

  group('queued messages', () {
    testWidgets('go as one message when the turn ends, by default', (tester) async {
      final (agent, session, _) = await _working(tester);
      await _enter(tester, 'first');
      await _enter(tester, 'second');
      expect(find.text('2 queued: sent together when the turn ends'), findsOneWidget);
      agent.idle('end_turn');
      await _settle(tester);
      expect(agent.prompts, ['first\n\nsecond']);
      expect(session.queue, isEmpty);
    });

    testWidgets('go one a turn with queue_delivery = each', (tester) async {
      final (agent, session, _) = await _working(tester, config: _steering(WhileWorking.queue, delivery: QueueDelivery.each));
      await _enter(tester, 'first');
      await _enter(tester, 'second');
      expect(find.text('2 queued: sent one at a time, as each turn ends'), findsOneWidget);
      agent.idle('end_turn');
      await _settle(tester);
      expect(agent.prompts, ['first']);
      expect(session.queue.single.text, 'second');
    });

    testWidgets('joined by the row menu keep each one\'s attachments once, and Merge all joins the rest', (tester) async {
      final (agent, session, store) = await _working(tester);
      Uint8List bytes(String text) => Uint8List.fromList(utf8.encode(text));
      store.addAttachment(session, 'a.txt', bytes('a'), mimeType: 'text/plain');
      await _enter(tester, 'one');
      store.addAttachment(session, 'a.txt', bytes('a'), mimeType: 'text/plain');
      store.addAttachment(session, 'b.txt', bytes('b'), mimeType: 'text/plain');
      await _enter(tester, 'two');
      await _enter(tester, 'three');
      await tester.tap(find.byKey(const Key('queue-more-1')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.text('Merge with above'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(session.queue.map((message) => message.text), ['one\n\ntwo', 'three']);
      expect(session.queue.first.attachments.map((item) => item.name), ['a.txt', 'b.txt']);
      await tester.tap(find.text('Merge all'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(session.queue.single.text, 'one\n\ntwo\n\nthree');
      agent.idle('end_turn');
      await _settle(tester);
      expect(agent.prompts, ['one\n\ntwo\n\nthree']);
    });

    testWidgets('moved to the composer leave the queue, and what went with them comes back to the draft', (tester) async {
      final (agent, session, store) = await _working(tester);
      store.addAttachment(session, 'a.txt', Uint8List.fromList(utf8.encode('a')), mimeType: 'text/plain');
      await _enter(tester, 'think again');
      await _enter(tester, 'keep this');
      await tester.enterText(_composer, 'already writing');
      await tester.tap(find.byKey(const Key('queue-more-0')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.text('Move to composer'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(session.queue.single.text, 'keep this');
      expect(_textOf(tester), 'already writing\n\nthink again');
      expect(session.attachments.map((item) => item.name), ['a.txt']);
      agent.idle('end_turn');
      await _settle(tester);
      expect(agent.prompts, ['keep this'], reason: 'what was taken back is not sent');
    });

    testWidgets('Steer now in a row stops the turn and sends that message first', (tester) async {
      final (agent, session, _) = await _working(tester, config: _steering(WhileWorking.queue, delivery: QueueDelivery.each));
      await _enter(tester, 'first');
      await _enter(tester, 'second');
      await tester.tap(find.byKey(const Key('queue-more-1')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.text('Steer now'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(agent.cancels, 1);
      expect(session.queue.map((message) => message.text), ['second', 'first']);
      agent.idle('cancelled');
      await _settle(tester);
      expect(agent.prompts, ['second']);
    });

    testWidgets('moved up and down change the order they go in', (tester) async {
      final (agent, session, _) = await _working(tester, config: _steering(WhileWorking.queue, delivery: QueueDelivery.each));
      for (final text in ['a', 'b', 'c']) {
        await _enter(tester, text);
      }
      await tester.tap(find.byKey(const Key('queue-more-2')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.text('Move up'));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.byKey(const Key('queue-more-0')));
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.text('Move down'));
      await tester.pump(const Duration(milliseconds: 400));
      expect(session.queue.map((message) => message.text), ['c', 'a', 'b']);
      agent.idle('end_turn');
      await _settle(tester);
      expect(agent.prompts, ['c']);
    });

    testWidgets('held after a stop, Merge & send sends them as one and Send next sends only the first', (tester) async {
      final (agent, session, _) = await _working(tester);
      for (final text in ['a', 'b', 'c']) {
        await _enter(tester, text);
      }
      await tester.tap(find.byTooltip('Stop'));
      agent.idle('cancelled');
      await _settle(tester);
      expect(session.queueHeld, 'the turn was stopped');
      await tester.tap(find.text('Send next'));
      await _settle(tester);
      expect(agent.prompts, ['a']);
      agent.running();
      await _settle(tester);
      await tester.tap(find.byTooltip('Stop'));
      agent.idle('cancelled');
      await _settle(tester);
      expect(session.queueHeld, 'the turn was stopped');
      expect(find.text('Merge & send'), findsOneWidget);
      await tester.tap(find.text('Merge & send'));
      await _settle(tester);
      expect(agent.prompts, ['a', 'b\n\nc']);
      expect(session.queue, isEmpty);
    });
  });

  group('canoa.cfg', () {
    test('while_working and queue_delivery default to queue and merge, parse, and are written only when changed', () {
      final plain = CanoaConfig.parse('[sessions]\nkeep = 5\n');
      expect(plain.sessions.whileWorking, WhileWorking.queue);
      expect(plain.sessions.queueDelivery, QueueDelivery.merge);
      expect(plain.encode(), isNot(contains('while_working')));
      expect(plain.encode(), isNot(contains('queue_delivery')));
      final set = CanoaConfig.parse('[sessions]\nwhile_working = "steer"\nqueue_delivery = "each"\n');
      expect(set.sessions.whileWorking, WhileWorking.steer);
      expect(set.sessions.queueDelivery, QueueDelivery.each);
      final again = CanoaConfig.parse(set.encode());
      expect(again.sessions.whileWorking, WhileWorking.steer);
      expect(again.sessions.queueDelivery, QueueDelivery.each);
    });

    test('another word for either is refused, naming the key', () {
      expect(() => CanoaConfig.parse('[sessions]\nwhile_working = "stop"\n'), throwsA(predicate((e) => '$e'.contains('sessions.while_working'))));
      expect(() => CanoaConfig.parse('[sessions]\nqueue_delivery = "all"\n'), throwsA(predicate((e) => '$e'.contains('sessions.queue_delivery'))));
    });
  });
}
