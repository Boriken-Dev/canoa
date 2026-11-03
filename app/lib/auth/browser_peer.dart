import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'browser_peer_io.dart'
    if (dart.library.js_interop) 'browser_peer_web.dart' as platform;
import 'peer_proof.dart';

export 'peer_proof.dart' show MethodUnavailable, PeerMethod;

/// Browser-only, pinned application-layer Coqui peer proof by [method], executed BEFORE ACP, ending in the channel ACP must
/// use. Native builds require a separately implemented platform method.
Future<VerifiedPeer> verifyBrowserPeer(WebChannel channel, String expectedPeerId, PeerMethod method) =>
    platform.verifyBrowserPeer(channel, expectedPeerId, method);

/// Why this browser cannot prove by [method], or null; asked before any connection.
Future<String?> browserMethodUnavailable(PeerMethod method) => platform.browserMethodUnavailable(method);

/// Shows the local public identity under [method] for explicit pairing on the agent host.
Future<String> localBrowserPeerId(PeerMethod method) => platform.localBrowserPeerId(method);

/// Pairs this browser's Noise key by a typed PIN on [channel], and answers the id the other machine proved.
Future<String> pairBrowserByPin(WebChannel channel, String pin) => platform.pairBrowserByPin(channel, pin);
