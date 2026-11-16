// Where links lead and where a repository starts are the host's to say, for the remembered rules, which ask where nothing can wait:
// the paths a probe needs are found out, asked in batches, and answered again from what the host said.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart' show MemoryLedgerBackend;
import 'package:canoa/permissions/host_paths.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostLink {
  _Host(this.answer);

  final Object? Function(String method, Map<String, Object?> params) answer;
  final asked = <(String, Map<String, Object?>)>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    asked.add((method, params));
    final result = answer(method, params);
    final reply = result is String && result.startsWith('error:')
        ? {'error': {'code': -32003, 'message': result.substring(6)}}
        : {'result': result};
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

Object? _paths(String method, Map<String, Object?> params) {
  final list = (params['paths'] as List<Object?>?) ?? const [];
  return switch (method) {
    'fs/realpath' => {
      'paths': [
        for (final path in list) {'path': path, 'real': path.toString().startsWith('/alias') ? '/work${path.toString().substring(6)}' : path},
      ],
    },
    'fs/repo_root' => {
      'folders': [
        for (final path in list) {'path': path, 'root': path.toString().startsWith('/work') ? '/work' : null},
      ],
    },
    'fs/guards' => {'home': '/home/me', 'config': null},
    _ => 'error:unknown',
  };
}

void main() {
  test('answers nothing it was not asked for ahead, then what the host said, after each step a probe needs', () async {
    final host = _Host(_paths);
    final paths = HostPaths(HostControl.dialing(() async => host));
    addTearDown(host.close);
    expect(paths.real('/alias/a'), isNull);
    String? seen, root;
    await paths.settle(() {
      seen = paths.real('/alias/a');
      final resolved = seen;
      root = resolved == null ? null : paths.repoRoot(resolved);
    });
    expect(seen, '/work/a', reason: 'the host followed the link');
    expect(root, '/work', reason: 'the repository of what the link led to, asked once that was known');
    expect(host.asked.map((each) => each.$1), ['fs/realpath', 'fs/repo_root']);
    expect(paths.real('/alias/a'), '/work/a', reason: 'read from what was said, with no new question');
    expect(host.asked, hasLength(2));
  });

  test('asks again in a later round, so a link changed meanwhile is followed afresh', () async {
    final host = _Host(_paths);
    final paths = HostPaths(HostControl.dialing(() async => host));
    addTearDown(host.close);
    await paths.settle(() => paths.real('/alias/a'));
    await paths.settle(() => paths.real('/alias/a'));
    expect(host.asked.where((each) => each.$1 == 'fs/realpath'), hasLength(2));
  });

  test('keeps a path the host could not answer for as no answer, so no rule is applied on a guess', () async {
    final host = _Host((method, params) => 'error:no');
    final paths = HostPaths(HostControl.dialing(() async => host));
    addTearDown(host.close);
    await paths.settle(() => paths.real('/work/a'));
    expect(paths.real('/work/a'), isNull);
    expect(await paths.guards(), isNull);
  });

  test('asks for at most 64 paths at a time', () async {
    final host = _Host(_paths);
    final paths = HostPaths(HostControl.dialing(() async => host));
    addTearDown(host.close);
    await paths.settle(() {
      for (var i = 0; i < 150; i++) {
        paths.real('/work/f$i');
      }
    });
    final sizes = host.asked.where((each) => each.$1 == 'fs/realpath').map((each) => (each.$2['paths']! as List<Object?>).length);
    expect(sizes, [64, 64, 22]);
    expect(paths.real('/work/f149'), '/work/f149');
  });

  test('the guards of the host are kept once said, and a rule list takes them over a page that knows none', () async {
    final host = _Host(_paths);
    final paths = HostPaths(HostControl.dialing(() async => host));
    addTearDown(host.close);
    final rules = PermissionRules(MemoryLedgerBackend())..useHost(paths, up: () => true);
    expect(rules.guards.home, isNull);
    await rules.prepare(() {
      rules.pathsAllowedIn('/home/me');
      rules.pathsAllowedIn('/work/proj');
    });
    await rules.prepare(() {});
    expect(rules.guards.home, '/home/me');
    expect(host.asked.where((each) => each.$1 == 'fs/guards'), hasLength(1));
    expect(rules.pathsAllowedIn('/home/me'), isFalse, reason: 'the folder is the home folder');
    expect(rules.pathsAllowedIn('/work/proj'), isTrue);
  });

  test('a host that is not answering is not asked: the rules follow links where the app can', () async {
    final host = _Host(_paths);
    final paths = HostPaths(HostControl.dialing(() async => host));
    final rules = PermissionRules(MemoryLedgerBackend(), real: (path) => path)..useHost(paths, up: () => false);
    await rules.prepare(() => rules.pathsAllowedIn('/work/proj'));
    expect(host.asked, isEmpty);
    expect(rules.pathsAllowedIn('/work/proj'), isTrue);
  });
}
