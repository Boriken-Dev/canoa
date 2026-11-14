import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatConnection, UsageNotice, UsageReport;

import 'client.dart';

/// A [VerifiedAgent] as the chat model's [ChatConnection]: what the agent sends comes in, and what the conversation asks of it goes
/// out. The proof of the peer, the transport and the app's own extensions stay with the [VerifiedAgent].
final class VerifiedChatConnection extends ChatConnection {
  VerifiedChatConnection(this.agent);

  final VerifiedAgent agent;

  @override
  Stream<AcpSessionUpdate> get updates => agent.updates;
  @override
  Stream<AcpEvent> get events => agent.events;
  @override
  bool get isClosed => agent.isClosed;
  @override
  int? get protocolVersion => agent.protocolVersion;

  @override
  Future<String?> prompt(String sessionId, String text, {List<AcpAttachment> attachments = const []}) =>
      agent.prompt(sessionId, text, attachments: attachments);

  @override
  Future<void> cancel(String sessionId) => agent.cancel(sessionId);

  @override
  Future<void> setMode(String sessionId, String modeId) => agent.setMode(sessionId, modeId);

  @override
  Future<Map<String, dynamic>> setConfigOption(String sessionId, String configId, String value) =>
      agent.setConfigOption(sessionId, configId, value);

  @override
  bool get reportsUsage => agent.reportsUsage;
  @override
  Stream<UsageNotice> get usageNotices => agent.usageNotices;
  @override
  Future<UsageReport?> fetchUsage({String? sessionId}) => agent.fetchUsage(sessionId: sessionId);

  @override
  Future<void> close() => agent.close();
}
