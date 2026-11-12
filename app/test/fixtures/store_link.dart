// A host's control channel as far as the program Store goes, scripted: the answers to store/list, report, latest, plan, rollback
// and remove, and an apply that can say things, ask the person and wait to be cancelled. The program names in it are made up.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/store/programs.dart';

/// What an apply in progress can do: tell the person, ask them, and notice the link closing.
final class FakeApply {
  FakeApply._(this._link, this.id, this.params);

  final _FakeLink _link;
  final int id;
  final Map<String, Object?> params;
  int _asked = 0;

  /// Completes when the caller closes the link, which is how the real host is told to stop.
  Future<void> get closed => _link.closed.future;

  void say(String text) => _link._notify({'id': id, 'text': text});

  void progress(String text, double fraction) => _link._notify({'id': id, 'text': text, 'fraction': fraction});

  /// Puts a question to the person and gives the answer (`{values, secrets}`).
  Future<Map<String, Object?>> ask(String title, Map<String, Object?> schema) {
    final ask = 'ask-${++_asked}';
    final answer = Completer<Map<String, Object?>>();
    _link._questions[ask] = answer;
    _link._emit({'jsonrpc': '2.0', 'id': ask, 'method': 'store/ask', 'params': {'title': title, 'schema': schema}});
    return answer.future;
  }
}

/// One store of the fake host.
final class FakeStore {
  FakeStore(
    this.provider, {
    required this.title,
    this.serves = const [],
    this.copies = const [],
    this.latest,
    this.plan,
    this.latestFails,
  });

  final String provider, title;
  List<Map<String, Object?>> serves;
  List<Map<String, Object?>> copies;

  /// The version the publisher says is newest; null answers that it is not known.
  String? latest;
  String? latestFails;

  /// The plan `store/plan` answers with, by action; the token is added.
  Map<String, Map<String, Object?>>? plan;
}

/// The host's end: stores, a record of every request, and handlers a test replaces.
final class FakeStoreHost {
  FakeStoreHost(this.stores);

  final List<FakeStore> stores;

  /// Every request as `method provider`, in the order they came.
  final calls = <String>[];

  /// The params of each request, by method, in the order they came.
  final params = <String, List<Map<String, Object?>>>{};

  /// How many links were opened.
  int links = 0;

  /// Links closed by the caller.
  int closedLinks = 0;

  /// Answers [method] with an error instead, by the provider it names (or any).
  final failures = <String, ControlFailed>{};

  /// What `store/apply` does. By default it says one line and succeeds with the plan's version.
  Future<Object?> Function(FakeApply apply)? onApply;

  /// What a successful apply changes in the stores, such as the copy now found.
  void Function(FakeStore store, Map<String, Object?> plan)? afterApply;

  HostLink open() {
    links++;
    return _FakeLink(this);
  }

  /// A client over a link of this host.
  ControlClient client() => ControlClient(open());

  /// A [ProgramStores] over this host, as the app makes one: the shared link for questions, a link of its own for each apply.
  ProgramStores programs() {
    final shared = client();
    return ProgramStores(control: () async => shared, open: () async => client());
  }

  FakeStore store(String provider) => stores.firstWhere((each) => each.provider == provider);

  Object? _answer(String method, Map<String, Object?> given) {
    final provider = given['provider'] as String?;
    final key = '$method ${provider ?? ''}'.trim();
    calls.add(key);
    params.putIfAbsent(method, () => []).add(given);
    if (failures[method] ?? failures[key] case final failure?) throw failure;
    if (method == 'store/list') {
      return {
        'stores': [
          for (final each in stores)
            {
              'provider': each.provider,
              'program': each.provider.replaceAll('_store', ''),
              'title': each.title,
              'publisher': 'A Publisher',
              'serves': each.serves,
              'channels': <String>[],
            },
        ],
      };
    }
    final store = stores.where((each) => each.provider == provider).firstOrNull;
    if (store == null) throw ControlFailed(-32003, 'this host has no store $provider');
    switch (method) {
      case 'store/report':
        return {'copies': store.copies};
      case 'store/latest':
        if (store.latest == null) return {'known': false, 'unknown_because': store.latestFails ?? 'the publisher did not answer'};
        return {'known': true, 'version': store.latest};
      case 'store/plan':
        final action = given['action'] as String;
        final made = store.plan?[action] ?? {'action': action, 'version': store.latest, 'downloads': <Object?>[], 'changes': <Object?>[]};
        return {...made, 'action': action, 'token': 'token-${calls.length}'};
      case 'store/rollback':
        return {'provider': provider, 'version': '0.9.0', 'dir': '/programs/x/0.9.0'};
      case 'store/remove':
        return {'provider': provider, 'removed': true};
    }
    throw ControlFailed(-32601, 'this host has no such method');
  }
}

final class _FakeLink implements HostLink {
  _FakeLink(this._host);

  final FakeStoreHost _host;
  final _out = StreamController<String>(sync: false);
  final closed = Completer<void>();
  final _questions = <String, Completer<Map<String, Object?>>>{};

  @override
  Stream<String> get messages => _out.stream;

  void _emit(Map<String, Object?> message) {
    if (!_out.isClosed) _out.add(jsonEncode(message));
  }

  void _notify(Map<String, Object?> params) => _emit({'jsonrpc': '2.0', 'method': 'store/progress', 'params': params});

  @override
  Future<void> send(String message) async {
    if (closed.isCompleted) throw StateError('the link has ended');
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method'];
    if (method == null) {
      // An answer to a question the apply asked.
      final waiting = _questions.remove(request['id']);
      if (waiting != null && request['result'] is Map) {
        waiting.complete({for (final MapEntry(:key, :value) in (request['result'] as Map).entries) '$key': value});
      }
      return;
    }
    final id = request['id'];
    final given = {for (final MapEntry(:key, :value) in ((request['params'] as Map?) ?? const {}).entries) '$key': value};
    unawaited(() async {
      try {
        final Object? result;
        if (method == 'store/apply') {
          result = await _apply(id as int, given);
        } else {
          result = _host._answer(method as String, given);
        }
        _emit({'jsonrpc': '2.0', 'id': id, 'result': result});
      } on ControlFailed catch (failure) {
        _emit({'jsonrpc': '2.0', 'id': id, 'error': {'code': failure.code, 'message': failure.message}});
      } on Object {
        // The link closed under it.
      }
    }());
  }

  Future<Object?> _apply(int id, Map<String, Object?> given) async {
    final provider = given['provider'] as String?;
    _host.calls.add('store/apply $provider');
    _host.params.putIfAbsent('store/apply', () => []).add(given);
    if (_host.failures['store/apply'] case final failure?) throw failure;
    final store = _host.store(provider!);
    final plan = {for (final MapEntry(:key, :value) in (given['plan'] as Map).entries) '$key': value};
    final apply = FakeApply._(this, id, given);
    final result = _host.onApply == null
        ? () async {
            apply.say('Installing ${store.title}');
            return {'applied': true, 'version': plan['version'], 'dir': '/programs/${store.provider}'};
          }()
        : _host.onApply!(apply);
    final done = await result;
    _host.afterApply?.call(store, plan);
    return done;
  }

  @override
  Future<void> close() async {
    if (closed.isCompleted) return;
    _host.closedLinks++;
    closed.complete();
    for (final waiting in _questions.values) {
      if (!waiting.isCompleted) waiting.completeError(StateError('closed'));
    }
    await _out.close();
  }
}

/// A report's copy as the host words it.
Map<String, Object?> copyOf(String version, {bool managed = true, String path = '/programs/x/current/x', Object? update}) => {
  'path': path,
  'version': version,
  'manager': managed ? 'CANOA' : 'UNKNOWN',
  'managed': managed,
  'self_updates': false,
  'update': update ?? {'kind': 'by_store'},
};

/// A plan as the host words it, the way an install of [version] would.
Map<String, Object?> planOf(
  String action,
  String version, {
  String? from,
  List<Map<String, Object?>> downloads = const [],
  List<Map<String, Object?>> changes = const [],
  List<String> steps = const [],
  List<String> notices = const [],
  int? bytes,
}) => {
  'action': action,
  'version': version,
  'from_version': from,
  'downloads': downloads,
  'steps': steps,
  'changes': changes,
  'bytes': bytes,
  'notices': notices,
};

/// The harness `claude`-like store of the fake host, serving a provider id.
FakeStore claudeLike({List<Map<String, Object?>> copies = const [], String? latest, Map<String, Map<String, Object?>>? plan}) => FakeStore(
  'one_store',
  title: 'Program One',
  serves: [
    {'provider': 'one', 'when': <String, Object?>{}},
  ],
  copies: copies,
  latest: latest,
  plan: plan,
);

/// A store whose program a stdio source selects by its harness setting.
FakeStore whenLike({List<Map<String, Object?>> copies = const [], String? latest}) => FakeStore(
  'two_store',
  title: 'Program Two',
  serves: [
    {'provider': 'stdio', 'when': {'harness': 'two'}},
  ],
  copies: copies,
  latest: latest,
);

/// Shorthand for tests that read a status.
ProgramStatus? statusOf(ProgramStores programs, String provider) => programs.status(provider);
