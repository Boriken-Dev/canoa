// What the web app asks of the portal that served it for its Sign-in page, as a bare page for a browser test that cannot click through
// Flutter's canvas: the app's own code reads `GET /portal/account` and opens one of the portal's pages in this tab. Built as a web
// target beside the app:
//   flutter build web --csp --no-web-resources-cdn -t tool/signin_page.dart --output <dir>
// and served as the portal's site, where <dir>/index.html runs this file's main.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/served/portal_account.dart';
import 'package:canoa/served/served_app.dart';

Map<String, Object?> _page(PortalPage? page) => page == null
    ? {}
    : {'path': page.path, 'allowed': page.allowed, 'reason': page.reason, 'state': page.state};

/// What the app's Sign-in page lists, as the app reads it from the portal.
Future<String> _account() async {
  final app = await ServedApp.discover(base: Uri.base.replace(path: '/', query: '', fragment: ''));
  final portal = app?.portal;
  if (portal == null) return jsonEncode({'error': 'this page was not served by a portal'});
  final account = await portal.account();
  if (account == null) return jsonEncode({'error': 'the portal did not say'});
  return jsonEncode({
    'user': account.user,
    'code': _page(account.code),
    'codeSetUp': account.codeSetUp,
    'passkeys': _page(account.passkeys),
    'passkeyCount': account.passkeyCount,
    'certificate': _page(account.certificate),
    'certificatePinned': account.certificatePinned,
  });
}

/// Opens the portal's page called [which] (`code`, `passkeys` or `certificate`) in this tab, as the section's button does.
Future<String> _open(String which) async {
  final app = await ServedApp.discover(base: Uri.base.replace(path: '/', query: '', fragment: ''));
  final portal = app?.portal;
  final account = await portal?.account();
  final page = switch (which) {
    'code' => account?.code,
    'passkeys' => account?.passkeys,
    'certificate' => account?.certificate,
    _ => null,
  };
  if (portal == null || page == null) return jsonEncode({'error': 'no such page'});
  portal.open(page.path);
  return jsonEncode({'opened': page.path});
}

Future<String> _guard(Future<String> step) => step.catchError((Object error) => jsonEncode({'error': '$error'}));

void main() {
  globalContext
    ..setProperty('canoaSigninAccount'.toJS, (() => _guard(_account()).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaSigninOpen'.toJS, ((JSString which) => _guard(_open(which.toDart)).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaReady'.toJS, true.toJS);
}
