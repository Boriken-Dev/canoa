import 'package:coqui_net_transport/message_handoff.dart';

import 'agent_transport.dart';

/// Adapts Canoa's AgentConnection to a protocol-neutral transport queue.
/// The queue does not authenticate. A coqui-peer method drives setup and
/// the authenticated caller calls allowApplicationMessages().
final class StagedAgentConnection implements AgentConnection {
  StagedAgentConnection(this._source) {
    _handoff = MessageHandoff(
      incoming: _source.messages,
      send: _source.send,
      closeSource: _source.close,
    );
  }

  final AgentConnection _source;
  late final MessageHandoff _handoff;

  @override
  AgentTransportKind get kind => _source.kind;
  @override
  Stream<String> get messages => _handoff.applicationMessages;

  Future<String> nextSetupMessage() => _handoff.nextSetupMessage();

  void allowApplicationMessages() => _handoff.handoff();

  @override
  Future<void> send(String message) => _handoff.send(message);
  @override
  Future<void> close() => _handoff.close();
}
