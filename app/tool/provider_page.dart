// Canoa's own connection code as a bare page, for the extension's browser test, which cannot click through Flutter's canvas:
//   dart compile js -o <dir>/dart/page.js tool/provider_page.dart
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/provider_connect.dart';
import 'package:coqui_acp/page.dart';

Future<AcpPage> _provider() async {
  final acp = await findAcp(timeout: const Duration(seconds: 1));
  if (acp == null) throw StateError('no provider');
  return acp;
}

Future<int> _ask() async {
  final acp = await _provider();
  return (await acp.request(purpose: 'Chat with it in Canoa')).length;
}

Future<String> _turn(String label, String text) async {
  final acp = await _provider();
  final agents = await acp.agents();
  final client = await connectProviderAgent(
    acp,
    agents.firstWhere((agent) => agent.label == label),
  );
  try {
    final session = await client.createSession(cwd: '/');
    final completed = client.events.firstWhere(
      (event) => event is AcpCompleted,
    );
    await client.prompt(session, text);
    final stop =
        (await completed.timeout(const Duration(seconds: 20)) as AcpCompleted)
            .stop;
    return jsonEncode({
      'peer': client.peerId,
      'session': session,
      'stop': stop,
    });
  } finally {
    await client.close();
  }
}

void main() {
  globalContext
    ..setProperty(
      'canoaAsk'.toJS,
      (() => _ask().then((count) => count.toJS).toJS).toJS,
    )
    ..setProperty(
      'canoaTurn'.toJS,
      ((JSString label, JSString text) => _turn(
        label.toDart,
        text.toDart,
      ).then((answer) => answer.toJS).toJS).toJS,
    );
}
