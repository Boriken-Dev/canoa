// The models and efforts of a source before it starts are asked of the host over its control channel, as the account chosen, so a
// page that only reaches a host lists them as the desktop's own app does; a host that cannot answer leaves the old ways.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/control_choices.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/source_choices.dart';

/// A control channel that answers `options/choices` with [answer] and records the params; `code` makes it an error.
final class _Control implements HostLink {
  _Control({this.answer, this.code});

  final Map<String, Object?>? answer;
  final int? code;
  final asked = <Map<String, Object?>>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    if (method == 'options/choices') asked.add((request['params'] as Map).cast<String, Object?>());
    final reply = switch (method) {
      'options/choices' when code == null => {'result': answer},
      'ledger/list' => {'result': {'records': <Object?>[]}},
      'host/grants' => {'result': {'grants': <String>[]}},
      _ => {'error': {'code': code ?? -32601, 'message': 'no'}},
    };
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

const _claude = AgentSource.hosted('claude', harness: 'claude');

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

void main() {
  test('the host\'s answer is the source\'s choices, asked as the account named', () async {
    final link = _Control(answer: claudeAnswer);
    final control = HostControl.dialing(() async => link);
    final got = await choicesOverControl(control, _claude, account: 'work');
    expect(got!.known, isTrue);
    expect(got.models.map((model) => model.id), contains('sonnet'));
    expect(link.asked, [
      {'entry': 'claude', 'account': 'work'},
    ]);
    await control.close();
  });

  test('the machine\'s own login and no account are asked with none', () async {
    final link = _Control(answer: claudeAnswer);
    final control = HostControl.dialing(() async => link);
    await choicesOverControl(control, _claude);
    await choicesOverControl(control, _claude, account: '');
    expect(link.asked, [
      {'entry': 'claude'},
      {'entry': 'claude'},
    ]);
    await control.close();
  });

  test('a host with no such method, or a refusal, or no link, answers nothing so the caller asks another way', () async {
    for (final link in [_Control(code: -32601), _Control(code: -32001)]) {
      final control = HostControl.dialing(() async => link);
      expect(await choicesOverControl(control, _claude), isNull);
      await control.close();
    }
    expect(await choicesOverControl(HostControl.dialing(() async => throw StateError('down')), _claude), isNull);
  });

  testWidgets('a page served by a host lists the models before a session starts', (tester) async {
    final link = _Control(answer: claudeAnswer);
    final host = ServedHost(
      peer: 'sha256:${'a' * 64}',
      method: PeerMethod.noise,
      entries: const [ServedEntry(name: 'Claude', kind: 'agent', url: 'ws://127.0.0.1:1/claude', harness: 'claude')],
      control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
    );
    await pumpApp(
      tester,
      _store(),
      host: ServedSupervisor(host),
      served: ServedApp.over(host, HostControl.dialing(() async => link)),
      hostChoices: true,
    );
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await settle(tester);
    final pill = [
      for (final element in find.descendant(of: find.byKey(const Key('option-model')), matching: find.byType(Text)).evaluate())
        (element.widget as Text).data,
    ].join();
    expect(pill, isNot(contains('after start')), reason: pill);
    expect(link.asked.map((asked) => asked['entry']), contains('Claude'));
    await finish(tester);
  });
}
