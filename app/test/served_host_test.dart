// canoa.json, which `canoa serve --web` serves beside the page: read only as a filling for the connect dialog.
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

const _peer = 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

String _listed({String peer = _peer, String method = 'noise', String url = 'ws://127.0.0.1:7801'}) => jsonEncode({
  'peer': peer,
  'method': method,
  'entries': [
    {'name': 'claude', 'kind': 'agent', 'url': url},
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

  Future<void> open(WidgetTester tester, Future<void> Function(ConnectTarget) onConnect, {bool inBrowser = true}) async {
    await tester.binding.setSurfaceSize(const Size(900, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () => showCanoaConnectDialog(
        context,
        onConnect,
        methodUnavailable: (method) async => null,
        localPeerId: (method) async => 'sha256:own-${method.id}',
        servedHost: () async => ServedHost.parse(_listed()),
        inBrowser: inBrowser,
      ),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
  }

  String field(WidgetTester tester, String label) =>
      tester.widget<TextField>(find.widgetWithText(TextField, label)).controller!.text;

  testWidgets('a page canoa served starts filled with its first entry, its id and its method', (tester) async {
    final targets = <ConnectTarget>[];
    await open(tester, (target) async => targets.add(target));
    expect(field(tester, 'WebSocket WSS endpoint'), 'ws://127.0.0.1:7801');
    expect(field(tester, 'Pinned server peer ID'), _peer);
    expect(find.textContaining('sha256:own-noise'), findsOneWidget, reason: 'its method is chosen, so this browser shows its id');
    expect(find.textContaining('canoa accept'), findsOneWidget, reason: 'the key still has to be accepted on the host');
    expect(find.textContaining('canoa id'), findsOneWidget, reason: 'the id filled in is the page\'s word: compare it');

    await tester.tap(find.widgetWithText(ChoiceChip, 'echo'));
    await tester.pumpAndSettle();
    expect(field(tester, 'WebSocket WSS endpoint'), 'ws://127.0.0.1:7802');

    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect([for (final target in targets) (target.webSocket, target.peerId, target.method)], [
      ('ws://127.0.0.1:7802', _peer, PeerMethod.noise),
    ]);
  });

  testWidgets('an app that is no web page asks for no list', (tester) async {
    await open(tester, (_) async {}, inBrowser: false);
    expect(field(tester, 'WebSocket WSS endpoint'), isEmpty);
    expect(find.byType(ChoiceChip), findsNothing);
  });
}
