import 'dart:convert';
import 'dart:js_interop';

import '../transport/agent_transport.dart';
import '../transport/staged_connection.dart';

@JS('canoaPeer')
external JSObject? get _bridge;

@JS('canoaPeer.localId')
external JSPromise<JSString> _localId();

Future<String> localBrowserPeerId() async {
  if (_bridge == null) throw StateError('Canoa peer bridge not loaded');
  return (await _localId().toDart).toDart;
}

@JS('canoaPeer.begin')
external JSPromise<JSString> _begin(JSString expectedPeerId);
@JS('canoaPeer.answer')
external JSPromise<JSString> _answer(JSString token, JSString challenge);
@JS('canoaPeer.finish')
external JSPromise<JSString> _finish(JSString token, JSString accepted);
@JS('canoaPeer.abort')
external void _abort(JSString token);

Future<String> verifyBrowserPeer(AgentConnection channel, String expectedPeerId) async {
  if (channel is! StagedAgentConnection) {
    throw StateError('A staged transport connection is required');
  }
  if (_bridge == null) {
    throw StateError('Canoa WebCrypto peer bridge has not loaded');
  }
  String? token;
  try {
    final raw = (await _begin(expectedPeerId.toJS).toDart).toDart;
    final start = jsonDecode(raw);
    if (start is! Map || start['token'] is! String ||
        start['hello'] is! String) {
      throw const FormatException('Invalid browser identity handshake');
    }
    token = start['token'] as String;
    final first = channel.nextSetupMessage();
    await channel.send(start['hello'] as String);
    final challenge = await first.timeout(const Duration(seconds: 15));
    final proof = (await _answer(token.toJS, challenge.toJS).toDart).toDart;

    final second = channel.nextSetupMessage();
    await channel.send(proof);
    final accepted = await second.timeout(const Duration(seconds: 15));
    return (await _finish(token.toJS, accepted.toJS).toDart).toDart;
  } finally {
    if (token != null) _abort(token.toJS);
  }
}
