// A host's control channel as far as the program Store goes, scripted: the answers to store/list, report, latest, plan, rollback
// and remove, and an apply that can say things, ask the person and wait to be cancelled. The program names in it are made up.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/link_secret.dart' show LinkSecretIdentity, linkSecretIdentity;
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/store/programs.dart';
import 'package:coqui_acp/secret.dart';

import 'fake_accounts.dart' show linkEnds, linkSecret;

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

  /// Puts a question to the person the way the real host does: sealed to the key the link was proved by, as a sign-in's is. Gives
  /// what the host reads once it opens the answer (`{values, secrets}`), or `{cancelled: true}` when the person did not answer.
  Future<Map<String, Object?>> ask(String title, Map<String, Object?> schema) async {
    final ask = 'canoa-secret-${++_asked}';
    final ends = await linkEnds();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final text = jsonEncode(schema);
    final context = AcpSecretContext(sessionId: 'accounts', requestId: ask, message: title, schema: text);
    final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, ends.host, ends.app.identity.prover.id, now);
    final properties = schema['properties'] is Map ? (schema['properties'] as Map).cast<String, Object?>() : const <String, Object?>{};
    bool secret(String name) => properties[name] is Map && (properties[name] as Map)['writeOnly'] == true;
    final answer = Completer<Object?>();
    _link._questions[ask] = answer;
    _link._emit({
      'jsonrpc': '2.0',
      'id': ask,
      'method': acpSecretRequestMethod,
      'params': AcpSecretRequestParams(context: context, recipient: receiver.recipient).toJson(),
    });
    final given = await answer.future;
    final reply = AcpSecretRequestReply.fromJson(given, secret);
    if (reply is! AcpSecretAccepted) return {'cancelled': true};
    final opened = await receiver.openValues(ends.hostVerifier, ends.app.identity.prover.id, reply.plain, reply.sealed, reply.auth, now);
    return {
      'values': opened.plain,
      'secrets': {for (final MapEntry(:key, :value) in opened.secrets.entries) key: value.text},
    };
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
    this.runtime = false,
    this.extensionType,
    this.summary,
    this.icon,
    this.commands = const [],
    this.endpoints = const [],
    this.homepage,
  });

  /// What `store/list` says of the program besides its name, when the store says it.
  final String? summary, icon, homepage;
  final List<String> commands, endpoints;

  /// The id of the type an extension adds that the store belongs to, which `store/list` gives only for that type.
  final String? extensionType;

  final String provider, title;

  /// Whether the store is of the Runtimes type, which `store/list` gives only when asked for `runtimes`.
  final bool runtime;
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

  /// The types `store/extensions` answers with.
  List<Map<String, Object?>> extensionTypes = const [];

  /// The host's `[host]` table as `config/get`, `config/check` and `config/set` have it; null when canoa.cfg has none.
  Map<String, Object?>? hostTable;

  /// The grants `host/grants` says this key holds.
  List<String> grants = ['store.read', 'store.programs'];

  /// What `config/check` refuses with, in the host's words; null accepts.
  String? checkRefusal;

  /// What `extension/catalogs` answers: the extensions catalogs and their entries, as the host's index words them.
  Object? catalogIndex;

  /// What `extension/plan` refuses with, in the host's words; null makes a plan of the entry.
  String? extensionPlanRefusal;

  /// What `extension/apply` refuses with; null installs, which writes `[host.extensions.NAME]` as the host does.
  String? extensionApplyRefusal;

  /// What `shell/list` answers: the shells found, the host's choice and what each agent entry is told.
  Map<String, Object?> shellReport = {'shells': <Object?>[], 'agents': <Object?>[]};

  /// Foundry Local's models as `foundry/models` lists them (`id`, `cached`, `loaded` and the rest), and whether the host found
  /// Foundry Local; `foundry/model` changes them the way the real command line does.
  List<Map<String, Object?>> foundryModels = [];
  bool foundryInstalled = false;

  /// Every line the app sent on any link, whole, to look for what must never be in one.
  final wire = <String>[];

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
  ProgramStores programs({String? kind}) {
    final shared = client();
    return ProgramStores(control: () async => shared, open: () async => client(), kind: kind);
  }

  FakeStore store(String provider) => stores.firstWhere((each) => each.provider == provider);

  Object? _answer(String method, Map<String, Object?> given) {
    // What the host grants this key is not a store request, so it is not in [calls].
    if (method == 'host/grants') return {'grants': grants};
    if (method == 'foundry/models' || method == 'foundry/model') {
      calls.add(method);
      params.putIfAbsent(method, () => []).add(given);
      if (failures[method] case final failure?) throw failure;
      if (method == 'foundry/models') return {'installed': foundryInstalled, 'models': foundryModels};
      final model = foundryModels.firstWhere((each) => each['id'] == given['model']);
      switch (given['action']) {
        case 'download':
          model['cached'] = true;
        case 'load':
          if (model['cached'] != true) throw ControlFailed(-32002, '${model['id']} is not downloaded');
          model['loaded'] = true;
        case 'unload':
          model['loaded'] = false;
      }
      return {'ok': true, 'model': given['model'], 'action': given['action']};
    }
    if (method == 'speech/enable') {
      calls.add(method);
      params.putIfAbsent(method, () => []).add(given);
      if (failures[method] case final failure?) throw failure;
      return {'name': 'foundry', 'provider': 'speech_endpoint', 'model': given['model'], 'saved': true};
    }
    final provider = given['provider'] as String?;
    final key = '$method ${provider ?? ''}'.trim();
    calls.add(key);
    params.putIfAbsent(method, () => []).add(given);
    if (failures[method] ?? failures[key] case final failure?) throw failure;
    if (method == 'store/extensions') return {'types': extensionTypes};
    if (method == 'extension/catalogs') return catalogIndex ?? {'schema': 1, 'sources': <Object?>[], 'entries': <Object?>[]};
    if (method == 'extension/plan') {
      if (extensionPlanRefusal != null) throw ControlFailed(-32002, extensionPlanRefusal!);
      return {
        'action': 'install',
        'version': null,
        'downloads': [
          {'url': 'https://example.org/${given['name']}.lib', 'bytes': null, 'check': {'kind': 'sha256', 'value': 'ab' * 32}},
        ],
        'steps': <Object?>[],
        'changes': [
          {'kind': 'canoa_cfg', 'target': '[host.extensions.${given['name']}]', 'detail': 'Names the library.', 'undone_by_uninstall': true},
        ],
        'notices': ['An extension is code the host loads into its own process: it runs with your access.'],
        'token': 'token-${given['name']}',
        'extension': {'catalog': given['catalog'], 'name': given['name'], 'provider': 'p'},
      };
    }
    if (method == 'extension/apply') {
      if (extensionApplyRefusal != null) throw ControlFailed(-32002, extensionApplyRefusal!);
      final name = ((given['plan'] as Map)['extension'] as Map)['name'] as String;
      hostTable = {
        ...?hostTable,
        'extensions': {
          ...?(hostTable?['extensions'] as Map?)?.cast<String, Object?>(),
          name: {'library': '/home/extensions/$name/$name.lib', 'provider': 'p', 'sha256': 'ab' * 32},
        },
      };
      return {'installed': true, 'name': name, 'path': '/home/extensions/$name/$name.lib', 'restart': true};
    }
    if (method == 'extension/remove') {
      final extensions = {...?(hostTable?['extensions'] as Map?)?.cast<String, Object?>()};
      if (extensions.remove(given['name']) == null) throw ControlFailed(-32002, 'there is no extension ${given['name']} in canoa.cfg');
      hostTable = extensions.isEmpty ? null : {...?hostTable, 'extensions': extensions};
      return {'removed': true, 'files_removed': true, 'restart': true};
    }

    if (method == 'shell/list') return shellReport;
    if (method == 'config/get') return {'section': given['section'], 'value': hostTable, 'restart': true};
    if (method == 'config/check') {
      if (checkRefusal != null) throw ControlFailed(-32002, checkRefusal!);
      return {'valid': true};
    }
    if (method == 'config/set') {
      hostTable = given['value'] is Map ? {for (final MapEntry(:key, :value) in (given['value'] as Map).entries) '$key': value} : null;
      return {'section': given['section'], 'restart': true};
    }
    if (method == 'store/list') {
      final extension = given['kind'] == 'extension';
      return {
        'stores': [
          for (final each in stores.where(
            (each) => extension
                ? each.extensionType == given['type']
                : each.extensionType == null && each.runtime == (given['kind'] == 'runtimes'),
          ))
            {
              'provider': each.provider,
              'program': each.provider.replaceAll('_store', ''),
              'title': each.title,
              'publisher': 'A Publisher',
              'serves': each.serves,
              'channels': <String>[],
              'summary': ?each.summary,
              'icon': ?each.icon,
              'homepage': ?each.homepage,
              'commands': each.commands,
              'endpoints': each.endpoints,
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

final class _FakeLink implements HostLink, HostLinkBinding {
  _FakeLink(this._host);

  final FakeStoreHost _host;
  final _out = StreamController<String>(sync: false);
  final closed = Completer<void>();
  final _questions = <String, Completer<Object?>>{};

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<LinkSecretIdentity?> secretBinding() => linkSecretIdentity(linkSecret);

  void _emit(Map<String, Object?> message) {
    if (!_out.isClosed) _out.add(jsonEncode(message));
  }

  void _notify(Map<String, Object?> params) => _emit({'jsonrpc': '2.0', 'method': 'store/progress', 'params': params});

  @override
  Future<void> send(String message) async {
    if (closed.isCompleted) throw StateError('the link has ended');
    _host.wire.add(message);
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method'];
    if (method == null) {
      // An answer to a question the apply asked.
      final waiting = _questions.remove(request['id']);
      if (waiting != null && request['result'] is Map) waiting.complete(request['result']);
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
