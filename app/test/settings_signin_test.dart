// Settings' Sign-in page, for a page `canoa portal` served: it lists the one-time code, the passkeys, the certificate and Sign out
// from what `GET /portal/account` says, opens each of the portal's pages in the same tab, shows a page the portal will not let this
// session use with the portal's reason and no button, and is not there at all for a page nothing portal-shaped served.
@TestOn('vm')
library;

import 'dart:convert';

import 'package:canoa/served/portal_session.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/settings_fixture.dart';

final _base = Uri.parse('http://localhost:7890/');

Map<String, Object?> _page(String path, {bool allowed = true, String? reason, Map<String, Object?> state = const {}}) => {
  'path': path,
  'allowed': allowed,
  'reason': ?reason,
  ...state,
};

/// What the portal answers `GET /portal/account`, and what it was asked.
final class _Portal {
  _Portal(this.account, {this.status = 200});

  Map<String, Object?> account;
  int status;
  final asked = <String>[];
  final opened = <Uri>[];

  PortalSession get session => PortalSession(
    user: 'ana',
    base: _base,
    navigate: opened.add,
    client: MockClient((request) async {
      asked.add('${request.method} ${request.url.path}');
      return switch (request.url.path) {
        '/portal/account' => http.Response(jsonEncode(account), status),
        '/portal/logout' => http.Response('', 204),
        _ => http.Response('', 404),
      };
    }),
  );
}

Map<String, Object?> _signedIn() => {
  'user': 'ana',
  'code': _page('/portal/factor', state: {'set_up': false}),
  'passkeys': _page('/portal/passkeys', state: {'count': 2}),
  'certificate': _page('/portal/certificate', state: {'pinned': false}),
  'sign_out': {'path': '/portal/logout', 'allowed': true},
};

void main() {
  testWidgets('a page a portal served has a Sign-in section, and one served by anything else does not', (tester) async {
    await openSettings(tester);
    expect(find.byKey(const Key('settings-nav-signin')), findsNothing);
    final portal = _Portal(_signedIn());
    await openSettings(tester, portal: portal.session);
    expect(find.byKey(const Key('settings-nav-signin')), findsOneWidget);
    expect(find.text('Sign-in'), findsWidgets);
  });

  testWidgets('it says what stands in each page and opens the portal\'s page in this tab', (tester) async {
    final portal = _Portal(_signedIn());
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(portal.asked, ['GET /portal/account']);
    expect(find.text('Signed in as ana.'), findsOneWidget);
    expect(find.text('No code is set up'), findsOneWidget);
    expect(find.text('2 passkeys'), findsOneWidget);
    expect(find.text('No certificate is pinned'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('signin-code-open')), matching: find.text('Set up')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('signin-passkeys-open')), matching: find.text('Manage')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('signin-certificate-open')), matching: find.text('Enrol')), findsOneWidget);

    await tester.tap(find.byKey(const Key('signin-code-open')));
    await tester.tap(find.byKey(const Key('signin-passkeys-open')));
    await tester.tap(find.byKey(const Key('signin-certificate-open')));
    expect(portal.opened, [
      Uri.parse('http://localhost:7890/portal/factor'),
      Uri.parse('http://localhost:7890/portal/passkeys'),
      Uri.parse('http://localhost:7890/portal/certificate'),
    ]);
  });

  testWidgets('a code that is set up is set up again, and a pinned certificate is managed', (tester) async {
    final portal = _Portal({
      ..._signedIn(),
      'code': _page('/portal/factor', state: {'set_up': true}),
      'passkeys': _page('/portal/passkeys', state: {'count': 1}),
      'certificate': _page('/portal/certificate', state: {'pinned': true}),
    });
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(find.text('A code is set up'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('signin-code-open')), matching: find.text('Set up again')), findsOneWidget);
    expect(find.text('1 passkey'), findsOneWidget);
    expect(find.text('A certificate is pinned to your account'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('signin-certificate-open')), matching: find.text('Manage')), findsOneWidget);
  });

  testWidgets('a page the portal will not let this session use shows its reason and no button', (tester) async {
    final portal = _Portal({
      ..._signedIn(),
      'code': _page('/portal/factor', allowed: false, reason: 'Setting up a code is not allowed from here: ask an administrator.', state: {'set_up': false}),
    });
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(find.textContaining('not allowed from here'), findsOneWidget);
    expect(find.byKey(const Key('signin-code-open')), findsNothing);
    // The other pages are not held back by it.
    expect(find.byKey(const Key('signin-passkeys-open')), findsOneWidget);
  });

  testWidgets('scene: every page open, at desktop width', (tester) async {
    await openSettings(tester, portal: _Portal(_signedIn()).session, size: const Size(1100, 700));
    await goTo(tester, 'signin');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/signin_desktop.png'));
  });

  testWidgets('scene: the code and certificate with the portal reasons, at phone width', (tester) async {
    final shut = _Portal({
      ..._signedIn(),
      'code': _page('/portal/factor', allowed: false, reason: 'Setting up a code is not allowed from here: ask an administrator.', state: {'set_up': false}),
      'certificate': _page('/portal/certificate', allowed: false, reason: 'This connection carries no certificate: open the portal where the browser offers one.', state: {'pinned': false}),
    });
    await openSettings(tester, portal: shut.session, size: const Size(390, 1400));
    await goTo(tester, 'signin');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/signin_blocked_phone.png'));
  });

  testWidgets('a page the portal does not have is left out', (tester) async {
    final portal = _Portal({'user': 'ana', 'code': _page('/portal/factor', state: {'set_up': false})});
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(find.byKey(const Key('signin-code')), findsOneWidget);
    expect(find.byKey(const Key('signin-passkeys')), findsNothing);
    expect(find.byKey(const Key('signin-certificate')), findsNothing);
  });

  testWidgets('Sign out ends the session on the portal and returns to the sign-in page', (tester) async {
    final portal = _Portal(_signedIn());
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    await tester.tap(find.byKey(const Key('signin-signout-button')));
    await tester.pumpAndSettle();
    expect(portal.asked, ['GET /portal/account', 'POST /portal/logout']);
    expect(portal.opened, [Uri.parse('http://localhost:7890/portal/login')]);
  });

  testWidgets('an answer the page cannot read says so and can be asked again', (tester) async {
    final portal = _Portal(_signedIn(), status: 503);
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(find.byKey(const Key('signin-unreadable')), findsOneWidget);
    portal.status = 200;
    await tester.tap(find.byKey(const Key('signin-retry')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('signin-unreadable')), findsNothing);
    expect(find.byKey(const Key('signin-code')), findsOneWidget);
  });

  testWidgets('a session that ended sends the browser to sign in again', (tester) async {
    final portal = _Portal(_signedIn(), status: 401);
    await openSettings(tester, portal: portal.session);
    await goTo(tester, 'signin');
    expect(portal.opened, [Uri.parse('http://localhost:7890/portal/login')]);
  });
}
