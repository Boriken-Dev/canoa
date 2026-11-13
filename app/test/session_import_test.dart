// Bringing in earlier sessions through the host's control channel, scripted: what the app asks and when, what it makes of the
// answers, and how an imported Claude and an imported Codex session show up in Earlier conversations and open.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/chat/session_views.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/history/session_import.dart';
import 'package:canoa/history/session_move.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A host's control channel that answers each method from [answers] and records what it was asked, in order.
final class _Host implements HostLink {
  _Host(this.answers);

  final Map<String, Object? Function(Map<String, Object?> params)> answers;
  final asked = <String>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    asked.add(method);
    final answer = answers[method];
    _out.add(jsonEncode(answer == null
        ? {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32601, 'message': 'no such method'}}
        : {'jsonrpc': '2.0', 'id': request['id'], 'result': answer((request['params'] as Map).cast())}));
  }

  @override
  Future<void> close() async => _out.close();
}

const _sources = <ImportSource>[
  (entry: 'Claude', harness: 'claude', label: 'Claude'),
  (entry: 'Codex', harness: 'codex', label: 'Codex'),
];

const _claudeId = '6f2c1b0e-0000-4000-8000-00000000c0de';
const _codexId = '01a1cb7b-a5cd-7ec1-b69a-c112db39d6d3';

const _history = '{"format":"coqui-acp-history","version":1,"id":"$_claudeId","origin":{"harness":"claude"}}\n'
    '{"method":"session/prompt","params":{"prompt":[{"type":"text","text":"Fix the failing build"}]}}\n'
    '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Fixed it."}}}}\n'
    '{"method":"history/turn_end","params":{"stopReason":"end_turn"}}\n';

_Host _scripted() => _Host({
  'history/importable': (params) => {
    'sessions': [
      if (params['harness'] == 'claude') {'id': _claudeId, 'title': 'Fix the failing build', 'cwd': '/work/app', 'imported': false},
      if (params['harness'] == 'codex') {'id': _codexId, 'title': 'Add a health endpoint', 'cwd': '/work/api', 'imported': true},
      {'id': 7, 'title': 'not a session'},
      'nonsense',
    ],
  },
  'history/import': (params) => {
    'imported': [
      for (final id in params['ids']! as List<Object?>)
        {'id': id, 'title': 'Fix the failing build', 'turns': 1, 'notCarried': {'lines': id == _claudeId ? 2 : 0}},
    ],
    'refused': [
      {'id': 'gone', 'why': 'it holds no conversation'},
    ],
  },
  'history/imported': (params) => {
    'sessions': [
      {'session': params['entry'] == 'Claude' ? _claudeId : _codexId, 'title': 'Imported ${params['entry']}', 'cwd': '/w', 'at': '2026-11-05T09:45:11Z'},
    ],
  },
  'history/export': (params) => {'id': params['session'], 'text': _history},
});

void main() {
  test('nothing is asked of the host until a list is wanted, and a list is titles and folders', () async {
    final host = _scripted();
    final control = ControlClient(host);
    addTearDown(control.close);
    expect(host.asked, isEmpty);
    final found = await listImportable(control, _sources);
    expect(host.asked, ['history/importable', 'history/importable']);
    expect([for (final each in found) (each.source, each.id, each.title, each.cwd, each.imported)], [
      ('Claude', _claudeId, 'Fix the failing build', '/work/app', false),
      ('Codex', _codexId, 'Add a health endpoint', '/work/api', true),
    ]);
  });

  test('an import says how many came and what did not, in sentences', () async {
    final host = _scripted();
    final control = ControlClient(host);
    addTearDown(control.close);
    final found = await listImportable(control, _sources);
    host.asked.clear();
    final done = await importChosen(control, [found.first]);
    expect(host.asked, ['history/import']);
    expect(done.imported, 1);
    expect(done.notes, [
      'Fix the failing build: 2 lines of its file could not be read and did not carry.',
      'A session: not imported. it holds no conversation',
    ]);
  });

  test('a host that refuses is heard, and a malformed answer is no list', () async {
    final control = ControlClient(_Host({'history/importable': (_) => 'nope'}));
    addTearDown(control.close);
    await expectLater(listImportable(control, _sources), throwsStateError);
    final none = ControlClient(_Host({}));
    addTearDown(none.close);
    await expectLater(listImportable(none, _sources), throwsA(isA<ControlFailed>()));
  });

  test('Earlier conversations lists what was imported first, then what the agent has, and survives an agent that cannot list', () async {
    final control = ControlClient(_scripted());
    addTearDown(control.close);
    final seen = <String>{};
    final page = await earlierPage(
      null,
      imported: () => importedSessions(control, 'Claude'),
      agent: (_) async => const AcpSessionPage([AcpSessionInfo(id: 'own', title: 'The agent\'s own')], 'more'),
      onImported: seen.addAll,
    );
    expect([for (final each in page.sessions) each.title], ['Imported Claude', 'The agent\'s own']);
    expect(page.nextCursor, 'more');
    expect(seen, {_claudeId});
    final later = await earlierPage('more', imported: () => importedSessions(control, 'Claude'), agent: (_) async => const AcpSessionPage([], null));
    expect(later.sessions, isEmpty, reason: 'what was imported is on the first page only');
    final failing = await earlierPage(null, imported: () => importedSessions(control, 'Codex'), agent: (_) => Future.error(const AcpFailure('no')));
    expect([for (final each in failing.sessions) each.id], [_codexId]);
    await expectLater(earlierPage(null, agent: (_) => Future.error(const AcpFailure('no'))), throwsA(isA<AcpFailure>()));
  });

  testWidgets('an imported Claude and an imported Codex session appear in Earlier conversations and open as what was said', (tester) async {
    final host = _scripted();
    final control = ControlClient(host);
    addTearDown(control.close);
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    for (final entry in ['Claude', 'Codex']) {
      final brought = <String>{};
      AcpSessionInfo? picked;
      await tester.pumpWidget(MaterialApp(
        home: Builder(builder: (context) => TextButton(
          onPressed: () async => picked = await showDialog<AcpSessionInfo>(
            context: context,
            builder: (_) => EarlierConversations(
              list: (cursor) => earlierPage(cursor, imported: () => importedSessions(control, entry), onImported: brought.addAll),
            ),
          ),
          child: const Text('open'),
        )),
      ));
      await tester.tap(find.text('open'));
      await tester.pump();
      await tester.pump();
      expect(find.text('Earlier conversations'), findsOneWidget);
      expect(find.text('Imported $entry'), findsOneWidget);
      await tester.tap(find.text('Imported $entry'));
      await tester.pumpAndSettle();
      expect(picked?.id, entry == 'Claude' ? _claudeId : _codexId);
      expect(brought, contains(picked!.id));
      final said = historyMessages(await sessionHistory(control, entry: entry, session: picked!.id));
      final shown = store.showImported(title: picked!.title!, agent: entry, said: said);
      expect(shown.status, SessionStatus.preview);
      expect(shown.messages.map((message) => message.text), containsAll(['Fix the failing build', 'Fixed it.']));
      expect(shown.messages.first.text, contains('Nothing typed here reaches an agent'));
    }
    expect(store.sessions.map((session) => session.title), ['Imported Codex', 'Imported Claude']);
  });
}
