import 'dart:async';
import 'dart:convert';

import 'host_tool.dart';

/// The host answered a control request with an error.
final class ControlFailed implements Exception {
  const ControlFailed(this.code, this.message);

  /// JSON-RPC's code: `-32001` is a grant the caller lacks, `-32002` a limit, `-32602` bad parameters.
  final int code;
  final String message;

  @override
  String toString() => 'the host refused: $message ($code)';
}

/// The host's control channel (`@control`) over a link: requests with the answers matched to them by number.
final class ControlClient {
  ControlClient(this._link, {this.timeout = const Duration(seconds: 30)}) {
    _listening = _link.messages.listen(_answered, onDone: _ended, onError: (Object _) => _ended());
  }

  final HostLink _link;

  /// How long an answer may take before the request is given up.
  final Duration timeout;
  late final StreamSubscription<String> _listening;
  final _waiting = <int, Completer<Object?>>{};
  int _next = 0;
  bool _over = false;

  /// The result of [method], or a [ControlFailed] with the host's error.
  Future<Object?> call(String method, [Map<String, Object?> params = const {}]) async {
    if (_over) throw StateError('the link to the host has ended');
    final id = ++_next;
    final answer = Completer<Object?>();
    _waiting[id] = answer;
    try {
      await _link.send(jsonEncode({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params}));
      return await answer.future.timeout(timeout);
    } finally {
      _waiting.remove(id);
    }
  }

  /// Whether the link has ended, so nothing more can be asked.
  bool get over => _over;

  void _answered(String line) {
    final Object? message;
    try {
      message = jsonDecode(line);
    } on FormatException {
      return;
    }
    if (message is! Map || message['id'] is! int) return;
    final waiting = _waiting[message['id']];
    if (waiting == null || waiting.isCompleted) return;
    if (message['error'] case final Map<Object?, Object?> error) {
      waiting.completeError(ControlFailed(error['code'] is int ? error['code'] as int : 0, '${error['message']}'));
    } else {
      waiting.complete(message['result']);
    }
  }

  void _ended() {
    _over = true;
    for (final waiting in _waiting.values) {
      if (!waiting.isCompleted) waiting.completeError(StateError('the link to the host has ended'));
    }
  }

  Future<void> close() async {
    _over = true;
    await _listening.cancel();
    await _link.close();
  }
}
