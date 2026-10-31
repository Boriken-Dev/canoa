import 'dart:async';

import 'package:web_socket_channel/web_socket_channel.dart';

import 'channel.dart';

/// ACP's remote WebSocket profile: exactly one JSON-RPC message per text frame.
Future<AcpMessageChannel> openAcpWebSocket(
  Uri endpoint,
  Duration timeout,
) async {
  final socket = WebSocketChannel.connect(endpoint);
  try {
    await socket.ready.timeout(timeout);
    return _SocketMessages(socket);
  } catch (_) {
    await socket.sink.close().catchError((Object _) {});
    rethrow;
  }
}

final class _SocketMessages implements AcpMessageChannel {
  _SocketMessages(this._socket) {
    _subscription = _socket.stream.listen(
      (dynamic incoming) {
        if (_events.isClosed) return;
        if (incoming is String) {
          _events.add(incoming);
        } else {
          _events.addError(FormatException('ACP WebSocket frames must be text'));
          unawaited(close());
        }
      },
      onError: (Object error, StackTrace stack) {
        if (!_events.isClosed) _events.addError(error, stack);
      },
      onDone: () {
        if (!_events.isClosed) unawaited(_events.close());
      },
    );
  }

  final WebSocketChannel _socket;
  final StreamController<String> _events = StreamController<String>();
  late final StreamSubscription<dynamic> _subscription;
  bool _closing = false;

  @override
  Stream<String> get messages => _events.stream;

  @override
  Future<void> send(String message) async {
    if (_closing || _events.isClosed) throw StateError('ACP WebSocket is closed');
    _socket.sink.add(message);
  }

  @override
  Future<void> close() async {
    if (_closing) return;
    _closing = true;
    await _socket.sink.close();
    await _subscription.cancel();
    if (!_events.isClosed) await _events.close();
  }
}
