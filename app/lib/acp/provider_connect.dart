import 'package:coqui_acp/page.dart';

import 'client.dart';

/// Connects to an agent through the browser's provider, which holds the channel and proved the peer. Canoa takes only an agent
/// the provider reports as proven, the same rule as for one it connects to itself. [initializeMeta] goes in `initialize`'s `_meta`,
/// where a host reads the loadout and profile the session starts with.
Future<VerifiedAgent> connectProviderAgent(
  AcpPage acp,
  AcpPageAgent agent, {
  AgentDecider? decider,
  AgentElicitor? elicitor,
  Map<String, Object?> initializeMeta = const {},
}) async {
  final client = await VerifiedAgent.fromProvider(
    await acp.connect(agent),
    decider: decider,
    elicitor: elicitor,
    initializeMeta: initializeMeta,
  );
  try {
    await client.initialize();
    return client;
  } catch (_) {
    await client.close();
    rethrow;
  }
}
