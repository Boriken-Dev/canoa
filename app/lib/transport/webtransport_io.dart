import 'contracts.dart';

/// Dart VM has no browser WebTransport constructor. Prefer a WSS fallback.
Future<AgentConnection> openAgentWebTransport(
  Uri uri,
  Duration timeout,
  List<String> certificatePins,
) async => throw const TransportUnavailable('WebTransport requires a compatible browser');
