import 'dart:js_interop';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'peer_proof.dart';

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
@JS('canoaPeer.seal')
external JSPromise<JSUint8Array> _seal(JSString session, JSUint8Array message);
@JS('canoaPeer.open')
external JSPromise<JSUint8Array?> _open(JSString session, JSUint8Array record);
@JS('canoaPeer.closeRecord')
external JSPromise<JSUint8Array> _closeRecord(JSString session);
@JS('canoaPeer.closeSession')
external void _closeSession(JSString session);

/// `web/canoa_peer_bridge.mjs`, loaded by the page.
final class _PageBridge implements PeerBridge {
  const _PageBridge();

  @override
  Future<String> begin(String expectedPeerId) async => (await _begin(expectedPeerId.toJS).toDart).toDart;
  @override
  Future<String> answer(String token, String challenge) async => (await _answer(token.toJS, challenge.toJS).toDart).toDart;
  @override
  Future<String> finish(String token, String accepted) async => (await _finish(token.toJS, accepted.toJS).toDart).toDart;
  @override
  void abort(String token) => _abort(token.toJS);
  @override
  Future<Uint8List> seal(String session, Uint8List message) async => (await _seal(session.toJS, message.toJS).toDart).toDart;
  @override
  Future<Uint8List?> open(String session, Uint8List record) async => (await _open(session.toJS, record.toJS).toDart)?.toDart;
  @override
  Future<Uint8List> closeRecord(String session) async => (await _closeRecord(session.toJS).toDart).toDart;
  @override
  void closeSession(String session) => _closeSession(session.toJS);
}

Future<VerifiedPeer> verifyBrowserPeer(WebChannel channel, String expectedPeerId) {
  if (_bridge == null) {
    return Future.error(StateError('Canoa WebCrypto peer bridge has not loaded'));
  }
  return provePeer(const _PageBridge(), channel, expectedPeerId);
}
