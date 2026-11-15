// A page `canoa portal` served: it pairs its own key after the first refused connect, goes to sign in on a 401, and signs out. The
// session is a cookie the page cannot read, so nothing here sends a token.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/served/portal_session.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

final _host = 'sha256:${'b' * 64}';
const _own = 'sha256:own-noise';
final _base = Uri.parse('http://localhost:7890/');

String _index({String? user = 'canoatesta'}) => jsonEncode({
  'peer': _host,
  'method': 'noise',
  'user': ?user,
  'entries': [
    {'name': 'scripted', 'kind': 'agent', 'url': 'ws://localhost:7890/portal/ws/entry/scripted'},
    {'name': '@control', 'kind': 'control', 'url': 'ws://localhost:7890/portal/ws/control'},
  ],
});

/// What the portal is asked, in order, and what it answers to each.
final class _Portal {
  _Portal({this.session = 200, this.pairing = 200});

  int session;
  int pairing;
  final asked = <String>[];
  final headers = <Map<String, String>>[];
  final bodies = <String>[];

  MockClient get client => MockClient((request) async {
    asked.add('${request.method} ${request.url.path}');
    headers.add(request.headers);
    bodies.add(request.body);
    return switch (request.url.path) {
      '/portal/session' => http.Response('{}', session),
      '/portal/pair' => http.Response('{}', pairing),
      '/portal/logout' => http.Response('', 204),
      '/canoa.json' => http.Response(_index(), session == 200 ? 200 : 401),
      _ => http.Response('', 404),
    };
  });
}

PortalSession _session(_Portal portal, List<String> events, {List<Uri>? navigated}) => PortalSession(
  user: 'canoatesta',
  base: _base,
  client: portal.client,
  navigate: (address) {
    events.add('navigate $address');
    navigated?.add(address);
  },
  localPeer: (method) async => 'sha256:own-${method.id}',
);

void main() {
  group('the portal\'s index', () {
    test('names the account, which marks a portal', () {
      expect(ServedHost.parse(_index())!.user, 'canoatesta');
      expect(ServedHost.parse(_index(user: null))!.user, isNull, reason: 'canoa serve --web names none');
      expect(ServedHost.parse(_index())!.control?.url, endsWith('/portal/ws/control'));
    });

    test('a 401 sends the browser to sign in and gives no host', () async {
      final portal = _Portal(session: 401);
      final logins = <Uri>[];
      final host = await loadServedHost(client: portal.client, base: _base, signedOut: logins.add);
      expect(host, isNull);
      expect(logins, [Uri.parse('http://localhost:7890/portal/login')]);
    });

    test('a signed-in browser gets its host and no sign-in', () async {
      final portal = _Portal();
      final logins = <Uri>[];
      final host = await loadServedHost(client: portal.client, base: _base, signedOut: logins.add);
      expect(host?.user, 'canoatesta');
      expect(logins, isEmpty);
    });
  });

  group('pairing', () {
    test('after the first refused connect: check the session, introduce this key, connect again', () async {
      final portal = _Portal();
      final events = <String>[];
      final session = _session(portal, events);
      var dials = 0;
      final result = await session.connect(PeerMethod.noise, () async {
        events.add('dial');
        if (++dials == 1) throw StateError('turned away');
        return 'connected';
      });
      expect(result, 'connected');
      expect(events, ['dial', 'dial']);
      expect(portal.asked, ['GET /portal/session', 'POST /portal/pair']);
      expect(jsonDecode(portal.bodies.last), {'peer': _own});
      expect(portal.headers.last['content-type'], startsWith('application/json'));
    });

    test('a key already paired connects once and asks the portal nothing', () async {
      final portal = _Portal();
      final session = _session(portal, []);
      expect(await session.connect(PeerMethod.noise, () async => 'connected'), 'connected');
      expect(portal.asked, isEmpty);
    });

    test('it holds and sends no token: the cookie is the browser\'s', () async {
      final portal = _Portal();
      final session = _session(portal, []);
      var dials = 0;
      await session.connect(PeerMethod.noise, () async => ++dials == 1 ? throw StateError('no') : 0);
      await session.signOut();
      for (final sent in portal.headers) {
        expect(sent.keys.map((name) => name.toLowerCase()), isNot(anyOf(contains('cookie'), contains('authorization'))));
      }
    });

    test('a connect that fails after pairing fails: it is not tried a third time', () async {
      final portal = _Portal();
      final session = _session(portal, []);
      var dials = 0;
      await expectLater(session.connect(PeerMethod.noise, () async => throw StateError('dial ${++dials}')), throwsStateError);
      expect(dials, 2);
      expect(portal.asked.where((each) => each == 'POST /portal/pair'), hasLength(1));
    });

    test('a refused connect with the session over goes to sign in and pairs nothing', () async {
      final portal = _Portal(session: 401);
      final navigated = <Uri>[];
      final session = _session(portal, [], navigated: navigated);
      var dials = 0;
      await expectLater(
        session.connect(PeerMethod.noise, () async => throw StateError('dial ${++dials}')),
        throwsA(isA<PortalSignedOut>()),
      );
      expect(dials, 1);
      expect(portal.asked, ['GET /portal/session']);
      expect(navigated, [Uri.parse('http://localhost:7890/portal/login')]);
    });

    test('a 401 on the pairing itself goes to sign in', () async {
      final portal = _Portal(pairing: 401);
      final navigated = <Uri>[];
      final session = _session(portal, [], navigated: navigated);
      await expectLater(session.connect(PeerMethod.noise, () async => throw StateError('no')), throwsA(isA<PortalSignedOut>()));
      expect(navigated, [Uri.parse('http://localhost:7890/portal/login')]);
    });

    test('a key the host never saw is said so, not retried', () async {
      final portal = _Portal(pairing: 404);
      final session = _session(portal, []);
      var dials = 0;
      await expectLater(
        session.connect(PeerMethod.noise, () async => throw StateError('dial ${++dials}')),
        throwsA(isA<PortalPairingFailed>().having((error) => error.message, 'message', contains('never saw'))),
      );
      expect(dials, 1);
    });

    test('a portal that does not answer is not taken for a sign-out', () async {
      final navigated = <Uri>[];
      final session = PortalSession(
        user: 'canoatesta',
        base: _base,
        client: MockClient((request) async => throw http.ClientException('down')),
        navigate: navigated.add,
        localPeer: (method) async => _own,
      );
      expect(await session.active(), isTrue);
      expect(navigated, isEmpty);
    });
  });

  group('signing out', () {
    test('posts the sign-out, then goes to the sign-in page', () async {
      final portal = _Portal();
      final events = <String>[];
      await _session(portal, events).signOut();
      expect(portal.asked, ['POST /portal/logout']);
      expect(events, ['navigate http://localhost:7890/portal/login']);
    });
  });

  group('the app behind a portal', () {
    ServedApp app(_Portal portal, Future<HostLink> Function() dial, List<String> events) {
      final host = ServedHost.parse(_index())!;
      return ServedApp.over(host, HostControl.dialing(dial), portal: _session(portal, events));
    }

    test('its first probe is refused, pairs, and then reaches the control channel', () async {
      final portal = _Portal();
      final events = <String>[];
      var dials = 0;
      final link = _AnsweringLink();
      final served = app(portal, () async {
        events.add('dial');
        if (++dials == 1) throw StateError('the host turned this key away');
        return link;
      }, events);
      await served.probe();
      expect(served.reachable, isTrue);
      expect(events, ['dial', 'dial']);
      expect(portal.asked, ['GET /portal/session', 'POST /portal/pair']);
      await served.close();
    });

    test('a key already paired is reached with no pairing, as after a reload', () async {
      final portal = _Portal();
      final served = app(portal, () async => _AnsweringLink(), []);
      await served.probe();
      expect(served.reachable, isTrue);
      expect(portal.asked, isEmpty);
      await served.close();
    });

    test('a probe in a signed-out session says so and goes to sign in', () async {
      final portal = _Portal(session: 401);
      final events = <String>[];
      final served = app(portal, () async => throw StateError('closed'), events);
      await served.probe();
      expect(served.reachable, isFalse);
      expect(served.problem, contains('signed out'));
      expect(events, ['navigate http://localhost:7890/portal/login']);
    });

    test('a host that is not a portal\'s has no sign-in', () {
      final plain = ServedApp.over(ServedHost.parse(_index(user: null))!, HostControl.dialing(() async => throw StateError('no')));
      expect(plain.portal, isNull);
    });
  });

  group('the section selector', () {
    Future<void> show(WidgetTester tester, {String? account, Future<void> Function()? onSignOut}) async {
      await tester.binding.setSurfaceSize(const Size(260, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: SectionRail(
            sections: const [AppSection(id: 'conversations', label: 'Conversations', icon: Icons.chat_bubble_outline, main: SizedBox())],
            selected: 'conversations',
            onSelect: (_) {},
            account: account,
            onSignOut: onSignOut,
          ),
        ),
      ));
      await tester.pump();
    }

    testWidgets('behind a portal it names the account and has a Sign out that signs out', (tester) async {
      var signedOut = 0;
      await show(tester, account: 'canoatesta', onSignOut: () async => signedOut++);
      expect(find.byTooltip('Signed in as canoatesta. Sign out'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('sign-out')));
      await tester.pump();
      expect(signedOut, 1);
    });

    testWidgets('elsewhere it shows neither', (tester) async {
      await show(tester);
      expect(find.byKey(const ValueKey('sign-out')), findsNothing);
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
