// The host's `terminal/*` through the app's control client, on a link that answers as the host does: a shell's pushed output is its
// own and not a request's, output that arrives with the answer to `terminal/open` is not lost, writes go in order and in pieces
// the host takes, and a link that ends ends the shell.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:flutter_test/flutter_test.dart';

/// A host's end of a control link: it records each request and answers by [answer].
final class _HostLink implements HostLink {
  final _in = StreamController<String>();
  final requests = <Map<String, Object?>>[];
  bool closed = false;

  /// What the host answers a request with, or its error; also may push before returning.
  Object? Function(Map<String, Object?> request)? answer;

  /// Lines sent right after the answer to a request, in the same turn.
  final after = <String>[];

  @override
  Stream<String> get messages => _in.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    requests.add(request);
    final result = answer?.call(request);
    if (request['id'] case final int id) {
      _in.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
      for (final line in after) {
        _in.add(line);
      }
      after.clear();
    }
  }

  void push(String method, Map<String, Object?> params) => _in.add(jsonEncode({'jsonrpc': '2.0', 'method': method, 'params': params}));

  void end() => unawaited(_in.close());

  @override
  Future<void> close() async => closed = true;
}

String _b64(String text) => base64Encode(utf8.encode(text));

void main() {
  late _HostLink link;
  late HostControl control;
  late ControlTerminals terminals;

  setUp(() {
    link = _HostLink();
    control = HostControl.dialing(() async => link);
    terminals = ControlTerminals(control);
    // Terminal ids are the host's, and start where request ids do: the first shell is 1, as is the first request.
    link.answer = (request) => request['method'] == 'terminal/open' ? {'id': 1, 'shell': 'pwsh', 'cwd': '/work'} : {};
  });

  tearDown(() => control.close());

  test('a shell\'s output is its own, though its id is also a request\'s number, and arrives in order', () async {
    final shell = await terminals.open(cwd: '/work', cols: 100, rows: 30);
    expect(link.requests.single['params'], {'cols': 100, 'rows': 30, 'cwd': '/work'});
    expect((shell.shell, shell.folder), ('pwsh', '/work'));
    final seen = <String>[];
    shell.output.listen((bytes) => seen.add(utf8.decode(bytes)));
    // Request number 1 is finished; a pending one with the same number as the shell must not take its output.
    final pending = control.ready().then((client) => client.call('terminal/scrollback', {'id': 1}));
    link.push('terminal/output', {'id': 1, 'data': _b64('one '), 'end': 4});
    link.push('terminal/output', {'id': 1, 'data': _b64('two'), 'end': 7});
    await pending;
    await Future<void>.delayed(Duration.zero);
    expect(seen.join(), 'one two');
  });

  test('a shell is named by the id the host listed, and only when one is named', () async {
    link.answer = (request) => switch (request['method']) {
      'terminal/open' => {'id': 1, 'shell': r'C:\Git\bin\bash.exe', 'cwd': '/work'},
      'shell/list' => {
        'shells': [
          {'id': 'git-bash', 'label': 'Git Bash', 'path': r'C:\Git\bin\bash.exe', 'family': 'bash', 'origin': 'git-for-windows'},
          {'id': 'pwsh', 'label': 'PowerShell 7', 'path': r'C:\PS\pwsh.exe', 'family': 'powershell', 'origin': 'system'},
          {'label': 'a shell with no id'},
        ],
      },
      _ => {},
    };
    final listed = await terminals.shells();
    expect([for (final shell in listed) shell.id], ['git-bash', 'pwsh'], reason: 'an entry with no id is no shell');
    expect(listed.first.family, 'bash');
    final named = await terminals.open(shell: 'git-bash', cols: 80, rows: 24);
    expect(link.requests.last['params'], {'cols': 80, 'rows': 24, 'shell': 'git-bash'});
    expect(named.shell, r'C:\Git\bin\bash.exe');
    link.answer = (request) => request['method'] == 'terminal/open' ? {'id': 2, 'shell': 'pwsh', 'cwd': '/work'} : {};
    await terminals.open(cols: 80, rows: 24);
    expect((link.requests.last['params'] as Map).containsKey('shell'), isFalse, reason: 'the host\'s own choice names none');
  });

  test('a key that may open a terminal but not list what is installed is offered no shells and still opens one', () async {
    link.answer = (request) => request['method'] == 'shell/list' ? throw const ControlFailed(-32001, 'no grant') : {'id': 1, 'shell': 'pwsh', 'cwd': '/work'};
    expect(await terminals.shells(), isEmpty);
  });

  test('output that came in the same turn as the answer to open is kept, not dropped', () async {
    link.after.add(jsonEncode({'jsonrpc': '2.0', 'method': 'terminal/output', 'params': {'id': 1, 'data': _b64('banner\r\n'), 'end': 8}}));
    final shell = await terminals.open(cols: 80, rows: 24);
    final seen = <String>[];
    shell.output.listen((bytes) => seen.add(utf8.decode(bytes)));
    await Future<void>.delayed(Duration.zero);
    expect(seen.join(), 'banner\r\n');
  });

  test('typing goes to the shell in order, in pieces the host takes, as UTF-8', () async {
    final shell = await terminals.open(cols: 80, rows: 24);
    final first = shell.write('ls\r');
    final big = shell.write('é' * 20000);
    await Future.wait([first, big]);
    final writes = [for (final request in link.requests) if (request['method'] == 'terminal/write') request['params'] as Map<String, Object?>];
    expect(writes.every((write) => write['id'] == 1), isTrue);
    final bytes = [for (final write in writes) ...base64Decode(write['data']! as String)];
    expect(utf8.decode(bytes), 'ls\r${'é' * 20000}');
    expect(writes.every((write) => base64Decode(write['data']! as String).length <= 16 * 1024), isTrue);
    expect(utf8.decode(base64Decode(writes.first['data']! as String)), 'ls\r', reason: 'the first write went first');
  });

  test('a shell that exits says its status, and one the host ended with the link says none', () async {
    final first = await terminals.open(cols: 80, rows: 24);
    link.push('terminal/exit', {'id': 1, 'code': 7});
    expect(await first.exited, 7);
    await expectLater(first.output.toList(), completion(isEmpty));

    link.answer = (request) => request['method'] == 'terminal/open' ? {'id': 2, 'shell': 'pwsh', 'cwd': '/work'} : {};
    final second = await terminals.open(cols: 80, rows: 24);
    link.end();
    expect(await second.exited, isNull, reason: 'the shell ends with its link, and the pane says so');
  });

  test('closing asks the host to, once, and an answer that names no shell is a failure', () async {
    final shell = await terminals.open(cols: 80, rows: 24);
    await shell.close();
    await shell.close();
    expect([for (final request in link.requests) request['method']], ['terminal/open', 'terminal/close']);
    expect(await shell.exited, isNull);

    final failing = _HostLink();
    final failed = ControlTerminals(HostControl.dialing(() async => failing));
    failing.answer = (_) => {'unexpected': true};
    await expectLater(failed.open(cols: 80, rows: 24), throwsA(isA<ControlFailed>()));
  });
}
