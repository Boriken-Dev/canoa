import 'dart:async';
import 'dart:convert';

import 'host_tool.dart';

/// The host answered a control request with an error.
final class ControlFailed implements Exception {
  const ControlFailed(this.code, this.message);

  /// JSON-RPC's code: `-32001` is a grant the caller lacks, `-32002` a limit or a refusal, `-32003` nothing by that name,
  /// `-32602` bad parameters.
  final int code;
  final String message;

  @override
  String toString() => 'the host refused: $message ($code)';
}

/// What a long request hears while it runs: the host's notifications naming it, and the questions the host puts to the caller.
final class ControlListener {
  const ControlListener({this.onNotification, this.onRequest});

  /// A notification of the host about this request (`store/progress`), with its params.
  final void Function(String method, Map<String, Object?> params)? onNotification;

  /// A request of the host to the caller (`store/ask`): the result to answer with. A throw answers with an error.
  final Future<Object?> Function(String method, Map<String, Object?> params)? onRequest;
}

/// The host's control channel (`@control`) over a link: requests with the answers matched to them by number.
final class ControlClient {
  ControlClient(this._link, {this.timeout = const Duration(seconds: 30)}) {
    _listening = _link.messages.listen(_heard, onDone: _ended, onError: (Object _) => _ended());
  }

  final HostLink _link;

  /// How long an answer may take before the request is given up.
  final Duration timeout;
  late final StreamSubscription<String> _listening;
  final _waiting = <int, Completer<Object?>>{};
  final _listeners = <int, ControlListener>{};
  int _next = 0;
  bool _over = false;

  /// The result of [method], or a [ControlFailed] with the host's error. [listener] hears what the host says meanwhile, and
  /// [wait] replaces [timeout] for a request that takes long.
  Future<Object?> call(
    String method, [
    Map<String, Object?> params = const {},
    ControlListener? listener,
    Duration? wait,
  ]) async {
    if (_over) throw StateError('the link to the host has ended');
    final id = ++_next;
    final answer = Completer<Object?>();
    _waiting[id] = answer;
    if (listener != null) _listeners[id] = listener;
    try {
      await _link.send(jsonEncode({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params}));
      return await answer.future.timeout(wait ?? timeout);
    } finally {
      _waiting.remove(id);
      _listeners.remove(id);
    }
  }

  /// Whether the link has ended, so nothing more can be asked.
  bool get over => _over;

  void _heard(String line) {
    final Object? message;
    try {
      message = jsonDecode(line);
    } on FormatException {
      return;
    }
    if (message is! Map) return;
    final method = message['method'];
    if (method is String) {
      _called(method, message);
      return;
    }
    if (message['id'] is! int) return;
    final waiting = _waiting[message['id']];
    if (waiting == null || waiting.isCompleted) return;
    if (message['error'] case final Map<Object?, Object?> error) {
      waiting.completeError(ControlFailed(error['code'] is int ? error['code'] as int : 0, '${error['message']}'));
    } else {
      waiting.complete(message['result']);
    }
  }

  /// The host speaking first: a notification names the request it belongs to in its params; a question has an id of its own
  /// and goes to the oldest request that listens for questions.
  void _called(String method, Map<Object?, Object?> message) {
    final params = <String, Object?>{
      if (message['params'] case final Map<Object?, Object?> given)
        for (final MapEntry(:key, :value) in given.entries) '$key': value,
    };
    final id = message['id'];
    if (id == null) {
      final owner = params['id'];
      if (owner is int) _listeners[owner]?.onNotification?.call(method, params);
      return;
    }
    final asked = _listeners.values.where((each) => each.onRequest != null).firstOrNull?.onRequest;
    if (asked == null) {
      _send({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32601, 'message': 'the caller answers no $method'}});
      return;
    }
    unawaited(() async {
      try {
        _send({'jsonrpc': '2.0', 'id': id, 'result': await asked(method, params)});
      } on Object catch (error) {
        _send({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32603, 'message': '$error'}});
      }
    }());
  }

  void _send(Map<String, Object?> message) {
    if (_over) return;
    unawaited(_link.send(jsonEncode(message)).catchError((Object _) {}));
  }

  void _ended() {
    _over = true;
    for (final waiting in _waiting.values) {
      if (!waiting.isCompleted) waiting.completeError(StateError('the link to the host has ended'));
    }
  }

  Future<void> close() async {
    _over = true;
    // A stream may finish its cancel only once its link has closed.
    final cancelling = _listening.cancel();
    _ended();
    await _link.close();
    await cancelling;
  }
}
