import 'dart:async';

/// One complete, reliably ordered ACP JSON-RPC message per event.
///
/// The transport does not authenticate a peer or grant agent permissions.
/// No ACP message may be sent until the peer handshake has succeeded.
abstract interface class AcpMessageChannel {
  Stream<String> get messages;

  Future<void> send(String message);

  Future<void> close();
}

/// The message transport used for the connection, not the ACP version.
enum AcpWireTransport { webTransport, webSocket }

final class AcpTransportConnection {
  const AcpTransportConnection(this.kind, this.channel);

  final AcpWireTransport kind;
  final AcpMessageChannel channel;
}
