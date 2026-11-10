import 'dart:async';
import 'dart:convert';

import '../chat/usage.dart';

/// A report that reached the client: for [sessionId] when the agent named one, whose spend it then holds.
typedef UsageNotice = ({String? sessionId, UsageReport report});

/// The usage extension on one connection, beside the shared ACP client, which ignores what it does not know: the opt-in this
/// client adds to `initialize`, the agent's `_coqui_acp/usage_update` notifications as they pass, and the app's own
/// `_coqui_acp/usage` requests, whose ids are text so they never meet the client's numbered ones.
final class UsageWire {
  UsageWire(this._send);

  final Future<void> Function(String) _send;
  final _notices = StreamController<UsageNotice>.broadcast();
  final _waiting = <String, Completer<UsageReport?>>{};
  int _next = 0;

  /// The reports the agent sends when one changes.
  Stream<UsageNotice> get notices => _notices.stream;

  /// [message] on its way to the agent, with the opt-in added when it is `initialize`: this client takes notifications. The
  /// host adds the account itself.
  String outgoing(String message) {
    if (!message.contains('"initialize"')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['method'] != 'initialize' || decoded['params'] is! Map) return message;
    final params = decoded['params'] as Map;
    final key = params['capabilities'] is Map ? 'capabilities' : 'clientCapabilities';
    final capabilities = params[key] is Map ? params[key] as Map : (params[key] = <String, Object?>{});
    final meta = capabilities['_meta'] is Map ? capabilities['_meta'] as Map : (capabilities['_meta'] = <String, Object?>{});
    meta[usageCapability] = {'notify': true};
    return jsonEncode(decoded);
  }

  /// [message] from the agent, passed on unchanged: a usage notification is kept, and the answer to one of this wire's requests.
  String incoming(String message) {
    final notification = message.contains(usageNotification);
    if (!notification && _waiting.isEmpty) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map) return message;
    if (notification && decoded['method'] == usageNotification) {
      final params = decoded['params'];
      final report = params is Map ? UsageReport.parse(params['report']) : null;
      if (report != null && !_notices.isClosed) {
        final session = params is Map ? params['sessionId'] : null;
        _notices.add((sessionId: session is String ? session : null, report: report));
      }
    } else if (decoded['method'] == null && decoded['id'] is String) {
      final waiting = _waiting.remove(decoded['id']);
      if (waiting != null && !waiting.isCompleted) waiting.complete(UsageReport.parse(decoded['result']));
    }
    return message;
  }

  /// Asks the agent for its report, with [sessionId]'s spend when one is named. Null when it did not answer in [timeout] or
  /// answered with an error: the report is a convenience, never a reason to fail.
  Future<UsageReport?> fetch({String? sessionId, Duration timeout = const Duration(seconds: 10)}) async {
    final id = 'canoa-usage-${_next++}';
    final waiting = Completer<UsageReport?>();
    _waiting[id] = waiting;
    try {
      await _send(jsonEncode({
        'jsonrpc': '2.0',
        'id': id,
        'method': usageMethod,
        'params': {'sessionId': ?sessionId},
      }));
      return await waiting.future.timeout(timeout, onTimeout: () => null);
    } on Object {
      return null;
    } finally {
      _waiting.remove(id);
    }
  }

  void close() {
    for (final waiting in _waiting.values) {
      if (!waiting.isCompleted) waiting.complete(null);
    }
    _waiting.clear();
    unawaited(_notices.close());
  }
}
