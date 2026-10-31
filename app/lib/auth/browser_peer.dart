import '../transport/agent_transport.dart';
import 'browser_peer_io.dart'
    if (dart.library.js_interop) 'browser_peer_web.dart' as platform;

/// Browser-only, pinned application-layer Coqui peer proof, executed BEFORE
/// ACP. Native builds require a separately implemented platform method.
Future<String> verifyBrowserPeer(AgentConnection channel, String expectedPeerId) =>
    platform.verifyBrowserPeer(channel, expectedPeerId);

/// Shows the local public identity for explicit pairing on the agent host.
Future<String> localBrowserPeerId() => platform.localBrowserPeerId();
