import 'dart:async';
import 'dart:collection';

import '../transport/agent_transport.dart';

/// Keeps one subscription to the network channel from authentication through
/// ACP startup. The authentication exchange consumes frames with nextHandshake;
/// after promote(), buffered frames and subsequent traffic go to ACP.
///
/// This prevents the stream-subscription handoff from dropping incoming data.
final class HandshakeChannel implements AgentConnection {
  HandshakeChannel(this._upstream) {
    _subscription = _upstream.messages.listen(
      _receive, onError: _fail, onDone: () =>
        _fail(StateError('Peer connection closed during authentication')),
    );
  }

  final AgentConnection _upstream;
  final Queue<String> _pending = Queue<String>();
  final StreamController<String> _messages = StreamController<String>();
  late final StreamSubscription<String> _subscription;
  Completer<String>? _waiter;
  Object? _failure;
  bool _promoted = false;
  bool _closed = false;

  @override
  AgentTransportKind get kind => _upstream.kind;
  @override
  Stream<String> get messages => _messages.stream;

  void _receive(String raw) {
    if (_closed) return;
    if (_promoted) {
      _messages.add(raw);
    } else if (_waiter case final waiter?) {
      _waiter = null;
      waiter.complete(raw);
    } else {
      if (_pending.length >= 16) {
        _fail(StateError('Too many frames before peer authentication'));
      } else {
        _pending.add(raw);
      }
    }
  }

  void _fail(Object error, [StackTrace? trace]) {
    if (_closed) return;
    _failure = error;
    final waiter = _waiter;
    _waiter = null;
    if (waiter != null && !waiter.isCompleted) waiter.completeError(error, trace);
    if (_promoted) {
      _messages.addError(error, trace);
      unawaited(_messages.close());
    }
  }

  /// Await exactly one handshake message. Never also subscribe to messages.
  Future<String> nextHandshake() {
    if (_closed || _promoted) throw StateError('Handshake phase ended');
    if (_waiter != null) throw StateError('Concurrent handshake reads are forbidden');
    if (_pending.isNotEmpty) return Future.value(_pending.removeFirst());
    if (_failure != null) return Future.error(_failure!);
    final waiter = Completer<String>();
    _waiter = waiter;
    return waiter.future;
  }

  /// Called only after the expected coqui-peer identity has been verified.
  void promote() {
    if (_closed || _promoted || _waiter != null || _failure != null) {
      throw StateError('Channel cannot be promoted to ACP');
    }
    _promoted = true;
    while (_pending.isNotEmpty) {
      _messages.add(_pending.removeFirst());
    }
  }

  @override
  Future<void> send(String message) => _upstream.send(message);

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    final waiter = _waiter;
    _waiter = null;
    if (waiter != null && !waiter.isCompleted) {
      waiter.completeError(StateError('Authentication cancelled'));
    }
    await _subscription.cancel();
    await _upstream.close();
    // A single-subscription controller never completes its close future
    // until somebody subscribes. Failed pre-auth connections have no ACP
    // listener, so close without awaiting that future.
    if (!_messages.isClosed) unawaited(_messages.close());
  }
}
