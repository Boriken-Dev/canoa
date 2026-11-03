import 'dart:js_interop';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'peer_proof.dart';

@JS('canoaPeer')
external JSObject? get _bridge;

@JS('canoaPeer.unreachable')
external JSPromise<JSString?> _unreachable(JSString method);
@JS('canoaPeer.localId')
external JSPromise<JSString> _localId(JSString method);
@JS('canoaPeer.begin')
external JSPromise<JSString> _begin(JSString method, JSString expectedPeerId);
@JS('canoaPeer.pair')
external JSPromise<JSString> _pair(JSString pin);
@JS('canoaPeer.step')
external JSPromise<_Step> _step(JSString token, JSUint8Array? received);
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

/// What `canoaPeer.step` answers.
extension type _Step._(JSObject _) implements JSObject {
  external JSArray<JSUint8Array> get send;
  external JSString? get peerId;
  external JSString? get session;
}

/// `web/canoa_peer_bridge.mjs`, loaded by the page.
final class _PageBridge implements PeerBridge {
  const _PageBridge();

  @override
  Future<String?> unreachable(PeerMethod method) async => (await _unreachable(method.id.toJS).toDart)?.toDart;
  @override
  Future<String> localId(PeerMethod method) async => (await _localId(method.id.toJS).toDart).toDart;
  @override
  Future<String> begin(PeerMethod method, String expectedPeerId) async =>
      (await _begin(method.id.toJS, expectedPeerId.toJS).toDart).toDart;
  @override
  Future<String> pair(String pin) async => (await _pair(pin.toJS).toDart).toDart;
  @override
  Future<PeerStep> step(String token, Uint8List? received) async {
    final step = await _step(token.toJS, received?.toJS).toDart;
    return PeerStep(
      send: [for (final message in step.send.toDart) message.toDart],
      peerId: step.peerId?.toDart,
      session: step.session?.toDart,
    );
  }

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

const _notLoaded = 'the Canoa peer bridge has not loaded';

Future<String?> browserMethodUnavailable(PeerMethod method) async =>
    _bridge == null ? _notLoaded : const _PageBridge().unreachable(method);

Future<String> localBrowserPeerId(PeerMethod method) async {
  if (_bridge == null) throw StateError(_notLoaded);
  return const _PageBridge().localId(method);
}

Future<VerifiedPeer> verifyBrowserPeer(WebChannel channel, String expectedPeerId, PeerMethod method) {
  if (_bridge == null) return Future.error(StateError(_notLoaded));
  return provePeer(const _PageBridge(), channel, expectedPeerId, method: method);
}

Future<String> pairBrowserByPin(WebChannel channel, String pin) {
  if (_bridge == null) return Future.error(StateError(_notLoaded));
  return pairByPin(const _PageBridge(), channel, pin);
}
