// A session the host keeps for several clients, against a scripted host that speaks the hub's wire: joining it from the list with
// the conversation replayed, coming back with only what was missed, who else is attached shown in the header, and a permission
// request another client answered closing here.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/presence.dart';
import 'package:canoa/ledger/ledger_list.dart';
import 'package:canoa/ledger/running_sessions.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

const _hub = 'dev.boriken.coqui.acp/hub';

/// What the host's hub does for one session, as one client sees it: the log with its cursors, the replay after a cursor, the
/// presence and the permission requests of every client.
final class _Host {
  final log = <Map<String, Object?>>[];
  final requests = <Map<String, dynamic>>[];
  final answers = <Object?, Map<String, dynamic>>{};
  StreamController<String> _toClient = StreamController<String>();
  var _seq = 0;

  /// Everyone attached, as presence says it; the first is the client the test drives.
  final others = <Map<String, Object?>>[];

  void _push(Map<String, Object?> message) {
    if (!_toClient.isClosed) _toClient.add(jsonEncode(message));
  }

  Map<String, Object?> _update(Map<String, Object?> body, int seq) => {
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {
      'sessionId': 's1',
      'update': body,
      '_meta': {_hub: {'seq': '$seq'}},
    },
  };

  /// One logged message, sent now to the client when it is attached.
  void say(Map<String, Object?> body) {
    log.add({'seq': ++_seq, 'body': body});
    _push(_update(body, _seq));
  }

  /// What another client said and what the agent answered, while this one is attached or not.
  void otherSays(String text) {
    say({'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': text}});
    say({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'echo: $text'}});
  }

  void presence() => _push({
    'jsonrpc': '2.0',
    'method': '_dev.boriken.coqui.acp/hub_presence',
    'params': {
      'sessionId': 's1',
      'clients': [
        {'id': 'sha256:self0000000000', 'label': 'shared', 'role': 'operator', 'self': true},
        ...others,
      ],
    },
  });

  void askPermission(int number, String toolCallId) => _push({
    'jsonrpc': '2.0',
    'id': 'hub-perm-$number',
    'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': toolCallId, 'title': 'Write a file', 'kind': 'edit', 'rawInput': {'file_path': 'a.txt'}},
      'options': [
        {'optionId': 'allow', 'name': 'Allow once', 'kind': 'allow_once'},
        {'optionId': 'reject', 'name': 'Reject', 'kind': 'reject_once'},
      ],
      '_meta': {_hub: {'request': number}},
    },
  });

  void closedBy(int number, String by, String optionId) => _push({
    'jsonrpc': '2.0',
    'method': '_dev.boriken.coqui.acp/hub_permission_closed',
    'params': {
      'sessionId': 's1',
      'requestId': 'hub-perm-$number',
      'by': by,
      'outcome': {'outcome': 'selected', 'optionId': optionId},
      '_meta': {_hub: {'seq': '${++_seq}'}},
    },
  });

  /// The host says an approver the person added settled the request about [toolCall].
  void approved(String toolCall, String by, String outcome) => _push({
    'jsonrpc': '2.0',
    'method': '_dev.boriken.canoa/approved',
    'params': {'sessionId': 's1', 'toolCallId': toolCall, 'by': by, 'outcome': outcome},
  });

  /// The link ends, as a closed connection does; the session goes on in the host.
  void dropLink() => unawaited(_toClient.close());

  Future<void> _send(String text) async {
    final message = jsonDecode(text) as Map<String, dynamic>;
    final id = message['id'];
    final method = message['method'];
    if (method == null) {
      if (id != null) answers[id] = message;
      return;
    }
    requests.add(message);
    void reply(Object? result) => _push({'jsonrpc': '2.0', 'id': id, 'result': result});
    final params = message['params'] as Map<String, dynamic>? ?? const {};
    switch (method) {
      case 'initialize':
        reply({
          'protocolVersion': 1,
          'agentCapabilities': {'loadSession': true, 'sessionCapabilities': {'resume': <String, Object?>{}}},
          'agentInfo': {'name': 'canoa-hub'},
        });
      case 'session/load':
        final cursor = ((params['_meta'] as Map?)?[_hub] as Map?)?['cursor'];
        final after = cursor is String ? int.parse(cursor) : 0;
        for (final entry in log) {
          if ((entry['seq']! as int) > after) _push(_update(entry['body']! as Map<String, Object?>, entry['seq']! as int));
        }
        reply({'_meta': {_hub: {'cursor': '$_seq', 'truncated': false}}});
        presence();
      case 'session/prompt':
        final text = ((params['prompt'] as List).first as Map)['text'] as String;
        say({'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': text}});
        say({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'echo: $text'}});
        reply({'stopReason': 'end_turn'});
      default:
        if (id != null) reply(<String, Object?>{});
    }
  }

  /// A connection to the host: each one starts a stream of its own.
  VerifiedAgent connect([AgentDecider? decider]) {
    _toClient = StreamController<String>();
    return VerifiedAgent.fromChild(label: 'Claude', messages: _toClient.stream, send: _send, close: () async {}, decider: decider);
  }

  String? get lastLoadCursor {
    final load = requests.lastWhere((request) => request['method'] == 'session/load');
    return (((load['params'] as Map)['_meta'] as Map?)?[_hub] as Map?)?['cursor'] as String?;
  }
}

List<String> _said(ChatSession session) => [
  for (final message in session.messages)
    if (message.role == MessageRole.user || message.role == MessageRole.agent) message.text,
];

LedgerEntry _entry(SessionLedger ledger) => ledger.record(
  peerId: 'local:Claude', sessionId: 's1', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/resume', cwd: '/w', title: 'Shared',
)!;

Future<void> _settle() async {
  for (var i = 0; i < 5; i++) {
    await pumpEventQueue();
  }
}

void main() {
  group('joining a session the host keeps', () {
    test('it is replayed in full, and a later join replays only what was missed', () async {
      final host = _Host()
        ..otherSays('one')
        ..others.add({'id': 'sha256:phone000000000', 'label': 'shared', 'role': 'operator'});
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final entry = _entry(ledger);

      final first = host.connect();
      await first.initialize();
      final session = await store.reopen(entry, first, attach: true);
      await _settle();
      expect(host.requests.map((request) => request['method']), containsAllInOrder(['initialize', 'session/load']));
      expect(host.lastLoadCursor, isNull, reason: 'nothing was kept, so all of it is replayed');
      expect(_said(session), ['one', 'echo: one']);
      expect(entry.cursor, '2');

      // What another client says while this one is attached arrives as it happens.
      host.otherSays('two');
      await _settle();
      expect(_said(session), ['one', 'echo: one', 'two', 'echo: two']);

      // The link ends; what is said meanwhile is missed.
      host.dropLink();
      await _settle();
      host.otherSays('three');
      final second = host.connect();
      await second.initialize();
      final again = await store.reopen(entry, second, attach: true);
      await _settle();
      expect(identical(again, session), isTrue, reason: 'the open conversation is attached again in place');
      expect(host.lastLoadCursor, '4', reason: 'it asks for what follows the last message it saw');
      expect(_said(session), ['one', 'echo: one', 'two', 'echo: two', 'three', 'echo: three']);
      expect(entry.cursor, '6');
      await ledger.flush();
    });

    test('with nothing kept of what was said before, the whole of it is asked for again', () async {
      final host = _Host()..otherSays('one');
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final entry = _entry(ledger);
      entry.cursor = '2';
      final client = host.connect();
      await client.initialize();
      final session = await store.reopen(entry, client, attach: true);
      await _settle();
      expect(host.lastLoadCursor, isNull, reason: 'a cursor with nothing before it would show half a conversation');
      expect(_said(session), ['one', 'echo: one']);
    });

    test('who else is attached follows what the host says, and leaves out this client', () async {
      final host = _Host()..otherSays('one');
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      final client = host.connect();
      await client.initialize();
      final session = await store.reopen(_entry(store.ledger!), client, attach: true);
      await _settle();
      expect(session.others, isEmpty);

      host.others.add({'id': 'sha256:phone0000aaaa', 'label': 'shared', 'role': 'operator'});
      host.presence();
      await _settle();
      expect([for (final other in session.others) other.id], ['sha256:phone0000aaaa']);

      host.others.clear();
      host.presence();
      await _settle();
      expect(session.others, isEmpty);
    });

    test('a request another client answered closes here, and says who answered', () async {
      final host = _Host()..otherSays('one');
      host.others.add({'id': 'sha256:phone0000aaaa', 'label': 'shared', 'role': 'operator'});
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      final client = host.connect(store.askPermission);
      await client.initialize();
      final session = await store.reopen(_entry(store.ledger!), client, attach: true);
      await _settle();

      host.askPermission(3, 't1');
      await _settle();
      expect(session.pendingPermissions, hasLength(1));
      expect(session.needsYou, isTrue);

      host.closedBy(3, 'sha256:phone0000aaaa', 'allow');
      await _settle();
      expect(session.pendingPermissions, isEmpty, reason: 'it is closed here');
      expect(session.needsYou, isFalse);
      expect(
        session.messages.map((message) => message.text),
        contains(allOf(contains('shared (phone0)'), contains('Write a file'), contains('Allow once'))),
      );
      // This client answers nothing for the host: what it sends is a refusal to choose, which the hub ignores once it is closed.
      expect((host.answers['hub-perm-3']!['result'] as Map)['outcome'], {'outcome': 'cancelled'});
    });
  });

  group('a request an approver settled', () {
    for (final (outcome, said) in [('allowed', 'Approved from phone'), ('rejected', 'Refused from phone'), ('refused', 'Refused, no answer in time')]) {
      test('closes the card and says it was $outcome', () async {
        final host = _Host()..otherSays('one');
        final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
        final client = host.connect(store.askPermission);
        await client.initialize();
        final session = await store.reopen(_entry(store.ledger!), client, attach: true);
        await _settle();

        host.askPermission(3, 't1');
        await _settle();
        expect(session.pendingPermissions, hasLength(1));

        host.approved('t1', 'phone', outcome);
        await _settle();
        expect(session.pendingPermissions, isEmpty);
        expect(session.messages.map((message) => message.text), contains(allOf(contains(said), contains('Write a file'))));
      });
    }

    test('another call is left open', () async {
      final host = _Host()..otherSays('one');
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      final client = host.connect(store.askPermission);
      await client.initialize();
      final session = await store.reopen(_entry(store.ledger!), client, attach: true);
      await _settle();
      host.askPermission(3, 't1');
      await _settle();
      host.approved('t2', 'phone', 'allowed');
      await _settle();
      expect(session.pendingPermissions, hasLength(1));
    });
  });

  group('the list of what the host keeps running', () {
    RunningSession running({List<Object?>? clients}) => RunningSession.parse({
      'entry': 'Claude',
      'session': 's1',
      'title': 'Fix the build',
      'cwd': '/w',
      'cursor': '9',
      'lastActivity': DateTime.now().toUtc().millisecondsSinceEpoch,
      'clients': clients ??
          [
            {'id': 'sha256:aaaaaa', 'label': 'Claude', 'role': 'operator'},
            {'id': 'sha256:bbbbbb', 'label': 'Claude', 'role': 'operator'},
          ],
      'turnRunning': true,
      'permissionsWaiting': 1,
    })!;

    test('the host\'s words are read, and what is not a session is left out', () {
      final parsed = running();
      expect((parsed.entry, parsed.sessionId, parsed.title, parsed.cwd, parsed.cursor), ('Claude', 's1', 'Fix the build', '/w', '9'));
      expect(parsed.clients.map((client) => client.short), ['aaaaaa', 'bbbbbb']);
      expect(parsed.turnRunning, isTrue);
      expect(RunningSession.parse({'entry': 'Claude'}), isNull);
      expect(RunningSession.parse('x'), isNull);
    });

    testWidgets('Earlier conversations lists it above the rest and joining hands it over', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.record(peerId: 'local:Claude', sessionId: 's1', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/resume', cwd: '/w', title: 'Mine');
      ledger.record(peerId: 'local:Claude', sessionId: 'old', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w', title: 'An old one');
      await ledger.flush();
      final store = SessionStore(ledger: ledger);
      final list = RunningSessions(() async => [running()]);
      addTearDown(list.dispose);
      final joined = <String>[];
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(
        child: LedgerList(ledger: ledger, store: store, running: list, onAttach: (session) async => joined.add(session.sessionId), onReopen: (_) async {}),
      ))));
      await tester.pump();
      await tester.pump();

      expect(find.text('Running now'), findsOneWidget);
      // The list's own name for it wins over the host's, and it is not listed twice.
      expect(find.text('Mine'), findsOneWidget);
      expect(find.byKey(ValueKey('session ${LedgerEntry.keyOf('local:Claude', 's1')}')), findsNothing);
      expect(find.textContaining('2 attached'), findsOneWidget);
      expect(find.textContaining('working'), findsOneWidget);
      expect(find.text('An old one'), findsOneWidget);

      await tester.tap(find.byTooltip('Join'));
      await tester.pump();
      expect(joined, ['s1']);
      await tester.pump(SessionLedger.saveDelay);
    });

    testWidgets('nothing is shown when the host keeps none', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final list = RunningSessions(() async => const []);
      addTearDown(list.dispose);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: LedgerList(ledger: ledger, store: store, running: list, onAttach: (_) async {}))));
      await tester.pump();
      expect(find.text('Running now'), findsNothing);
    });

    testWidgets('a host that cannot be asked shows none', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final list = RunningSessions(() async => throw StateError('no host'));
      addTearDown(list.dispose);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: LedgerList(ledger: ledger, store: store, running: list, onAttach: (_) async {}))));
      await tester.pump();
      expect(find.text('Running now'), findsNothing);
    });
  });

  group('the header', () {
    testWidgets('names who else is attached', (tester) async {
      final many = [
        const HubClient(id: 'sha256:aaaaaa0000', label: 'Claude', role: 'operator'),
        const HubClient(id: 'sha256:bbbbbb0000', label: 'Claude', role: 'operator'),
      ];
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: Row(children: [PresenceMark(others: many.take(1).toList(), narrow: false)]))));
      expect(find.text('With Claude (aaaaaa)'), findsOneWidget);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: Row(children: [PresenceMark(others: many, narrow: false)]))));
      expect(find.text('With 2 others'), findsOneWidget);
      expect(PresenceMark.message(many), 'Also attached: Claude (aaaaaa), Claude (bbbbbb)');
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: Row(children: [PresenceMark(others: const [], narrow: false)]))));
      expect(find.byKey(const Key('presence-mark')), findsNothing);
    });
  });
}
