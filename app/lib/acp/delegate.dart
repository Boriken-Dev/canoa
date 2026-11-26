import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' as pkg;

import '../brand.dart';
import 'delegate_targets.dart';

/// The `_meta` key of `initialize` with which a client tells the host it can show a delegated task, answer what the task's agent
/// asks of the person and stop it. Without it the host gives its agents no delegation tool.
const delegateMeta = 'dev.boriken.canoa/delegate';

/// The host's notification of what became of a task and of what its agent said.
const delegateEventMethod = '_canoa/delegate/event';

/// The request for the updates the host kept of a task.
const delegateHistoryMethod = '_canoa/delegate/history';

/// The request to stop a session's tasks, or one of them.
const delegateStopMethod = '_canoa/delegate/stop';

/// The request for what a session may hand a task to.
const delegateTargetsMethod = '_canoa/delegate/targets';

/// The request that starts a task of the person's own, as a task of the session.
const delegateStartMethod = '_canoa/delegate/start';

/// The request that narrows what a session may delegate to, and sets its default and its limit.
const delegateSetupMethod = '_canoa/delegate/setup';

/// The request for what handing the conversation's turns or files on would give a task.
const delegateContextMethod = '_canoa/delegate/context';

/// A request of the app's that the host refused, with its words.
final class DelegateRefusal {
  const DelegateRefusal(this.message);
  final String message;
}

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

/// What a task's own session spent, as the host's usage log has it; a counter the agent never reported is null. The sessions the
/// task started are not in it: they have figures of their own, so a sum over the tasks counts each once.
final class TaskUsage {
  const TaskUsage({this.input, this.output, this.cacheRead, this.cacheWrite, this.costMicroUsd});

  /// Reads the host's `usage` metadata; null when it holds no counter.
  static TaskUsage? parse(Object? raw) {
    if (raw is! Map) return null;
    int? n(String key) {
      final value = raw[key];
      return value is int && value >= 0 ? value : null;
    }

    final usage = TaskUsage(input: n('input'), output: n('output'), cacheRead: n('cacheRead'), cacheWrite: n('cacheWrite'), costMicroUsd: n('costMicroUsd'));
    return usage.input == null && usage.output == null && usage.cacheRead == null && usage.cacheWrite == null ? null : usage;
  }

  final int? input, output, cacheRead, cacheWrite, costMicroUsd;
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

  /// What the task was handed besides its text, as the host recorded it ("2 turns, 1,204 bytes"); null when only the task.
  String? handed;

  /// What the task's own session spent, once the host has said; null while it has not, or the agent reported none.
  TaskUsage? usage;

  /// Whether the task runs on another machine, whose agent's questions are the person's alone: no remembered rule, allowance or
  /// Bypass answers for it, since none of them was written with that agent in mind.
  bool remote = false;
  TaskPhase phase = TaskPhase.submitted;

  /// Whether the agent of the conversation started it itself (a provider-native subagent) rather than through the host's delegation
  /// tools. The host knows nothing of it: it cannot be stopped from here, and it has no time of its own.
  bool native = false;

  /// For a native task, the agent's tool call that started it; the transcript folds that call into the delegation row.
  String? parentCall;

  /// For a native task, whether the agent reports what the child does as it works. When false only [message], [result] and the task
  /// the child was given are known.
  bool reportsProgress = true;

  /// What a native task was asked to do.
  String? brief;

  /// How many of the child's updates a native task has taken.
  int nativeApplied = 0;

  /// When the host first recorded it, and when it ended; null when the host sent no time, or the task has not ended.
  DateTime? started, ended;

  /// What the host said with its last state, and the task's reply once it has one.
  String? message, result;

  /// What the task's agent sent as session updates, oldest first, as far as the host kept them.
  final List<Map<String, Object?>> updates = [];

  /// How many updates the task has had, those forgotten for the bound included, so a reader that applied some knows how many are new.
  int updatesSeen = 0;

  /// The task that started this one, when the host names it in the task's metadata. A task a task's own delegation call answered
  /// for is nested under it as well ([DelegateTasks.parentOf]).
  String? parent;

  /// The ids this task's own delegation calls named in their results.
  final Set<String> nested = {};

  /// When the task last changed or said something, as far as the app heard.
  DateTime? updated;

  /// Whether the connection that carried the task ended while it ran: how it ended is not known.
  bool lost = false;
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

  /// The tasks the host runs and has not ended: what Stop all asks the host to stop.
  List<DelegateTask> get hostLive => [for (final task in live) if (!task.native) task];

  /// The task that started [task]: the one the host's metadata names, else the first whose own delegation call named it.
  DelegateTask? parentOf(DelegateTask task) {
    final named = task.parent;
    if (named != null && named != task.id && _byId[named] != null) return _byId[named];
    for (final other in _byId.values) {
      if (other.id != task.id && other.nested.contains(task.id)) return other;
    }
    return null;
  }

  /// The tasks [id] started itself, in the order they were first heard of.
  List<DelegateTask> childrenOf(String id) => [
    for (final task in all) if (parentOf(task)?.id == id) task,
  ];

  /// Every task below [id], deepest first, so stopping them in order never leaves one running under a stopped parent.
  List<DelegateTask> descendantsOf(String id) {
    final out = <DelegateTask>[];
    final seen = <String>{id};
    void walk(String at) {
      for (final child in childrenOf(at)) {
        if (!seen.add(child.id)) continue;
        walk(child.id);
        out.add(child);
      }
    }
    walk(id);
    return out;
  }

  /// The native task started by the agent's tool call [call], or null.
  DelegateTask? nativeOf(String call) {
    for (final task in _byId.values) {
      if (task.native && task.parentCall == call) return task;
    }
    return null;
  }

  /// Takes the subagents the conversation's agent named ([children] of the model's conversation) as tasks beside the host's own, so
  /// one list and one view serve both. [agent] names the source they run on, and [now] stands for the time the agent did not say.
  /// Whether anything changed.
  bool syncNative(List<pkg.ChatChild> children, DateTime now, String agent) {
    var changed = false;
    for (final child in children) {
      final id = 'native:${child.id}';
      final held = _byId[id];
      final task = held ?? _task(id);
      if (task == null) continue;
      if (held == null) {
        task.native = true;
        task.source = agent;
        task.started = now;
        changed = true;
      }
      task.parentCall = child.parentToolCallId;
      task.reportsProgress = child.reportsProgress;
      final title = child.title ?? (child.task == null ? null : _text(child.task!.split('\n').first, 80));
      if (title != null && task.title != title) {
        task.title = title;
        changed = true;
      }
      if (task.title == id) task.title = 'Subagent';
      if (task.brief != child.task) {
        task.brief = child.task;
        changed = true;
      }
      if (child.result != null && task.result != child.result) {
        task.result = child.result;
        changed = true;
      }
      final phase = switch (child.state) {
        pkg.ChildState.starting => TaskPhase.submitted,
        pkg.ChildState.working => TaskPhase.working,
        pkg.ChildState.completed => TaskPhase.completed,
        pkg.ChildState.failed => TaskPhase.failed,
        pkg.ChildState.cancelled => TaskPhase.canceled,
        null => null,
      };
      if (phase != null && phase != task.phase && !task.phase.ended) {
        task.phase = phase;
        task.updated = now;
        if (phase.ended) {
          task.ended = now;
          final span = child.usage?.durationMs;
          if (span != null) task.started = now.subtract(Duration(milliseconds: span));
        }
        if (phase == TaskPhase.failed) task.message = child.result;
        changed = true;
      }
      final updates = child.updates;
      while (task.nativeApplied < updates.length) {
        final raw = Map<String, Object?>.from(updates[task.nativeApplied].raw);
        // The mark belongs to the conversation that holds the children; a thread of its own would take it for a child's.
        final meta = raw['_meta'];
        if (meta is Map) raw['_meta'] = {for (final entry in meta.entries) if (entry.key != pkg.childMetaKey) entry.key: entry.value};
        task.updates.add(raw);
        task.updated = now;
        task.updatesSeen++;
        task.nativeApplied++;
        if (task.updates.length > maxTaskUpdates) task.updates.removeAt(0);
        changed = true;
      }
    }
    return changed;
  }

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

  static final _namedTask = RegExp(r'\\?"taskId\\?"\s*:\s*\\?"([^"\\]{1,200})');

  /// Notes the tasks a tool call's content of [task] names, which are the ones it started.
  static void _noteNested(DelegateTask task, Map<String, Object?> update) {
    final kind = update['sessionUpdate'];
    if ((kind != 'tool_call' && kind != 'tool_call_update') || task.nested.length >= maxTasks) return;
    final content = update['content'] ?? update['rawOutput'];
    if (content == null) return;
    for (final match in _namedTask.allMatches(jsonEncode(content))) {
      final id = match.group(1)!;
      if (id != task.id) task.nested.add(id);
    }
  }

  /// Takes the params of one `_canoa/delegate/event`, heard at [at] when the caller keeps a clock. Whether the list changed.
  bool apply(Object? params, {DateTime? at}) {
    if (params is! Map) return false;
    final id = params['taskId'];
    if (id is! String || id.isEmpty || id.length > 200) return false;
    final update = params['update'];
    if (update is Map) {
      final task = _task(id);
      if (task == null) return false;
      task.updates.add(Map<String, Object?>.from(update));
      _noteNested(task, task.updates.last);
      task.updated = at ?? task.updated;
      task.updatesSeen++;
      if (task.updates.length > maxTaskUpdates) task.updates.removeAt(0);
      return true;
    }
    final event = params['event'];
    if (event is! Map) return false;
    final task = _task(id);
    if (task == null) return false;
    task.updated = at ?? task.updated;
    final whole = event['task'];
    if (whole is Map) {
      final metadata = whole['metadata'];
      if (metadata is Map) {
        task.title = _text(metadata['title'], 80) ?? task.title;
        task.source = _text(metadata['source']) ?? task.source;
        task.folder = _text(metadata['folder'], 1000) ?? task.folder;
        if (metadata['remote'] == true) task.remote = true;
        task.handed = _handed(metadata['context']) ?? task.handed;
        task.usage = TaskUsage.parse(metadata['usage']) ?? task.usage;
        task.parent = _text(metadata['parentTaskId']) ?? task.parent;
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
    for (final update in kept) {
      _noteNested(task, update);
    }
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
  /// A task the host stopped is not a native one, which the host never ran: [native] false leaves those as they are.
  bool cancelLive([DateTime? at, bool native = true]) {
    var changed = false;
    for (final task in _byId.values) {
      if (task.phase.ended || (task.native && !native)) continue;
      task.phase = TaskPhase.canceled;
      // Stopped by the host, it ended as it was told to; with the connection gone, how it ended is not known.
      task.lost = native;
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
      if (!waiting.isCompleted) {
        final error = decoded['error'];
        waiting.complete(error == null ? (decoded['result'] ?? const <String, Object?>{}) : DelegateRefusal(error is Map && error['message'] is String ? error['message'] as String : 'The host refused it.'));
      }
      return true;
    }
    return false;
  }

  /// What the host answered: its result, a [DelegateRefusal] when it refused, or null when it did not answer.
  Future<Object?> _ask(String method, Map<String, Object?> params, Duration timeout) async {
    final answer = await _askRaw(method, params, timeout);
    return answer is DelegateRefusal ? null : answer;
  }

  Future<Object?> _askRaw(String method, Map<String, Object?> params, Duration timeout) async {
    // Unguessable, so no message of the agent's can pass for the host's answer.
    final nonce = [for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join();
    final id = '${Brand.program}-delegate-${_next++}-$nonce';
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

  /// What [sessionId] may hand a task to; null when the host did not say. [source] asks for that agent's models too.
  Future<DelegateTargets?> targets(String sessionId, {String? source, Duration timeout = const Duration(seconds: 45)}) async =>
      DelegateTargets.parse(await _ask(delegateTargetsMethod, {'sessionId': sessionId, 'source': ?source}, timeout));

  /// Asks the host to start [request] as a task of [sessionId]. A refusal says why; no answer is uncertain, since the task may have
  /// been made: nothing is sent again here.
  Future<DelegateOutcome> start(String sessionId, DelegateRequest request, {Duration timeout = const Duration(seconds: 60)}) async {
    final answer = await _askRaw(delegateStartMethod, request.params(sessionId), timeout);
    if (answer is DelegateRefusal) return DelegateRefused(answer.message);
    if (answer is Map && answer['taskId'] is String) return DelegateStarted(answer['taskId'] as String, repeated: answer['repeated'] == true);
    return const DelegateUncertain();
  }

  /// What handing [kind] (`messages` or `files`) of [sessionId] on would give a task now; null when the host has none to give.
  Future<ContextPreview?> contextPreview(String sessionId, String kind, {Duration timeout = const Duration(seconds: 10)}) async =>
      ContextPreview.parse(await _ask(delegateContextMethod, {'sessionId': sessionId, 'kind': kind}, timeout));

  /// Narrows what [sessionId] may delegate to ([sources]), sets the agent a task with none named goes to ([defaultSource]) and how
  /// many tasks run at once ([maxRunning]); a null clears that setting, since the call states the whole setup. Whether the host took it.
  /// [context] lists what the session's agent may attach to a task by its own call (`messages`, `files`; empty allows none).
  Future<bool> setup(String sessionId, {List<String>? sources, String? defaultSource, int? maxRunning, List<String>? context, Duration timeout = const Duration(seconds: 10)}) async =>
      await _ask(delegateSetupMethod, {'sessionId': sessionId, 'sources': sources, 'default': defaultSource, 'maxRunning': maxRunning, 'context': context ?? const <String>[]}, timeout) != null;

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

/// The host's record of the context a task was handed, in words; null when it holds none.
String? _handed(Object? raw) {
  if (raw is! Map || raw['kind'] is! String) return null;
  final items = raw['items'] is List ? (raw['items'] as List).length : 0;
  final what = switch (raw['kind']) {
    'messages' => '$items ${items == 1 ? 'turn' : 'turns'} of the conversation',
    'files' => '$items ${items == 1 ? 'file' : 'files'} it touched',
    final other => other as String,
  };
  final bytes = raw['bytes'] is int ? ', ${raw['bytes']} bytes' : '';
  return '$what$bytes${raw['truncated'] == true ? ', cut to fit' : ''}';
}
