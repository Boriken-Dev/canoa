import 'channel.dart';

/// The built-in Dart VM has no browser WebTransport API. A future native
/// backend may supply WebTransport without changing the ACP channel contract.
Future<AcpMessageChannel> openAcpWebTransport(
  Uri endpoint,
  Duration timeout,
  List<String> certificatePins,
) => Future.error(UnsupportedError('WebTransport browser API is unavailable'));
