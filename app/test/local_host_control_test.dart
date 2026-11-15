// The session list the panel keeps in the Canoa on this computer, over the extension's worker: the control channel's requests go
// to the bridge one by one and its answers come back matched by number, with a refusal kept as the host's own error.
import 'dart:convert';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/extension/local_host.dart';
import 'package:canoa/extension/local_host_control.dart';
import 'package:canoa/ledger/host_ledger_backend.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Worker implements LocalHostBridge {
  final asked = <(String, Map<String, Object?>)>[];
  final records = <String, Object?>{};

  @override
  Future<Object?> control(String method, Map<String, Object?> params) async {
    asked.add((method, params));
    switch (method) {
      case 'ledger/put':
        records['${params['id']}'] = params['record'];
        return {'ok': true};
      case 'ledger/list':
        return {
          'records': [for (final entry in records.entries) {'id': entry.key, 'record': entry.value}],
        };
      case 'host/restart':
        throw Exception('the host refused: not granted (-32001)');
      default:
        return {};
    }
  }

  @override
  Future<LocalHostView> status() => throw UnimplementedError();
  @override
  Future<LocalHostView> start() => throw UnimplementedError();
  @override
  Future<LocalHostView> pair(String pin) => throw UnimplementedError();
  @override
  Future<LocalHostView> unpair() => throw UnimplementedError();
}

void main() {
  test('a control client over the worker gets each answer by its number, and a refusal as an error', () async {
    final worker = _Worker();
    final client = await localHostControl(worker).ready();
    expect(await client.call('ledger/put', {'id': 'a', 'record': {'title': 'one'}}), {'ok': true});
    final listed = await client.call('ledger/list', {}) as Map;
    expect(jsonEncode(listed), contains('"id":"a"'));
    await expectLater(client.call('host/restart', {}), throwsA(isA<ControlFailed>().having((e) => e.message, 'message', contains('not granted'))));
    expect(worker.asked.map((call) => call.$1), ['ledger/put', 'ledger/list', 'host/restart']);
    await client.close();
  });

  test('the panel\'s session list is written to the host and read back from it', () async {
    final worker = _Worker();
    final backend = HostLedgerBackend.over(localHostControl(worker));
    await backend.write(jsonEncode({
      'sessions': [
        {'peerId': 'sha256:aa', 'sessionId': 's1', 'title': 'a session'},
      ],
    }));
    expect(worker.asked.any((call) => call.$1 == 'ledger/put'), isTrue);
    final read = await backend.read();
    expect(read, contains('a session'));
  });
}
