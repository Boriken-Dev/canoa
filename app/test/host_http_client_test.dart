// The requests the desktop app makes for itself are sent by the Canoa host it runs, which follows the system's proxy, proxy login,
// certificate verifier and client certificates (the host's side is tested in canoa-host against a proxy stand-in with a login).
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/net/host_http_client.dart';
import 'package:canoa/net/system_http.dart';
import 'package:canoa/shell/hosts/host_status.dart';
import 'package:canoa/surfaces/terminal_remote.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/workspace/remote_host.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

final class _Host implements HostLink {
  _Host(this.answer);

  /// The result for a request, or an error when it gives a string starting `error:`.
  final Object? Function(Map<String, Object?> params) answer;
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
    final result = answer(params);
    final reply = result is String && result.startsWith('error:')
        ? {'error': {'code': -32002, 'message': result.substring(6)}}
        : {'result': result};
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

HostControl _over(_Host host) => HostControl.dialing(() async => host);

Map<String, Object?> _reply(String body, {int status = 200, List<List<String>> headers = const []}) =>
    {'status': status, 'headers': headers, 'body': base64Encode(utf8.encode(body))};

final _peer = 'sha256:${'a' * 64}';

Connection _saved() => Connection(
  name: 'Build',
  kind: ConnectionKind.canoa,
  address: 'https://build.corp.example:8443/',
  peer: _peer,
  method: 'noise',
  transport: ConnectionTransport.websocket,
);

void main() {
  tearDown(resetSystemHttp);

  group('the client', () {
    test('sends the method, headers and body to the host and hands back its status, headers and bytes', () async {
      final host = _Host((_) => _reply('{"ok":true}', status: 201, headers: [['Content-Type', 'application/json'], ['Set-Thing', 'a'], ['set-thing', 'b']]));
      final client = HostHttpClient(_over(host));
      final response = await client.post(
        Uri.parse('https://build.corp.example/x?y=1'),
        headers: {'accept': 'application/json'},
        body: utf8.encode('payload'),
      );
      final (method, params) = host.asked.single;
      expect(method, 'network/fetch');
      expect(params['url'], 'https://build.corp.example/x?y=1');
      expect(params['method'], 'POST');
      expect((params['headers']! as List).map((pair) => (pair as List).cast<String>()), anyElement(equals(['accept', 'application/json'])));
      expect(utf8.decode(base64Decode(params['body']! as String)), 'payload');
      expect(response.statusCode, 201);
      expect(response.body, '{"ok":true}');
      expect(response.headers['content-type'], 'application/json');
      expect(response.headers['set-thing'], 'a, b');
    });

    test('a request with no body sends none, and asks the host for seconds, not a clock of its own', () async {
      final host = _Host((_) => _reply(''));
      await HostHttpClient(_over(host), timeout: const Duration(seconds: 12)).get(Uri.parse('http://x/'));
      expect(host.asked.single.$2.containsKey('body'), isFalse);
      expect(host.asked.single.$2['seconds'], 12);
    });

    test('the host\'s refusal is the exception\'s message, and a host that cannot be reached says so', () async {
      final refused = HostHttpClient(_over(_Host((_) => 'error:The request got no answer: proxy login refused')));
      await expectLater(
        refused.get(Uri.parse('https://x/')),
        throwsA(isA<http.ClientException>().having((e) => e.message, 'message', contains('proxy login refused'))),
      );
      final down = HostHttpClient(HostControl.dialing(() async => throw StateError('the host is not running')));
      await expectLater(
        down.get(Uri.parse('https://x/')),
        throwsA(isA<http.ClientException>().having((e) => e.message, 'message', allOf(contains('cannot be reached'), contains('not running')))),
      );
    });

    test('an answer that is not a reply is refused, not read as an empty page', () async {
      final client = HostHttpClient(_over(_Host((_) => <String, Object?>{'status': 200})));
      await expectLater(client.get(Uri.parse('https://x/')), throwsA(isA<http.ClientException>()));
    });
  });

  group('the app\'s own fetches', () {
    final listing = jsonEncode({'peer': _peer, 'method': 'noise', 'entries': <Object?>[]});

    test('with no host installed they use the platform client, which a test replaces by passing its own', () {
      final client = systemHttpClient();
      expect(client, isNot(isA<HostHttpClient>()));
      client.close();
    });

    test('a saved connection\'s list, its probe and the terminal check are each sent through the host', () async {
      final host = _Host((_) => _reply(listing));
      useHostNetwork(_over(host));
      expect(systemHttpClient(), isA<HostHttpClient>());

      final served = await fetchServedHost(_saved());
      expect(served.peer, _peer);
      final status = await probeConnection(_saved());
      expect(status.state, HostState.online, reason: '${status.detail}');
      final access = await terminalOnConnection(_saved(), control: (_) => throw StateError('no control channel is listed'));
      expect(access, isA<TerminalRefused>());

      expect(host.asked.map((a) => a.$1), ['network/fetch', 'network/fetch', 'network/fetch']);
      expect(
        host.asked.map((a) => a.$2['url']),
        everyElement('https://build.corp.example:8443/canoa.json'),
        reason: 'each of them asked the host for the list at the connection\'s address',
      );
    });

    test('the requests the app\'s source makes for itself are these, and nothing else reaches out with a client of its own', () {
      // A new request that bypasses the host is a failing test, which says where it is.
      final own = RegExp(r'\bhttp\.Client\(\)|\bHttpClient\(|\bIOClient\(|\bhttp\.(?:get|post|put|delete|head|read)\(|WebSocket\.connect\(');
      final found = <String>{};
      for (final file in Directory('lib').listSync(recursive: true).whereType<File>().where((f) => f.path.endsWith('.dart'))) {
        final path = file.path.replaceAll(r'\', '/');
        final lines = file.readAsLinesSync();
        for (var i = 0; i < lines.length; i++) {
          final line = lines[i].trimLeft();
          if (line.startsWith('//')) continue;
          if (own.hasMatch(line)) found.add('$path:${i + 1}');
        }
      }
      final files = found.map((entry) => entry.split(':').first).toSet();
      expect(
        files,
        unorderedEquals([
          // The platform's client, which `systemHttpClient` hands out until a host is installed.
          'lib/net/system_http.dart',
          // The client of a WebSocket the host carries (`network/tunnel`): its sockets are made by the host's tunnel, and a connection the
          // system reaches directly is opened here.
          'lib/net/system_dial_io.dart',
          // A served page's own requests to the host that served it: the browser's, with its proxy and certificates.
          'lib/served/local_admission.dart',
          'lib/served/portal_session.dart',
          'lib/config/canoa_config_file.dart',
        ]),
        reason: 'found: $found',
      );
    });
  });
}
