import '../transport/agent_transport.dart';

Future<String> verifyBrowserPeer(AgentConnection channel, String expectedPeerId) =>
    Future.error(UnsupportedError(
      'Browser WebCrypto is unavailable; use a native coqui-peer method',
    ));
