// The permission rules over a host that keeps them as `[permissions.rules.ID]` of canoa.cfg, with how often each fired kept apart.
// The host here is a stand-in that answers `rules/read` and `rules/write` from a map.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/config_rules_backend.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostLink {
  /// What the host's canoa.cfg holds under `[permissions.rules]`.
  Map<String, Object?> rules = {};
  int writes = 0;
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    final Object? result;
    switch (request['method']) {
      case 'rules/read':
        result = {'rules': rules};
      case 'rules/write':
        writes++;
        rules = (params['rules']! as Map).cast<String, Object?>();
        result = {'rules': rules.length};
      default:
        result = null;
    }
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() => _out.close();
}

void main() {
  late _Host host;
  late MemoryLedgerBackend use;
  late PermissionRules rules;

  late HostControl control;
  PermissionRules open() => PermissionRules(ConfigRulesBackend(control, use));

  setUp(() {
    host = _Host();
    control = HostControl.dialing(() async => host);
    use = MemoryLedgerBackend();
    addTearDown(host.close);
    rules = open();
  });

  PermissionRule? make(PermissionRules each) => each.add(
    source: 'claude',
    folder: '/work',
    kind: 'execute',
    type: RuleType.command,
    value: 'ls',
  );

  test('a rule made is a table of canoa.cfg, and a rule counted is not written to canoa.cfg again', () async {
    await rules.load();
    final rule = make(rules)!;
    await rules.settled();
    expect(host.rules.keys, [rule.id]);
    expect((host.rules[rule.id]! as Map).keys, containsAll(['source', 'folder', 'tool', 'type', 'value', 'made', 'by']));
    expect(host.rules[rule.id], isNot(contains('fired')), reason: 'a count is data, not a setting');
    expect(host.writes, 1);

    rules.recordUse(rule, 'ls');
    await rules.settled();
    expect(host.writes, 1, reason: 'a use does not change canoa.cfg');
    expect(jsonDecode((await use.read())!), {
      'version': 1,
      'use': {rule.id: containsPair('fired', 1)},
    });

    final again = open();
    await again.load();
    expect(again.rules.single.id, rule.id);
    expect(again.rules.single.fired, 1, reason: 'the count comes back from the data file');
    expect(again.rules.single.external, isFalse);
  });

  test('a rule revoked leaves canoa.cfg and its count', () async {
    await rules.load();
    final rule = make(rules)!;
    rules.recordUse(rule, 'ls');
    await rules.settled();
    expect(rules.revoke(rule.id), isTrue);
    await rules.settled();
    expect(host.rules, isEmpty);
    expect(jsonDecode((await use.read())!)['use'], isEmpty);
  });

  test('a rule written into canoa.cfg by hand is taken up and flagged', () async {
    host.rules = {
      'abcd': {'source': 'codex', 'folder': '/w', 'tool': 'execute', 'type': 'command', 'value': 'pwd', 'made': '2026-11-27T00:00:00Z'},
    };
    await rules.load();
    expect(rules.rules.single.value, 'pwd');
    expect(rules.rules.single.external, isTrue, reason: 'no by = canoa');
  });
}
