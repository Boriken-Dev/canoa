// A page served to a client on the host's own computer (the Office add-in): after its first refused connect it asks the host that
// served it to accept its key, and connects again. Nothing is typed or paired by hand.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/served/local_admission.dart';
import 'package:canoa/served/served_app.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

final _host = 'sha256:${'b' * 64}';
final _base = Uri.parse('https://localhost:7800/office/app/');

String _index({Object? admit}) => jsonEncode({
  'peer': _host,
  'method': 'noise',
  'admit': ?admit,
  'entries': [
    {'name': '@control', 'kind': 'control', 'url': 'ws://127.0.0.1:7801'},
  ],
});

LocalAdmission _admission(MockClient client) => LocalAdmission(
  base: _base,
  path: '/office/admit',
  client: client,
  localPeer: (method) async => 'sha256:own-${method.id}',
);

void main() {
  test('the index names where to ask only when the host says so, and only as a path', () {
    expect(ServedHost.parse(_index())!.admit, isNull);
    expect(ServedHost.parse(_index(admit: '/office/admit'))!.admit, '/office/admit');
    expect(ServedHost.parse(_index(admit: 'https://elsewhere.example/admit'))!.admit, isNull, reason: 'a page asks its own host');
    expect(ServedHost.parse(_index(admit: 7))!.admit, isNull);
  });

  test('a refused connect is followed by one request for the key, then the connect again', () async {
    final asked = <Uri>[];
    final client = MockClient((request) async {
      asked.add(request.url);
      expect(request.method, 'POST');
      return http.Response('{}', 200);
    });
    var dials = 0;
    final value = await _admission(client).connect(PeerMethod.noise, () async {
      if (++dials == 1) throw StateError('turned away');
      return 'connected';
    });
    expect(value, 'connected');
    expect(dials, 2);
    expect(asked, hasLength(1));
    expect(asked.single.origin, 'https://localhost:7800', reason: 'the page\'s own origin');
    expect(asked.single.path, '/office/admit');
    expect(asked.single.queryParameters, {'peer': 'sha256:own-noise'});
  });

  test('a connect that works asks for nothing', () async {
    final client = MockClient((request) async => fail('asked ${request.url}'));
    expect(await _admission(client).connect(PeerMethod.noise, () async => 1), 1);
  });

  test('a host that never saw the key connect, or will not accept it, says so and the connect is not retried', () async {
    for (final (status, said) in [(404, 'never saw'), (403, 'would not accept')]) {
      var dials = 0;
      final client = MockClient((request) async => http.Response('{}', status));
      await expectLater(
        _admission(client).connect(PeerMethod.noise, () async {
          dials++;
          throw StateError('turned away');
        }),
        throwsA(isA<AdmissionFailed>().having((e) => e.message, 'message', contains(said))),
      );
      expect(dials, 1, reason: 'the second dial waits on an admission');
    }
  });

  group('the page of an app on this computer', () {
    test('its first probe is refused, the key is admitted, and the control channel is reached with no pairing', () async {
      final events = <String>[];
      final client = MockClient((request) async {
        events.add('${request.method} ${request.url.path}');
        return http.Response('{}', 200);
      });
      var dials = 0;
      final host = ServedHost.parse(_index(admit: '/office/admit'))!;
      final app = ServedApp.over(
        host,
        HostControl.dialing(() async {
          events.add('dial');
          if (++dials == 1) throw StateError('the host turned this key away');
          return _AnsweringLink();
        }),
        admission: _admission(client),
      );
      await app.probe();
      expect(app.reachable, isTrue);
      expect(events, ['dial', 'POST /office/admit', 'dial']);
      expect(app.portal, isNull, reason: 'no sign-in');
      await app.close();
    });

    test('a host that will not accept the key is said to, in the page\'s words', () async {
      final host = ServedHost.parse(_index(admit: '/office/admit'))!;
      final app = ServedApp.over(
        host,
        HostControl.dialing(() async => throw StateError('turned away')),
        admission: _admission(MockClient((request) async => http.Response('{}', 403))),
      );
      await app.probe();
      expect(app.reachable, isFalse);
      expect(app.detail, contains('would not accept'));
    });
  });
}

/// A control channel that answers every request with an empty result.
final class _AnsweringLink implements HostLink {
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': {'records': <Object?>[]}}));
  }

  @override
  Future<void> close() => _out.close();
}
