import 'dart:async';
import 'dart:convert';

/// The `_meta` key under which the host's session hub stamps what it knows about a message.
const String hubMetaKey = 'dev.boriken.coqui.acp/hub';

/// What the hub tells every attached client when the set of attached clients changes.
const String hubPresenceMethod = '_dev.boriken.coqui.acp/hub_presence';

/// What the hub tells the other clients when one answered a permission request.
const String hubPermissionClosedMethod = '_dev.boriken.coqui.acp/hub_permission_closed';

/// What the hub tells every attached client when the agent's end of the session ended.
const String hubEndedMethod = '_dev.boriken.coqui.acp/hub_ended';

const String _requestPrefix = 'hub-perm-';

/// A client attached to a session kept by the host: a device or window of this person, or another key the entry admits.
final class HubClient {
  const HubClient({required this.id, required this.label, required this.role, this.isSelf = false});

  /// The key's id, which is the same on every connection it makes.
  final String id;

  /// What the host calls it: the entry's name.
  final String label;

  /// `operator` for a person's own app.
  final String role;

  /// Whether this is the client the message was sent to.
  final bool isSelf;

  /// The first characters of the key's id, which tell two devices of one entry apart.
  String get short {
    final bare = id.contains(':') ? id.substring(id.indexOf(':') + 1) : id;
    return bare.length > 6 ? bare.substring(0, 6) : bare;
  }

  /// How a person is told which client this is.
  String get named => '$label ($short)';

  @override
  bool operator ==(Object other) =>
      other is HubClient && other.id == id && other.label == label && other.role == role && other.isSelf == isSelf;

  @override
  int get hashCode => Object.hash(id, label, role, isSelf);
}

/// What the hub said that a client acts on.
sealed class HubEvent {
  const HubEvent(this.sessionId);
  final String sessionId;
}

/// Who is attached to [sessionId] changed.
final class HubPresence extends HubEvent {
  const HubPresence(super.sessionId, this.clients);
  final List<HubClient> clients;
}

/// Another client answered the permission request that asks about [toolCallId].
final class HubClosed extends HubEvent {
  const HubClosed(super.sessionId, {required this.requestId, required this.toolCallId, required this.by, required this.outcome});

  /// The id the request came with.
  final String requestId;

  /// The tool call the request was about, when this connection saw the request; null when it did not.
  final String? toolCallId;

  /// The client that answered, by id; null when the host or nobody did.
  final String? by;

  /// The answer, as ACP says it: `{outcome: selected, optionId}` or `{outcome: cancelled}`.
  final Map<String, Object?>? outcome;
}

/// The agent's end of [sessionId] ended, so nothing more will come of it.
final class HubEnded extends HubEvent {
  const HubEnded(super.sessionId);
}

/// What a connection to a session the host keeps takes from the hub's messages, and what it adds to the ones it sends to attach:
/// who else is attached, a request another client answered, and the cursor of the last message seen, from which a later attach
/// replays only what was missed. A connection to an agent that has no hub sees none of it and nothing changes.
final class HubWire {
  final _cursors = <String, int>{};
  final _truncated = <String, bool>{};
  final _present = <String, List<HubClient>>{};
  final _asked = <String, ({String sessionId, String? toolCallId})>{};
  final _armed = <String, int?>{};
  final _awaiting = <Object?, String>{};
  final _events = StreamController<HubEvent>.broadcast(sync: true);
  bool _seen = false;

  /// What the hub says as it says it.
  Stream<HubEvent> get events => _events.stream;

  /// Whether the host's hub has spoken on this connection: the session is kept by the host, and others may attach.
  bool get shared => _seen;

  /// The cursor of the last message of [sessionId] this connection saw, from which an attach replays what follows; null before one.
  String? cursorOf(String sessionId) => _cursors[sessionId]?.toString();

  /// Whether the replay [sessionId] was last attached with lost entries the host no longer kept.
  bool truncated(String sessionId) => _truncated[sessionId] ?? false;

  /// Everyone attached to [sessionId], this client included, as the hub last said.
  List<HubClient> attached(String sessionId) => _present[sessionId] ?? const [];

  /// The clients of [sessionId] other than this one.
  List<HubClient> others(String sessionId) => [
    for (final client in attached(sessionId))
      if (!client.isSelf) client,
  ];

  /// What to call [id] in [sessionId], or null when it is not attached.
  HubClient? clientOf(String sessionId, String id) {
    for (final client in attached(sessionId)) {
      if (client.id == id) return client;
    }
    return null;
  }

  /// Makes the next `session/resume` of [sessionId] an attach with replay: it goes out as `session/load`, after [cursor] when one
  /// is given, else from the start.
  void arm(String sessionId, {String? cursor}) {
    _armed[sessionId] = cursor == null ? null : int.tryParse(cursor);
  }

  /// Forgets what this connection knew of [sessionId].
  void forget(String sessionId) {
    _cursors.remove(sessionId);
    _truncated.remove(sessionId);
    _present.remove(sessionId);
    _armed.remove(sessionId);
  }

  /// Reads one message the host sent; the message goes on unchanged.
  String incoming(String message) {
    if (!message.contains('hub')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map) return message;
    final method = decoded['method'];
    final params = decoded['params'];
    if (method == null) {
      _answered(decoded);
      return message;
    }
    if (params is! Map) return message;
    final session = params['sessionId'];
    if (session is String) _stamped(session, params);
    switch (method) {
      case hubPresenceMethod:
        if (session is String) _presence(session, params['clients']);
      case hubPermissionClosedMethod:
        if (session is String) _closed(session, params);
      case hubEndedMethod:
        if (session is String) _emit(HubEnded(session));
      case 'session/request_permission' when decoded['id'] is String && (decoded['id'] as String).startsWith(_requestPrefix):
        if (session is String) {
          _seen = true;
          _asked[decoded['id'] as String] = (sessionId: session, toolCallId: _toolCallOf(params));
        }
    }
    return message;
  }

  /// Rewrites a request this connection sends when it is an attach, and leaves every other as it is.
  String outgoing(String message) {
    if (_armed.isEmpty || !message.contains('session/resume')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['method'] != 'session/resume' || decoded['params'] is! Map) return message;
    final params = Map<String, Object?>.from(decoded['params'] as Map);
    final session = params['sessionId'];
    if (session is! String || !_armed.containsKey(session)) return message;
    final cursor = _armed.remove(session);
    params.remove('replayFrom');
    if (cursor != null) {
      final meta = params['_meta'] is Map ? Map<String, Object?>.from(params['_meta'] as Map) : <String, Object?>{};
      meta[hubMetaKey] = {'cursor': '$cursor'};
      params['_meta'] = meta;
    }
    _awaiting[decoded['id']] = session;
    return jsonEncode({...decoded.cast<String, Object?>(), 'method': 'session/load', 'params': params});
  }

  void _answered(Map<Object?, Object?> message) {
    final session = _awaiting.remove(message['id']);
    if (session == null) return;
    final result = message['result'];
    final meta = result is Map && result['_meta'] is Map ? (result['_meta'] as Map)[hubMetaKey] : null;
    if (meta is! Map) return;
    _seen = true;
    final head = meta['cursor'] is String ? int.tryParse(meta['cursor'] as String) : null;
    if (head != null) _bump(session, head);
    _truncated[session] = meta['truncated'] == true;
  }

  void _stamped(String session, Map<Object?, Object?> params) {
    final meta = params['_meta'];
    final stamp = meta is Map ? meta[hubMetaKey] : null;
    if (stamp is! Map) return;
    _seen = true;
    final seq = stamp['seq'] is String ? int.tryParse(stamp['seq'] as String) : null;
    if (seq != null) _bump(session, seq);
  }

  void _bump(String session, int cursor) {
    final known = _cursors[session];
    if (known == null || cursor > known) _cursors[session] = cursor;
  }

  void _presence(String session, Object? clients) {
    if (clients is! List) return;
    _seen = true;
    final listed = [
      for (final client in clients)
        if (client is Map && client['id'] is String)
          HubClient(
            id: client['id'] as String,
            label: client['label'] is String ? client['label'] as String : client['id'] as String,
            role: client['role'] is String ? client['role'] as String : 'client',
            isSelf: client['self'] == true,
          ),
    ];
    _present[session] = listed;
    _emit(HubPresence(session, listed));
  }

  void _closed(String session, Map<Object?, Object?> params) {
    final request = params['requestId'];
    if (request is! String) return;
    final asked = _asked.remove(request);
    final outcome = params['outcome'];
    _emit(HubClosed(
      session,
      requestId: request,
      toolCallId: asked?.toolCallId,
      by: params['by'] is String ? params['by'] as String : null,
      outcome: outcome is Map ? outcome.cast<String, Object?>() : null,
    ));
  }

  static String? _toolCallOf(Map<Object?, Object?> params) {
    final direct = params['toolCall'];
    final subject = params['subject'];
    final call = direct is Map ? direct : (subject is Map && subject['toolCall'] is Map ? subject['toolCall'] as Map : null);
    final id = call?['toolCallId'];
    return id is String ? id : null;
  }

  void _emit(HubEvent event) {
    if (!_events.isClosed) _events.add(event);
  }

  void close() {
    unawaited(_events.close());
  }
}
