import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart' show ControlClient, ControlFailed;
import '../desktop/host_control.dart';

/// A mail backend offered by the connected host (`mail/accounts` `"backends"`): its identifier, human label, and schema/form for
/// adding an account.
final class MailBackendInfo {
  const MailBackendInfo({
    required this.id,
    required this.title,
    this.description,
    this.noSecretState,
    this.schema,
    this.form,
  });

  final String id;
  final String title;
  final String? description;
  final String? noSecretState;
  final Map<String, Object?>? schema;
  final Map<String, Object?>? form;

  static MailBackendInfo? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'];
    final title = json['title'];
    if (id is! String || id.isEmpty || title is! String || title.isEmpty) return null;
    return MailBackendInfo(
      id: id,
      title: title,
      description: json['description'] is String ? json['description']! as String : null,
      noSecretState: json['no_secret_state'] is String ? json['no_secret_state']! as String : null,
      schema: json['schema'] is Map ? (json['schema']! as Map).cast<String, Object?>() : null,
      form: json['form'] is Map ? (json['form']! as Map).cast<String, Object?>() : null,
    );
  }
}

/// One mail account of the host's canoa.cfg, as `mail/accounts` words it. It carries where the mailbox is and whether its password
/// is held; never the password itself.
final class MailAccountInfo {
  const MailAccountInfo({
    required this.name,
    required this.backend,
    this.backendTitle,
    required this.state,
    this.details = const [],
    this.settings = const {},
    this.server,
    this.email,
    this.user,
  });

  final String name;
  final String backend;
  final String? backendTitle;
  /// What the host says of the sign-in: `password stored`, `needs a password`, `signs in with the Windows logon`, `signed in by Outlook`, and so on.
  final String state;
  final List<String> details;
  final Map<String, Object?> settings;
  final String? server;
  final String? email;
  final String? user;

  static MailAccountInfo? parse(Object? json) {
    if (json is! Map) return null;
    final name = json['name'];
    final backend = json['backend'];
    final state = json['state'];
    if (name is! String || backend is! String || state is! String) return null;
    final rawDetails = json['details'];
    final details = [
      if (rawDetails is List)
        for (final item in rawDetails)
          if (item is String && item.isNotEmpty) item,
    ];
    final rawSettings = json['settings'];
    final settings = rawSettings is Map ? rawSettings.cast<String, Object?>() : const <String, Object?>{};
    return MailAccountInfo(
      name: name,
      backend: backend,
      backendTitle: json['backend_title'] is String ? json['backend_title']! as String : null,
      state: state,
      details: details,
      settings: settings,
      server: json['server'] is String ? json['server']! as String : null,
      email: json['email'] is String ? json['email']! as String : null,
      user: json['user'] is String ? json['user']! as String : null,
    );
  }

  /// Title shown for this account's backend, preferring the host-reported provider title and falling back to [backends].
  String titleWith(List<MailBackendInfo> backends) =>
      backendTitle ?? backends.where((b) => b.id == backend).firstOrNull?.title ?? backend;

  /// Short subtitle details displayed after the backend title and before the sign-in state.
  List<String> summaryDetails() {
    if (details.isNotEmpty) return details;
    return [?email, ?server, ?user];
  }
}

/// Accounts and backends returned by `mail/accounts`.
final class MailAccountsSnapshot {
  const MailAccountsSnapshot({
    required this.accounts,
    required this.backends,
  });

  final List<MailAccountInfo> accounts;
  final List<MailBackendInfo> backends;
}

/// Parameters sent to `mail/add`: account name, backend id, and dynamic form values matching the backend's schema.
final class NewMailAccount {
  const NewMailAccount({
    required this.name,
    required this.backend,
    this.settings = const {},
    this.passwordEnv,
    this.timeoutSecs,
  });

  final String name;
  final String backend;
  final Map<String, Object?> settings;
  final String? passwordEnv;
  final int? timeoutSecs;

  Map<String, Object?> toParams() => {
        'name': name,
        'backend': backend,
        ...settings,
        if (passwordEnv != null) 'password_env': passwordEnv,
        if (timeoutSecs != null) 'timeout_secs': timeoutSecs,
      };
}

/// What `mail/test` answered: either signed in with a folder count, or why not.
final class MailTest {
  const MailTest({required this.ok, this.folders, this.problem});

  final bool ok;
  final int? folders;
  final String? problem;

  static MailTest parse(Object? json) {
    if (json is! Map) return const MailTest(ok: false, problem: 'The host answered something unexpected');
    final folders = json['folders'];
    return MailTest(
      ok: json['ok'] == true,
      folders: folders is num ? folders.toInt() : null,
      problem: json['problem'] is String ? json['problem']! as String : null,
    );
  }
}

/// Where Settings' mail section reads and changes the host's `[host.mail]`.
abstract interface class MailHost {
  Future<MailAccountsSnapshot> list();
  Future<MailAccountInfo> add(NewMailAccount account);
  Future<MailTest> test(String name);
  Future<void> remove(String name);
}

/// The real control channel of a connected host.
final class ControlMailHost implements MailHost {
  ControlMailHost(this._control);

  final HostControl _control;

  Future<Object?> _call(String method, Map<String, Object?> params, [Duration? wait]) async =>
      (await _control.ready(wait: false)).call(method, params, null, wait);

  @override
  Future<MailAccountsSnapshot> list() async {
    final result = await _call('mail/accounts', const {});
    final accounts = result is Map ? result['accounts'] : null;
    final backends = result is Map ? result['backends'] : null;
    return MailAccountsSnapshot(
      accounts: [if (accounts is List) for (final each in accounts) ?MailAccountInfo.parse(each)],
      backends: [if (backends is List) for (final each in backends) ?MailBackendInfo.parse(each)],
    );
  }

  @override
  Future<MailAccountInfo> add(NewMailAccount account) async {
    final result = await _call('mail/add', account.toParams());
    final made = result is Map ? MailAccountInfo.parse(result['account']) : null;
    if (made == null) throw StateError('the host did not say which account it made');
    return made;
  }

  // A mailbox that is slow to answer is the host's to give up on (`timeout_secs`), and the link the answer waits on is one of its own,
  // so nothing else this app asks of the host waits behind it.
  @override
  Future<MailTest> test(String name) async {
    ControlClient? client;
    try {
      client = await _control.open(wait: false);
      return MailTest.parse(await client.call('mail/test', {'name': name}, null, const Duration(minutes: 2)));
    } finally {
      unawaited(client?.close());
    }
  }

  @override
  Future<void> remove(String name) async {
    await _call('mail/remove', {'name': name});
  }
}

/// The mail accounts and available backends Settings lists: what the host says of each, and the result of the last sign-in test of each.
class MailAccountsController extends ChangeNotifier {
  MailAccountsController({required this.host});

  final MailHost host;

  List<MailAccountInfo> _accounts = const [];
  List<MailBackendInfo> _backends = const [];
  final _tests = <String, MailTest>{};
  final _testing = <String>{};
  String? _problem;
  bool _loading = false, _disposed = false;

  List<MailAccountInfo> get accounts => _accounts;
  List<MailBackendInfo> get backends => _backends;
  String? get problem => _problem;
  bool get loading => _loading;

  MailAccountInfo? named(String name) => _accounts.where((each) => each.name == name).firstOrNull;

  /// What the last test of [name] found; null before one ran.
  MailTest? testOf(String name) => _tests[name];

  bool isTesting(String name) => _testing.contains(name);

  Future<void> refresh() async {
    _loading = true;
    _notify();
    try {
      final snapshot = await host.list();
      _accounts = snapshot.accounts;
      _backends = snapshot.backends;
      _problem = null;
    } on ControlFailed catch (error) {
      _problem = error.code == -32001 ? 'This key may not manage the mail accounts of this host.' : error.message;
    } on Object catch (error) {
      _problem = 'The host did not list the mail accounts: $error';
    } finally {
      _loading = false;
      _notify();
    }
  }

  /// Adds [account]; the host's refusal is thrown as it words it, and the list is read again either way.
  Future<MailAccountInfo> add(NewMailAccount account) async {
    try {
      return await host.add(account);
    } finally {
      await refresh();
    }
  }

  /// Signs in to [name] and lists its folders, and keeps what it found for [testOf].
  Future<MailTest> test(String name) async {
    _testing.add(name);
    _tests.remove(name);
    _notify();
    MailTest result;
    try {
      result = await host.test(name);
    } on ControlFailed catch (error) {
      result = MailTest(ok: false, problem: error.message);
    } on Object catch (error) {
      result = MailTest(ok: false, problem: 'The host could not be reached: $error');
    }
    _testing.remove(name);
    _tests[name] = result;
    _notify();
    return result;
  }

  Future<void> remove(String name) async {
    try {
      await host.remove(name);
    } finally {
      _tests.remove(name);
      await refresh();
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
