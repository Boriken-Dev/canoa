// Canoa's own pairing and chat connection code as a bare page, for a browser test that cannot click through Flutter's canvas.
// It runs on the web build's peer bridge (canoa_peer_bridge.mjs and its vendored methods), loaded by the page beside it:
//   dart compile js -O1 -o <dir>/page.js tool/peer_page.dart
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/acp/browser_connect.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/browser_peer.dart';

Future<String> _turn(String url, String peer, String method, String text) async {
  final client = await connectBrowserAcp(
    chatEndpoints(webSocket: url),
    pinnedPeerId: peer,
    method: PeerMethod.parse(method),
  );
  try {
    final session = await client.openSession(cwd: '/');
    final written = StringBuffer();
    final listening = client.updates.listen((update) {
      if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
    });
    await client.prompt(session.id, text);
    await Future<void>.delayed(const Duration(milliseconds: 100));
    await listening.cancel();
    return jsonEncode({'peer': client.peerId, 'echoed': written.toString()});
  } finally {
    await client.close();
  }
}

/// What the connect dialog reads from the `canoa-host` that served this page, as JSON, or null.
Future<String?> _served() async {
  final host = await loadServedHost();
  if (host == null) return null;
  return jsonEncode({
    'peer': host.peer,
    'method': host.method.id,
    'entries': [for (final entry in host.entries) {'name': entry.name, 'kind': entry.kind, 'url': entry.url}],
  });
}

void main() {
  globalContext
    ..setProperty('canoaServedHost'.toJS, (() => _served().then((json) => json?.toJS).toJS).toJS)
    ..setProperty(
      'canoaOwnId'.toJS,
      ((JSString method) => localBrowserPeerId(PeerMethod.parse(method.toDart)).then((id) => id.toJS).toJS).toJS,
    )
    ..setProperty(
      'canoaPair'.toJS,
      ((JSString url, JSString pin) => pairBrowserAcp(Uri.parse(url.toDart), pin.toDart).then((id) => id.toJS).toJS).toJS,
    )
    ..setProperty(
      'canoaTurn'.toJS,
      ((JSString url, JSString peer, JSString method, JSString text) =>
          _turn(url.toDart, peer.toDart, method.toDart, text.toDart).then((answer) => answer.toJS).toJS).toJS,
    );
}
