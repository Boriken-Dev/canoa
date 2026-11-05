// The session ledger: harness names, what an agent says of itself, the bound, text kept only when turned on, the file backend
// across two ledgers as across two runs, and the store noting and reopening sessions through it.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/ledger_backend.dart';
import 'package:canoa/ledger/ledger_backend_io.dart' show FileLedgerBackend;
import 'package:canoa/ledger/ledger_list.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// An agent answering `initialize` with [initialized], keeping sessions it can list and load.
final class _Agent {
  _Agent(this.initialized);

  final Map<String, Object?> initialized;
  final toClient = StreamController<String>();
  final methods = <String>[];
  final listed = <Map<String, Object?>>[];
  var _next = 0;

  void _update(String session, Map<String, Object?> update) => toClient.add(jsonEncode({
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {'sessionId': session, 'update': update},
  }));

  Future<void> send(String text) async {
    final message = jsonDecode(text) as Map<String, dynamic>;
    final id = message['id'];
    if (id == null || message['method'] == null) return;
    methods.add(message['method'] as String);
    void answer(Object? result) => toClient.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
    final params = message['params'] as Map<String, dynamic>;
    switch (message['method']) {
      case 'initialize':
        answer(initialized);
      case 'session/new':
        answer({'sessionId': 's${++_next}'});
      case 'session/list':
        answer({'sessions': listed});
      case 'session/load' || 'session/resume':
        _update(params['sessionId'] as String, {'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': 'asked before'}});
        answer(<String, Object?>{});
      case 'session/prompt':
        final session = params['sessionId'] as String;
        _update(session, {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'the answer'}});
        answer({'stopReason': 'end_turn'});
    }
  }

  VerifiedAgent client(String label) =>
      VerifiedAgent.fromChild(label: label, messages: toClient.stream, send: send, close: () async {});
}

Map<String, Object?> _v1({Map<String, Object?>? info, Map<String, Object?> capabilities = const {'loadSession': true}, Map<String, Object?>? meta}) => {
  'protocolVersion': 1,
  'agentCapabilities': capabilities,
  'agentInfo': ?info,
  '_meta': ?meta,
};

const _claude = {'name': '@agentclientprotocol/claude-agent-acp', 'title': 'Claude Agent', 'version': '0.99.1'};

/// A clock that moves a minute each time it is read.
DateTime Function() _ticking() {
  var at = DateTime.utc(2026, 11, 3, 12);
  return () => at = at.add(const Duration(minutes: 1));
}

void main() {
  group('harness names', () {
    test('the adapters that name themselves, Gemini, OpenCode and Grok each have their harness', () {
      final names = HarnessNames();
      expect(names.label(reportedName: '@agentclientprotocol/claude-agent-acp', entry: 'Claude'), 'Claude Code');
      expect(names.label(reportedName: '@agentclientprotocol/codex-acp', entry: 'Codex'), 'Codex');
      expect(names.label(reportedName: 'gemini-cli', entry: 'x'), 'Gemini');
      expect(names.label(reportedName: 'pi-acp', entry: 'x'), 'Pi');
      expect(names.label(reportedName: 'OpenCode', entry: 'x'), 'OpenCode');
      expect(names.label(grokShell: true, entry: 'x'), 'Grok');
    });

    test('an unknown or absent name falls back to the entry, and canoa.cfg extends and overrides the map', () {
      expect(HarnessNames().label(reportedName: 'my-acp', entry: 'Mine'), 'Mine');
      expect(HarnessNames().label(entry: 'Mine'), 'Mine');
      final names = HarnessNames({'my-acp': 'My Harness', 'pi-acp': 'Pi (adapter)'});
      expect(names.label(reportedName: 'my-acp', entry: 'Mine'), 'My Harness');
      expect(names.label(reportedName: 'pi-acp', entry: 'x'), 'Pi (adapter)');
    });
  });

  group('what an agent says of itself', () {
    Future<AgentIdentity> initialized(Map<String, Object?> answer) async {
      final client = _Agent(answer).client('Test');
      await client.initialize();
      return client.identity;
    }

    test('v1 agentInfo, and session/load when it loads', () async {
      final identity = await initialized(_v1(info: _claude));
      expect((identity.name, identity.title, identity.version), ('@agentclientprotocol/claude-agent-acp', 'Claude Agent', '0.99.1'));
      expect(identity.reopenBy, 'session/load');
    });

    test('v1 with resume only, and with neither', () async {
      expect((await initialized(_v1(capabilities: {'sessionCapabilities': {'resume': <String, Object?>{}}}))).reopenBy, 'session/resume');
      final none = await initialized(_v1(capabilities: const {}));
      expect((none.name, none.version, none.reopenBy), (null, null, null));
    });

    test('v2 info, which always resumes', () async {
      final identity = await initialized({
        'protocolVersion': 2,
        'info': {'name': 'agent', 'version': '1'},
        'capabilities': {'session': <String, Object?>{}},
      });
      expect((identity.name, identity.version, identity.reopenBy), ('agent', '1', 'session/resume'));
    });

    test('Grok: no agentInfo, its version and shell in _meta', () async {
      final identity = await initialized(_v1(meta: {'agentVersion': '1.0.70', 'grokShell': true}));
      expect((identity.name, identity.grokShell, identity.version), (null, true, '1.0.70'));
    });
  });

  group('the ledger', () {
    test('notes a session by harness and version, and a second ledger on the same store reads it back', () async {
      final backend = MemoryLedgerBackend();
      final ledger = SessionLedger(backend, clock: _ticking());
      ledger.record(
        peerId: 'local:Claude', sessionId: 's1', entry: 'Claude', place: SessionPlace.local,
        reportedName: '@agentclientprotocol/claude-agent-acp', version: '0.99.1', reopenBy: 'session/load', cwd: '/w', title: 'Fix it',
      );
      await ledger.flush();
      final again = SessionLedger(backend);
      await again.load();
      final entry = again.entries.single;
      expect((entry.harness, entry.versionLabel, entry.entry, entry.place, entry.cwd, entry.sessionId, entry.title),
          ('Claude Code', 'claude-agent-acp 0.99.1', 'Claude', SessionPlace.local, '/w', 's1', 'Fix it'));
      expect(entry.canReopen, isTrue);
      expect(entry.created.isAtSameMomentAs(entry.lastUsed), isTrue);
    });

    test('saves by itself shortly after a change', () async {
      final backend = MemoryLedgerBackend();
      SessionLedger(backend).record(peerId: 'p', sessionId: 's', entry: 'e', place: SessionPlace.peer);
      await Future<void>.delayed(SessionLedger.saveDelay * 3);
      expect(backend.document, contains('"sessionId": "s"'));
    });

    test('keeps at most `keep`, dropping the least recently used, and none at 0', () {
      final ledger = SessionLedger(MemoryLedgerBackend(), settings: const LedgerSettings(keep: 2), clock: _ticking());
      for (final id in ['a', 'b', 'c']) {
        ledger.record(peerId: 'p', sessionId: id, entry: 'e', place: SessionPlace.peer);
      }
      ledger.touch('p', 'b');
      ledger.record(peerId: 'p', sessionId: 'd', entry: 'e', place: SessionPlace.peer);
      expect([for (final entry in ledger.entries) entry.sessionId], ['d', 'b']);
      ledger.configure(const LedgerSettings(keep: 0));
      expect(ledger.entries, isEmpty);
      expect(ledger.record(peerId: 'p', sessionId: 'e', entry: 'e', place: SessionPlace.peer), isNull);
    });

    test('keeps no text unless turned on, then the latest, each bounded, and clears it', () async {
      final backend = MemoryLedgerBackend();
      final ledger = SessionLedger(backend);
      ledger.record(peerId: 'p', sessionId: 's', entry: 'e', place: SessionPlace.peer);
      final said = [for (var n = 0; n < 5; n++) KeptMessage('user', 'secret $n', DateTime.utc(2026))];
      ledger.keepText('p', 's', said);
      await ledger.flush();
      expect(ledger.entries.single.messages, isEmpty);
      expect(backend.document, isNot(contains('secret')));

      ledger.configure(const LedgerSettings(keepText: true, textMessages: 3));
      ledger.keepText('p', 's', [...said, KeptMessage('agent', 'x' * (SessionLedger.maxKeptMessage + 10), DateTime.utc(2026))]);
      expect([for (final kept in ledger.entries.single.messages) kept.text.length > 20 ? 'long' : kept.text], ['secret 3', 'secret 4', 'long']);
      expect(ledger.entries.single.messages.last.text.length, SessionLedger.maxKeptMessage);
      await ledger.flush();
      expect(backend.document, contains('secret 4'));

      ledger.clearText();
      await ledger.flush();
      expect(ledger.holdsText, isFalse);
      expect(backend.document, isNot(contains('secret')));

      ledger.keepText('p', 's', said);
      ledger.configure(const LedgerSettings());
      await ledger.flush();
      expect(backend.document, isNot(contains('secret')), reason: 'turning it off clears what was kept');
    });

    test('removing an entry takes only the note', () {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.record(peerId: 'p', sessionId: 's', entry: 'e', place: SessionPlace.peer);
      ledger.remove(ledger.entries.single);
      expect(ledger.entries, isEmpty);
    });

    test('a document that is not a session list starts an empty list and says why', () async {
      final ledger = SessionLedger(MemoryLedgerBackend()..document = '{"version": 7}');
      await ledger.load();
      expect(ledger.entries, isEmpty);
      expect(ledger.problem, contains('not a session list'));
    });

    test('says plainly when a session cannot be opened again', () {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final none = ledger.record(peerId: 'local:Old', sessionId: 's', entry: 'Old', place: SessionPlace.local)!;
      expect(none.canReopen, isFalse);
      expect(none.whyNotReopened, 'Cannot be reopened: Old offers neither session/load nor session/resume.');
      final browser = ledger.record(peerId: 'b', sessionId: 's', entry: 'B', place: SessionPlace.browser, reopenBy: 'session/load')!;
      expect(browser.canReopen, isFalse);
      expect(browser.whyNotReopened, contains('Earlier conversations'));
    });
  });

  group('the file backend', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('canoa-ledger-'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('lives beside canoa.cfg', () {
      final backend = platformLedgerBackend(configPath: '${dir.path}${Platform.pathSeparator}canoa.cfg');
      expect((backend as FileLedgerBackend).path, '${dir.path}${Platform.pathSeparator}sessions.json');
      expect(platformLedgerBackend(), isA<MemoryLedgerBackend>());
    });

    test('a ledger written by one run is read by the next, and nothing is left beside it', () async {
      final path = '${dir.path}${Platform.pathSeparator}Canoa${Platform.pathSeparator}sessions.json';
      final first = SessionLedger(FileLedgerBackend(path), clock: _ticking());
      first.record(peerId: 'local:Pi', sessionId: 's1', entry: 'Pi', place: SessionPlace.local, reportedName: 'pi-acp', version: '0.0.34', reopenBy: 'session/load');
      first.record(peerId: 'sha256:aa', sessionId: 's2', entry: 'Host agent', place: SessionPlace.host,
          endpoint: 'ws://127.0.0.1:7801', method: 'noise', reopenBy: 'session/resume');
      await first.flush();
      expect(Directory('${dir.path}${Platform.pathSeparator}Canoa').listSync().map((file) => file.uri.pathSegments.last), ['sessions.json']);
      final second = SessionLedger(FileLedgerBackend(path));
      await second.load();
      expect([for (final entry in second.entries) (entry.harness, entry.place, entry.peerId)],
          [('Host agent', SessionPlace.host, 'sha256:aa'), ('Pi', SessionPlace.local, 'local:Pi')]);
      expect(second.entries.first.endpoint, 'ws://127.0.0.1:7801');
    });

    test('nothing written yet reads as an empty list', () async {
      final ledger = SessionLedger(FileLedgerBackend('${dir.path}${Platform.pathSeparator}sessions.json'));
      await ledger.load();
      expect(ledger.entries, isEmpty);
      expect(ledger.problem, isNull);
    });
  });

  group('canoa.cfg', () {
    test('[sessions] and [harnesses] are read, checked and written back', () {
      final config = CanoaConfig.parse('[sessions]\nkeep = 20\nkeep_text = true\ntext_messages = 5\n\n'
          '[harnesses]\n"my-acp" = "My Harness"\n');
      expect((config.sessions.keep, config.sessions.keepText, config.sessions.textMessages), (20, true, 5));
      expect(config.sessions.harnesses, {'my-acp': 'My Harness'});
      final again = CanoaConfig.parse(config.encode());
      expect((again.sessions.keep, again.sessions.keepText), (20, true));
      expect(again.sessions.harnesses, {'my-acp': 'My Harness'});
      expect(const CanoaConfig().sessions.keepText, isFalse);
      expect(const CanoaConfig().sessions.keep, LedgerSettings.defaultKeep);
      for (final bad in ['[sessions]\nkeep = -1\n', '[sessions]\nkeep_text = "yes"\n', '[harnesses]\nx = 3\n', '[sessions]\nother = 1\n']) {
        expect(() => CanoaConfig.parse(bad), throwsFormatException, reason: bad);
      }
    });
  });

  group('the store notes sessions', () {
    test('a session opened here is noted with what its agent said, its text only when turned on', () async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(includePreview: false, ledger: ledger);
      final agent = _Agent(_v1(info: _claude));
      final client = agent.client('Claude');
      await client.initialize();
      final opened = await client.openSession(cwd: '/w');
      final session = store.addProviderSession(title: 'Fix it', agent: 'Claude', peerId: client.peerId, remoteSessionId: opened.id, cwd: '/w');
      store.attachVerifiedClient(session.id, client);
      final entry = ledger.entries.single;
      expect((entry.harness, entry.version, entry.place, entry.cwd, entry.reopenBy, entry.title),
          ('Claude Code', '0.99.1', SessionPlace.local, '/w', 'session/load', 'Fix it'));
      await store.sendText('hello');
      await pumpEventQueue();
      expect(entry.messages, isEmpty);
      ledger.configure(const LedgerSettings(keepText: true));
      await store.sendText('again');
      await pumpEventQueue();
      expect([for (final kept in entry.messages) kept.text], containsAllInOrder(['hello', 'the answer', 'again']));
    });

    test('a session from the list opens again on a new agent, by session/load, and one that cannot says so', () async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _ticking());
      final entry = ledger.record(
        peerId: 'local:Claude', sessionId: 'old', entry: 'Claude', place: SessionPlace.local,
        reportedName: '@agentclientprotocol/claude-agent-acp', reopenBy: 'session/load', cwd: '/w', title: 'Noted',
      )!;
      final store = SessionStore(includePreview: false, ledger: ledger);
      final agent = _Agent(_v1(info: _claude, capabilities: {'loadSession': true, 'sessionCapabilities': {'list': <String, Object?>{}}}));
      agent.listed.add({'sessionId': 'old', 'cwd': '/w', 'title': 'As the agent names it'});
      final client = agent.client('Claude');
      await client.initialize();
      final session = await store.reopen(entry, client);
      expect(agent.methods, containsAllInOrder(['initialize', 'session/list', 'session/load']));
      expect(session.title, 'As the agent names it');
      expect(session.messages.map((message) => message.text), contains('asked before'));
      expect(ledger.entries.single.lastUsed.isAfter(ledger.entries.single.created), isTrue);

      final plain = _Agent(_v1(capabilities: const {})).client('Claude');
      await plain.initialize();
      final other = ledger.record(peerId: 'local:Claude', sessionId: 'older', entry: 'Claude', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w')!;
      await expectLater(store.reopen(other, plain), throwsA(isA<AcpFailure>().having((error) => error.message, 'message', contains('neither session/load nor session/resume'))));
      expect(store.sessions, hasLength(1));
    });

    test('a session on canoa is noted with the host as its peer, over plain WebSocket on loopback', () {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(includePreview: false, ledger: ledger);
      final session = store.addRemoteSession(
        title: 'Hosted', agent: 'sha256:aa', peerId: 'sha256:aa', remoteSessionId: 'h1',
        endpoint: Uri.parse('ws://127.0.0.1:7801'), place: SessionPlace.host, method: 'noise', cwd: '/srv',
      );
      expect((session.place, session.method), (SessionPlace.host, 'noise'));
      expect(() => store.addRemoteSession(title: 'x', agent: 'x', peerId: 'p', remoteSessionId: 'r', endpoint: Uri.parse('ws://192.0.2.1:7801')),
          throwsArgumentError);
    });
  });

  group('the list', () {
    Future<SessionLedger> shown(WidgetTester tester, {Future<void> Function(LedgerEntry)? onReopen}) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _ticking());
      ledger.record(peerId: 'local:Claude', sessionId: 'c1', entry: 'Claude', place: SessionPlace.local,
          reportedName: '@agentclientprotocol/claude-agent-acp', version: '0.99.1', reopenBy: 'session/load', cwd: '/a', title: 'Claude one');
      ledger.record(peerId: 'local:Pi', sessionId: 'p1', entry: 'Pi', place: SessionPlace.local,
          reportedName: 'pi-acp', version: '0.0.34', reopenBy: 'session/load', cwd: '/b', title: 'Pi one');
      ledger.record(peerId: 'local:Plain', sessionId: 'x1', entry: 'Plain', place: SessionPlace.local, cwd: '/c', title: 'Plain one');
      await ledger.flush();
      final store = SessionStore(includePreview: false, ledger: ledger);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(
        child: LedgerList(ledger: ledger, store: store, onReopen: onReopen ?? (_) async {}),
      ))));
      return ledger;
    }

    testWidgets('groups by harness, filters by one, and says which cannot be reopened', (tester) async {
      await shown(tester);
      for (final harness in ['Claude Code', 'Pi', 'Plain']) {
        expect(find.byKey(ValueKey('harness $harness')), findsOneWidget);
      }
      expect(find.textContaining('claude-agent-acp 0.99.1'), findsOneWidget);
      expect(find.textContaining('Cannot be reopened: Plain offers neither session/load nor session/resume.'), findsOneWidget);
      expect(find.byTooltip('Reopen'), findsNWidgets(2));
      await tester.tap(find.widgetWithText(ChoiceChip, 'Pi'));
      await tester.pump();
      expect(find.text('Pi one'), findsOneWidget);
      expect(find.text('Claude one'), findsNothing);
      expect(find.byKey(const ValueKey('harness Claude Code')), findsNothing);
    });

    testWidgets('removing one says the agent keeps its session, and takes only the note', (tester) async {
      final ledger = await shown(tester);
      await tester.tap(find.descendant(of: find.byKey(ValueKey('session ${LedgerEntry.keyOf('local:Pi', 'p1')}')),
          matching: find.byTooltip('Remove from this list')));
      await tester.pumpAndSettle();
      expect(find.textContaining('This does not delete the session itself: Pi still has it.'), findsOneWidget);
      await tester.tap(find.text('Remove'));
      await tester.pumpAndSettle();
      expect(ledger.entries.map((entry) => entry.sessionId), isNot(contains('p1')));
      expect(find.text('Pi one'), findsNothing);
      await tester.pump(SessionLedger.saveDelay);
    });

    testWidgets('Reopen hands the entry over', (tester) async {
      final asked = <String>[];
      await shown(tester, onReopen: (entry) async => asked.add(entry.sessionId));
      await tester.tap(find.descendant(of: find.byKey(ValueKey('session ${LedgerEntry.keyOf('local:Claude', 'c1')}')),
          matching: find.byTooltip('Reopen')));
      await tester.pump();
      expect(asked, ['c1']);
    });
  });
}
