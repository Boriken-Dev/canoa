// canoa.json, which `canoa serve --web` serves beside the page: what the app and the Hosts section read of a host.
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

const _peer = 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

String _listed({String peer = _peer, String method = 'noise', String url = 'ws://127.0.0.1:7801'}) => jsonEncode({
  'peer': peer,
  'method': method,
  'entries': [
    {'name': 'claude', 'kind': 'agent', 'url': url, 'harness': 'claude', 'title': 'Claude for review'},
    {'name': 'echo', 'kind': 'provider', 'url': 'ws://127.0.0.1:7802'},
  ],
});

void main() {
  test('the list the host writes is read in its order', () {
    final host = ServedHost.parse(_listed())!;
    expect(host.peer, _peer);
    expect(host.method, PeerMethod.noise);
    expect([for (final entry in host.entries) (entry.name, entry.kind, entry.url)], [
      ('claude', 'agent', 'ws://127.0.0.1:7801'),
      ('echo', 'provider', 'ws://127.0.0.1:7802'),
    ]);
    expect([for (final entry in host.entries) (entry.harness, entry.title)], [('claude', 'Claude for review'), (null, null)]);
  });

  test('the control channel is told apart from the agents', () {
    final text = jsonEncode({
      'peer': _peer,
      'method': 'noise',
      'entries': [
        {'name': 'claude', 'kind': 'agent', 'url': 'ws://127.0.0.1:7801'},
        {'name': '@control', 'kind': 'control', 'url': 'ws://127.0.0.1:7803'},
      ],
    });
    final host = ServedHost.parse(text)!;
    expect([for (final entry in host.entries) entry.name], ['claude'], reason: 'it is no agent for the connect dialog to offer');
    expect(host.control?.url, 'ws://127.0.0.1:7803');
    expect(ServedHost.parse(_listed())!.control, isNull, reason: 'a host that listens for none lists none');
  });

  test('anything else is no list', () {
    for (final text in [
      '<!doctype html><title>Canoa</title>',
      '[]',
      '{}',
      _listed(peer: 'sha256:short'),
      _listed(peer: 'BBBB'),
      _listed(method: 'tls'),
      _listed(url: 'https://127.0.0.1:7801'),
      _listed(url: 'javascript:alert(1)'),
      _listed(url: 'ws:'),
      jsonEncode({'peer': _peer, 'method': 'noise', 'entries': [{'name': 1, 'kind': 'agent', 'url': 'ws://a:1'}]}),
    ]) {
      expect(ServedHost.parse(text), isNull, reason: text);
    }
  });

  test('it is read beside the page, and a page from anything else has none', () async {
    final asked = <Uri>[];
    final served = MockClient((request) async {
      asked.add(request.url);
      return http.Response(_listed(), 200);
    });
    final host = await loadServedHost(client: served, base: Uri.parse('http://127.0.0.1:7800/'));
    expect(asked, [Uri.parse('http://127.0.0.1:7800/canoa.json')]);
    expect(host?.entries.first.name, 'claude');

    final elsewhere = MockClient((request) async => http.Response('not found', 404));
    expect(await loadServedHost(client: elsewhere, base: Uri.parse('https://example.github.io/canoa/')), isNull);
    final failing = MockClient((request) async => throw http.ClientException('refused'));
    expect(await loadServedHost(client: failing, base: Uri.parse('http://127.0.0.1:7800/')), isNull);
    expect(await loadServedHost(client: served), isNull, reason: 'an app that is no web page reads none');
  });
}
