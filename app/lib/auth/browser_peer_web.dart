import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'peer_proof.dart';
import 'web/web_peer_bridge.dart';

/// The page's own bridge: its keys, kept in this origin's IndexedDB, and the Noise module loaded from beside the page.
final WebPeerBridge _bridge = WebPeerBridge();

Future<String?> browserMethodUnavailable(PeerMethod method) => _bridge.unreachable(method);

Future<String> localBrowserPeerId(PeerMethod method) => _bridge.localId(method);

Future<VerifiedPeer> verifyBrowserPeer(WebChannel channel, String expectedPeerId, PeerMethod method) =>
    provePeer(_bridge, channel, expectedPeerId, method: method);

Future<String> pairBrowserByPin(WebChannel channel, String pin) => pairByPin(_bridge, channel, pin);

Future<({Uint8List spki, Future<Uint8List> Function(Uint8List message) sign})?> browserSecretSigner() async {
  if (await _bridge.unreachable(PeerMethod.webcrypto) != null) return null;
  return _bridge.webcryptoSigner();
}
