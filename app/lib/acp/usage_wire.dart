import 'dart:async';
import 'dart:convert';

import '../chat/agent_events.dart';

/// The `_meta` key of `initialize` with which a client says it answers the host's approval request.
const approveMeta = 'dev.boriken.canoa/approve';

/// The request with which a host asks the client that opened a session which of the profile's MCP servers, and which overlays'
/// hooks, those canoa.cfg does not allow, this session may start.
const approveMethod = '_canoa/profile/approve';

/// One server the host asks about: by name, the layer it comes from and the digest of its definition.
typedef ApprovalServer = ({String name, String layer, String digest});

/// One hook an overlay has: the [event] it is bound to, the script it runs, how long it may take, whether its failure refuses the
/// event ([onFailure] `refuse`), the reasons or tool kinds it is limited to, and the variables it may set. [fires] is false for
/// an event Canoa never sees, whose hook therefore never runs.
typedef ApprovalHookEvent = ({
  String event,
  String? run,
  int? timeout,
  String onFailure,
  List<String> match,
  List<String> sets,
  bool fires,
  String? note,
});

/// The hooks of one overlay the host asks about: the overlay, the digest of its files (what an allowance names), whether canoa.cfg
/// allowed another version of them before ([changed]), whether any hook fails closed ([failClosed]), and each hook.
typedef ApprovalHook = ({String overlay, String digest, bool changed, bool failClosed, List<ApprovalHookEvent> events});

/// What the host asks, for [profile] of [harness]'s entry.
typedef ProfileApproval = ({String profile, String harness, List<ApprovalServer> servers, List<ApprovalHook> hooks});

/// What the person allowed: the names of the servers, and the overlays whose hooks, this session may start.
typedef ApprovalAnswer = ({List<String> servers, List<String> hooks});

/// Answers an approval with what this session may start; none refuses them all.
typedef ProfileApprover = Future<ApprovalAnswer> Function(ProfileApproval ask);

/// Nothing allowed.
const noApproval = (servers: <String>[], hooks: <String>[]);

/// A report that reached the client: for [sessionId] when the agent named one, whose spend it then holds.
typedef UsageNotice = ({String? sessionId, UsageReport report});

/// The usage extension on one connection, beside the shared ACP client, which ignores what it does not know: the opt-in this
/// client adds to `initialize`, the agent's `_coqui_acp/usage_update` notifications as they pass, and the app's own
/// `_coqui_acp/usage` requests, whose ids are text so they never meet the client's numbered ones.
final class UsageWire {
  UsageWire(this._send, {this.initializeMeta = const {}, this.approver});

  final Future<void> Function(String) _send;

  /// Answers the host's approval requests; without one the host is not told this client answers them, and asks none.
  final ProfileApprover? approver;

  /// Entries added to the `_meta` of `initialize`, which a host reads before it starts anything (the cache lifetime).
  final Map<String, Object?> initializeMeta;
  final _notices = StreamController<UsageNotice>.broadcast();
  final _waiting = <String, Completer<UsageReport?>>{};
  int _next = 0;

  /// The reports the agent sends when one changes.
  Stream<UsageNotice> get notices => _notices.stream;

  /// [message] on its way to the agent, with the opt-in (and [initializeMeta]) added when it is `initialize`: this client takes notifications. The
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
    if (initializeMeta.isNotEmpty || approver != null) {
      final own = params['_meta'] is Map ? params['_meta'] as Map : (params['_meta'] = <String, Object?>{});
      own.addAll(initializeMeta);
      if (approver != null) own[approveMeta] = true;
    }
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

  /// Whether [message] is the host's approval request, which this wire answers itself so the shared client never sees it.
  bool answersApproval(String message) {
    final answer = approver;
    if (answer == null || !message.contains(approveMethod)) return false;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return false;
    }
    if (decoded is! Map || decoded['method'] != approveMethod || decoded['id'] == null) return false;
    final params = decoded['params'];
    final id = decoded['id'];
    final servers = <ApprovalServer>[];
    if (params is Map && params['servers'] is List) {
      for (final each in (params['servers'] as List).take(256)) {
        if (each is Map && each['name'] is String && each['layer'] is String && each['digest'] is String) {
          servers.add((name: each['name'] as String, layer: each['layer'] as String, digest: each['digest'] as String));
        }
      }
    }
    final hooks = <ApprovalHook>[];
    if (params is Map && params['hooks'] is List) {
      for (final each in (params['hooks'] as List).take(256)) {
        if (each is! Map || each['overlay'] is! String || each['digest'] is! String) continue;
        final events = <ApprovalHookEvent>[];
        for (final one in (each['events'] is List ? each['events'] as List : const []).take(64)) {
          if (one is! Map || one['event'] is! String) continue;
          List<String> texts(Object? value) =>
              value is List ? [for (final text in value.take(64)) if (text is String) text] : const [];
          events.add((
            event: one['event'] as String,
            run: one['run'] is String ? one['run'] as String : null,
            timeout: one['timeout'] is int ? one['timeout'] as int : null,
            onFailure: one['onFailure'] == 'refuse' ? 'refuse' : 'report',
            match: texts(one['match']),
            sets: texts(one['sets']),
            fires: one['fires'] != false,
            note: one['note'] is String ? one['note'] as String : null,
          ));
        }
        hooks.add((
          overlay: each['overlay'] as String,
          digest: each['digest'] as String,
          changed: each['changed'] == true,
          failClosed: each['failClosed'] == true,
          events: events,
        ));
      }
    }
    final ask = (
      profile: params is Map && params['profile'] is String ? params['profile'] as String : '',
      harness: params is Map && params['harness'] is String ? params['harness'] as String : '',
      servers: servers,
      hooks: hooks,
    );
    unawaited(() async {
      Map<String, Object?> body;
      try {
        final allowed = await answer(ask);
        body = {'result': {'allow': allowed.servers, 'allowHooks': allowed.hooks}};
      } on Object {
        body = {'result': {'allow': <String>[], 'allowHooks': <String>[]}};
      }
      try {
        await _send(jsonEncode({'jsonrpc': '2.0', 'id': id, ...body}));
      } on Object {
        // The link is gone; the host asks nothing more of it.
      }
    }());
    return true;
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
