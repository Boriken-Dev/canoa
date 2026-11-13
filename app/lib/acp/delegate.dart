import 'dart:async';
import 'dart:convert';
import 'dart:math';

/// The `_meta` key of `initialize` with which a client tells the host it can show a delegated task, answer what the task's agent
/// asks of the person and stop it. Without it the host gives its agents no delegation tool.
const delegateMeta = 'dev.boriken.canoa/delegate';

/// The host's notification of what became of a task and of what its agent said.
const delegateEventMethod = '_canoa/delegate/event';

/// The request for the updates the host kept of a task.
const delegateHistoryMethod = '_canoa/delegate/history';

/// The request to stop a session's tasks, or one of them.
const delegateStopMethod = '_canoa/delegate/stop';

/// Where a task is, in A2A's names.
enum TaskPhase {
  submitted,
  working,
  inputRequired,
  authRequired,
  completed,
  failed,
  rejected,
  canceled;

  /// The phase A2A's `TASK_STATE_*` [name] means; null for one this client does not know.
  static TaskPhase? named(Object? name) => switch (name) {
    'TASK_STATE_SUBMITTED' => submitted,
    'TASK_STATE_WORKING' => working,
    'TASK_STATE_INPUT_REQUIRED' => inputRequired,
    'TASK_STATE_AUTH_REQUIRED' => authRequired,
    'TASK_STATE_COMPLETED' => completed,
    'TASK_STATE_FAILED' => failed,
    'TASK_STATE_REJECTED' => rejected,
    'TASK_STATE_CANCELED' => canceled,
    _ => null,
  };

  /// Whether nothing changes a task after this.
  bool get ended => this == completed || this == failed || this == rejected || this == canceled;

  /// Whether the task waits for the person.
  bool get waitingOnPerson => this == inputRequired || this == authRequired;
}

/// One task a session handed to a source, as the host's events built it.
final class DelegateTask {
  DelegateTask(this.id, this.order);

  final String id;

  /// Where it came in the session's list: the order its first event arrived in.
  final int order;
  String title = '';
  String source = '';
  String? folder;
  TaskPhase phase = TaskPhase.submitted;

  /// When the host first recorded it, and when it ended; null when the host sent no time, or the task has not ended.
  DateTime? started, ended;

  /// What the host said with its last state, and the task's reply once it has one.
  String? message, result;

  /// What the task's agent sent as session updates, oldest first, as far as the host kept them.
  final List<Map<String, Object?>> updates = [];

  /// How many updates the task has had, those forgotten for the bound included, so a reader that applied some knows how many are new.
  int updatesSeen = 0;
}

/// The most updates a task keeps, which the host bounds in bytes as well.
const maxTaskUpdates = 2000;

/// The most tasks one session lists; past it the oldest that ended is forgotten.
const maxTasks = 64;

/// A session's tasks, built from the host's events in the order they came.
final class DelegateTasks {
  final Map<String, DelegateTask> _byId = {};

  /// The tasks, in the order each was first heard of.
  List<DelegateTask> get all => [for (final task in _byId.values) task]..sort((a, b) => a.order - b.order);

  /// The tasks that have not ended.
  List<DelegateTask> get live => [for (final task in _byId.values) if (!task.phase.ended) task]..sort((a, b) => a.order - b.order);

  DelegateTask? operator [](String id) => _byId[id];

  bool get isEmpty => _byId.isEmpty;

  int _next = 0;

  static String? _text(Object? value, [int limit = 200]) =>
      value is String && value.trim().isNotEmpty ? (value.length > limit ? value.substring(0, limit) : value) : null;

  static DateTime? _time(Object? value) => value is String ? DateTime.tryParse(value) : null;

  static String? _messageText(Object? message) {
    if (message is! Map || message['parts'] is! List) return null;
    final texts = [for (final part in message['parts'] as List) if (part is Map && part['text'] is String) part['text'] as String];
    return _text(texts.join('\n'), 1000);
  }

  /// The task [id], made when it is new; null when the session lists as many as it holds.
  DelegateTask? _task(String id) {
    final held = _byId[id];
    if (held != null) return held;
    if (_byId.length >= maxTasks) {
      // A task that ended is the one to forget, oldest first; a session whose every task runs lists no more.
      final gone = [for (final task in _byId.values) if (task.phase.ended) task]..sort((a, b) => a.order - b.order);
      if (gone.isEmpty) return null;
      _byId.remove(gone.first.id);
    }
    return _byId[id] = DelegateTask(id, _next++)..title = id;
  }

  void _status(DelegateTask task, Object? status) {
    if (status is! Map) return;
    final phase = TaskPhase.named(status['state']);
    // A task that ended stays as it ended, so a state that arrives late changes nothing.
    if (phase == null || task.phase.ended) return;
    task.phase = phase;
    final at = _time(status['timestamp']);
    if (phase == TaskPhase.submitted) task.started ??= at;
    if (phase == TaskPhase.working) task.started ??= at;
    if (phase.ended) task.ended = at;
    final said = _messageText(status['message']);
    if (said != null) task.message = said;
  }

  /// Takes the params of one `_canoa/delegate/event`. Whether the list changed.
  bool apply(Object? params) {
    if (params is! Map) return false;
    final id = params['taskId'];
    if (id is! String || id.isEmpty || id.length > 200) return false;
    final update = params['update'];
    if (update is Map) {
      final task = _task(id);
      if (task == null) return false;
      task.updates.add(Map<String, Object?>.from(update));
      task.updatesSeen++;
      if (task.updates.length > maxTaskUpdates) task.updates.removeAt(0);
      return true;
    }
    final event = params['event'];
    if (event is! Map) return false;
    final task = _task(id);
    if (task == null) return false;
    final whole = event['task'];
    if (whole is Map) {
      final metadata = whole['metadata'];
      if (metadata is Map) {
        task.title = _text(metadata['title'], 80) ?? task.title;
        task.source = _text(metadata['source']) ?? task.source;
        task.folder = _text(metadata['folder'], 1000) ?? task.folder;
      }
      _status(task, whole['status']);
      if (whole['artifacts'] is List) {
        for (final artifact in whole['artifacts'] as List) {
          _artifact(task, artifact);
        }
      }
    } else if (event['statusUpdate'] is Map) {
      _status(task, (event['statusUpdate'] as Map)['status']);
    } else if (event['artifactUpdate'] is Map) {
      _artifact(task, (event['artifactUpdate'] as Map)['artifact']);
    }
    return true;
  }

  void _artifact(DelegateTask task, Object? artifact) {
    if (artifact is! Map || artifact['artifactId'] != 'result' || artifact['parts'] is! List) return;
    final texts = [for (final part in artifact['parts'] as List) if (part is Map && part['text'] is String) part['text'] as String];
    task.result = _text(texts.join(), 65536);
  }

  /// Gives [id] the [updates] the host kept of it when it has heard none itself. Whether it took them.
  bool backfill(String id, List<Map<String, Object?>> updates) {
    final task = _byId[id];
    if (task == null || task.updates.isNotEmpty || updates.isEmpty) return false;
    final kept = updates.length > maxTaskUpdates ? updates.sublist(updates.length - maxTaskUpdates) : updates;
    task.updates.addAll(kept);
    task.updatesSeen += kept.length;
    return true;
  }

  /// Ends [id] as the host stopped it, when it had not ended. Whether it changed.
  bool cancelOne(String id, [DateTime? at]) {
    final task = _byId[id];
    if (task == null || task.phase.ended) return false;
    task.phase = TaskPhase.canceled;
    task.ended ??= at;
    return true;
  }

  /// Ends every task that has not, as the connection that carried them is gone and so is each task's agent. Whether any changed.
  bool cancelLive([DateTime? at]) {
    var changed = false;
    for (final task in _byId.values) {
      if (task.phase.ended) continue;
      task.phase = TaskPhase.canceled;
      task.ended ??= at;
      changed = true;
    }
    return changed;
  }
}

/// What a task gives its client at the host's word, for [DelegateWire.events].
typedef DelegateNotice = Map<String, Object?>;

/// The delegation extension on one connection, beside the shared ACP client, which never sees these messages: the opt-in added to
/// `initialize`, the host's `_canoa/delegate/event` notifications as they pass, and the app's own `history` and `stop` requests,
/// whose ids are text so they never meet the client's numbered ones.
final class DelegateWire {
  DelegateWire(this._send, {required this.optIn});

  final Future<void> Function(String) _send;

  /// Whether this client can show tasks, answer what a task's agent asks and send Stop, which is what it tells the host.
  final bool optIn;
  final _events = StreamController<DelegateNotice>.broadcast();
  final _waiting = <String, Completer<Object?>>{};
  int _next = 0;
  static final _random = Random.secure();

  /// The host's events, as their params.
  Stream<DelegateNotice> get events => _events.stream;

  /// [message] on its way to the host, with the opt-in added when it is `initialize`.
  String outgoing(String message) {
    if (!optIn || !message.contains('"initialize"')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['method'] != 'initialize' || decoded['params'] is! Map) return message;
    final params = decoded['params'] as Map;
    final own = params['_meta'] is Map ? params['_meta'] as Map : (params['_meta'] = <String, Object?>{});
    own[delegateMeta] = true;
    return jsonEncode(decoded);
  }

  /// Whether [message] is the host's, which this wire keeps so the shared client never sees it: an event, or the answer to one
  /// of its own requests.
  bool takes(String message) {
    final event = message.contains(delegateEventMethod);
    if (!event && _waiting.isEmpty) return false;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return false;
    }
    if (decoded is! Map) return false;
    if (event && decoded['method'] == delegateEventMethod) {
      final params = decoded['params'];
      if (params is Map && !_events.isClosed) _events.add(Map<String, Object?>.from(params));
      return true;
    }
    final id = decoded['id'];
    if (decoded['method'] == null && id is String) {
      final waiting = _waiting.remove(id);
      if (waiting == null) return false;
      if (!waiting.isCompleted) waiting.complete(decoded['error'] == null ? (decoded['result'] ?? const <String, Object?>{}) : null);
      return true;
    }
    return false;
  }

  Future<Object?> _ask(String method, Map<String, Object?> params, Duration timeout) async {
    // Unguessable, so no message of the agent's can pass for the host's answer.
    final nonce = [for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join();
    final id = 'canoa-delegate-${_next++}-$nonce';
    final waiting = Completer<Object?>();
    _waiting[id] = waiting;
    try {
      await _send(jsonEncode({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params}));
      return await waiting.future.timeout(timeout, onTimeout: () => null);
    } on Object {
      return null;
    } finally {
      _waiting.remove(id);
    }
  }

  /// Asks the host to stop [sessionId]'s tasks, or only [taskId]. Whether the host said it did.
  Future<bool> stop(String sessionId, {String? taskId, Duration timeout = const Duration(seconds: 10)}) async =>
      await _ask(delegateStopMethod, {'sessionId': sessionId, 'taskId': ?taskId}, timeout) != null;

  /// The updates the host kept of [taskId]; null when it has none to give or did not answer.
  Future<List<Map<String, Object?>>?> history(String sessionId, String taskId, {Duration timeout = const Duration(seconds: 10)}) async {
    final result = await _ask(delegateHistoryMethod, {'sessionId': sessionId, 'taskId': taskId}, timeout);
    if (result is! Map || result['updates'] is! List) return null;
    return [for (final update in result['updates'] as List) if (update is Map) Map<String, Object?>.from(update)];
  }

  void close() {
    for (final waiting in _waiting.values) {
      if (!waiting.isCompleted) waiting.complete(null);
    }
    _waiting.clear();
    unawaited(_events.close());
  }
}
