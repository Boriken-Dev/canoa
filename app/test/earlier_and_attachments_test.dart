// Earlier conversations and attachments through the store and the screen, against an agent saying what the real adapter says.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/transcript.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Agent {
  _Agent({this.images = true, this.files = true});

  final bool images;
  final bool files;
  final toClient = StreamController<String>();
  final received = <Map<String, dynamic>>[];
  List<Map<String, Object?>> listed = const [];
  bool refuseList = false;
  bool refuseOpen = false;

  void update(String session, Map<String, Object?> update) => toClient.add(jsonEncode({
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {'sessionId': session, 'update': update},
  }));

  Future<void> send(String text) async {
    final message = jsonDecode(text) as Map<String, dynamic>;
    received.add(message);
    final id = message['id'];
    if (id == null) return;
    void answer(Object? result) => toClient.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
    void fail() => toClient.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32601, 'message': 'no'}}));
    switch (message['method']) {
      case 'initialize':
        answer({
          'protocolVersion': 1,
          'agentCapabilities': {
            'promptCapabilities': {'image': images, 'embeddedContext': files},
            'loadSession': true,
            'sessionCapabilities': {'list': <String, Object?>{}},
          },
        });
      case 'session/new':
        answer({'sessionId': 'now'});
      case 'session/list':
        if (refuseList) return fail();
        answer({'sessions': listed});
      case 'session/load':
        if (refuseOpen) return fail();
        final session = message['params']['sessionId'] as String;
        update(session, {'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': 'asked before'}});
        update(session, {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'said before'}});
        answer(<String, Object?>{});
      case 'session/prompt':
        answer({'stopReason': 'end_turn'});
    }
  }
}

Future<(SessionStore, ChatSession, _Agent, VerifiedAgent)> _open({_Agent? with_}) async {
  final agent = with_ ?? _Agent();
  final client = VerifiedAgent.fromChild(
    label: 'Test',
    messages: agent.toClient.stream,
    send: agent.send,
    close: () async {},
  );
  await client.initialize();
  final store = SessionStore(includePreview: false);
  final remote = await client.createSession(cwd: '/w');
  final session = store.addProviderSession(title: 'Now', agent: 'Test', peerId: client.peerId, remoteSessionId: remote, cwd: '/w');
  store.attachVerifiedClient(session.id, client);
  return (store, session, agent, client);
}

const _earlier = AcpSessionInfo(id: 'old', cwd: '/w', title: 'From before');

void main() {
  group('earlier conversations', () {
    test('asks where this one works, and leaves itself out', () async {
      final (store, session, agent, _) = await _open();
      agent.listed = [
        {'sessionId': 'now', 'cwd': '/w', 'title': 'This one'},
        {'sessionId': 'old', 'cwd': '/w', 'title': 'From before'},
      ];
      final page = await store.earlierSessions(session);
      expect(page.sessions.map((item) => item.id), ['old']);
      expect(agent.received.last['params'], {'cwd': '/w'});
    });

    test('one opens beside this, with what was said before, on the same agent', () async {
      final (store, session, agent, _) = await _open();
      final opened = await store.openEarlier(session, _earlier);
      expect(store.sessions.length, 2);
      expect(store.selected, opened);
      expect(opened.title, 'From before');
      expect(opened.status, SessionStatus.connected);
      expect(opened.messages.map((message) => (message.role, message.text)), [
        (MessageRole.user, 'asked before'),
        (MessageRole.agent, 'said before'),
      ]);
      expect(session.messages, isEmpty, reason: 'what is replayed belongs to the one it is about');
      await store.sendText('more');
      expect(agent.received.last['params']['sessionId'], 'old');
    });

    test('one already open is selected, not opened twice', () async {
      final (store, session, agent, _) = await _open();
      final opened = await store.openEarlier(session, _earlier);
      store.select(session.id);
      final loads = agent.received.where((message) => message['method'] == 'session/load').length;
      expect(await store.openEarlier(session, _earlier), opened);
      expect(store.selected, opened);
      expect(agent.received.where((message) => message['method'] == 'session/load').length, loads);
      expect(store.sessions.length, 2);
    });

    test('one that cannot be opened leaves nothing behind, and this one goes on', () async {
      final (store, session, agent, _) = await _open();
      agent.refuseOpen = true;
      await expectLater(store.openEarlier(session, _earlier), throwsA(isA<AcpFailure>()));
      expect(store.sessions, [session]);
      expect(store.selected, session);
      await store.sendText('still here');
      expect(agent.received.last['params']['sessionId'], 'now');
    });

    test('a title cannot hide what is shown around it', () async {
      final (store, session, _, _) = await _open();
      final opened = await store.openEarlier(session, AcpSessionInfo(id: 'x', cwd: '/w', title: 'safe\u202egnp.exe\u0000'));
      expect(opened.title.contains('\u202e'), isFalse);
      expect(opened.title.contains('\u0000'), isFalse);
    });

    testWidgets('the list shows what the agent said and picks one', (tester) async {
      AcpSessionInfo? picked;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async {
          picked = await showDialog<AcpSessionInfo>(context: context, builder: (_) => EarlierConversations(
            list: (cursor) async => cursor == null
                ? const AcpSessionPage([_earlier], 'next')
                : const AcpSessionPage([AcpSessionInfo(id: 'older', title: 'Older still')], null),
          ));
        },
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('From before'), findsOneWidget);
      await tester.tap(find.text('More'));
      await tester.pumpAndSettle();
      expect(find.text('Older still'), findsOneWidget);
      expect(find.text('More'), findsNothing);
      await tester.tap(find.text('Older still'));
      await tester.pumpAndSettle();
      expect(picked?.id, 'older');
    });

    testWidgets('a connection that does not offer it says so', (tester) async {
      await tester.pumpWidget(MaterialApp(home: EarlierConversations(list: (_) => Future.error(const AcpFailure('no')))));
      await tester.pumpAndSettle();
      expect(find.text('This connection does not offer that.'), findsOneWidget);
    });
  });

  group('attachments', () {
    Map<String, dynamic> lastPrompt(_Agent agent) =>
        agent.received.lastWhere((message) => message['method'] == 'session/prompt');

    test('go with the next message, once, and say so in the conversation', () async {
      final (store, session, agent, _) = await _open();
      expect(store.addAttachment(session, 'notes.txt', utf8.encode('hello')), isNull);
      expect(store.addAttachment(session, 'a.PNG', [1, 2, 3]), isNull);
      await store.sendText('look');
      final prompt = lastPrompt(agent)['params']['prompt'] as List<dynamic>;
      expect(prompt.map((block) => block['type']), ['text', 'resource', 'image']);
      expect(prompt[1]['resource']['text'], 'hello');
      expect(prompt[2]['mimeType'], 'image/png');
      expect(session.attachments, isEmpty);
      expect(session.messages.last.text, contains('notes.txt, a.PNG'));
      await store.sendText('next');
      expect((lastPrompt(agent)['params']['prompt'] as List<dynamic>).length, 1);
    });

    test('can go with no words', () async {
      final (store, session, agent, _) = await _open();
      store.addAttachment(session, 'notes.txt', utf8.encode('hello'));
      await store.sendText('  ');
      expect((lastPrompt(agent)['params']['prompt'] as List<dynamic>).single['type'], 'resource');
    });

    test('are refused for what the agent does not take, what is too large, and what is not text', () async {
      final (store, session, _, _) = await _open(with_: _Agent(images: false, files: true));
      expect(store.addAttachment(session, 'a.png', [1]), contains('does not take images'));
      expect(store.addAttachment(session, 'a.bin', [0xff, 0xfe, 0x00]), contains('not text'));
      expect(store.addAttachment(session, 'big.txt', List.filled(maxAttachedText + 1, 97)), contains('larger'));
      expect(store.addAttachment(session, 'empty.txt', const []), contains('empty'));
      expect(session.attachments, isEmpty);
      final (other, there, _, _) = await _open(with_: _Agent(images: true, files: false));
      expect(other.addAttachment(there, 'a.txt', [97]), contains('does not take files'));
      expect(other.addAttachment(there, 'big.png', List.filled(maxAttachedImage + 1, 1)), contains('larger'));
    });

    test('are bounded in number, not repeated, and can be taken back', () async {
      final (store, session, agent, _) = await _open();
      expect(store.addAttachment(session, '0.txt', [97]), isNull);
      expect(store.addAttachment(session, '0.txt', [97]), contains('already'));
      for (var i = 1; i < maxAttachments; i++) {
        expect(store.addAttachment(session, '$i.txt', [97]), isNull);
      }
      expect(store.addAttachment(session, 'one more.txt', [97]), contains('as many'));
      store.removeAttachment(session, session.attachments.first);
      expect(store.addAttachment(session, '0.txt', [97]), isNull);
      for (final attachment in List.of(session.attachments)) {
        store.removeAttachment(session, attachment);
      }
      await store.sendText('nothing with it');
      expect((lastPrompt(agent)['params']['prompt'] as List<dynamic>).length, 1);
    });

    test('an agent that takes nothing has no way to attach', () async {
      final (store, session, _, _) = await _open(with_: _Agent(images: false, files: false));
      expect(store.canAttach(session), isFalse);
    });
  });
}
