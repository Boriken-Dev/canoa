import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart' show ControlClient, ControlFailed;
import '../desktop/host_control.dart';

/// One mail account of the host's canoa.cfg, as `mail/accounts` words it. It carries where the mailbox is and whether its password
/// is there, never the password.
final class MailAccountInfo {
  const MailAccountInfo({required this.name, required this.backend, required this.state, this.server, this.email, this.user});

  final String name;

  /// `ews` (Exchange Server) or `outlook` (classic Outlook for Windows).
  final String backend;

  /// What the host says of the sign-in: `password stored`, `needs a password`, `signs in with the Windows logon`, and so on.
  final String state;
  final String? server, email, user;

  static MailAccountInfo? parse(Object? json) {
    if (json is! Map || json['name'] is! String || json['backend'] is! String) return null;
    String? text(String key) => json[key] is String && (json[key] as String).isNotEmpty ? json[key] as String : null;
    return MailAccountInfo(
      name: json['name']! as String,
      backend: json['backend']! as String,
      state: text('state') ?? '',
      server: text('server'),
      email: text('email'),
      user: text('user'),
    );
  }

  String get backendTitle => backend == 'outlook' ? 'Outlook' : 'Exchange';

  /// Where the mailbox is, or whose it is.
  String? get where => server ?? email;

  /// Whether the account lacks the password it signs in with.
  bool get needsPassword => state == 'needs a password' || state.startsWith('needs ');
}

/// An account to add. The password is sent once and held nowhere after.
final class NewMailAccount {
  const NewMailAccount({required this.name, required this.backend, this.email, this.server, this.user, this.password, this.profile});

  final String name, backend;
  final String? email, server, user, password, profile;

  Map<String, Object?> toParams() => {
    'name': name,
    'backend': backend,
    'email': ?email,
    'server': ?server,
    'user': ?user,
    'password': ?password,
    'profile': ?profile,
  };
}

/// What `mail/test` answers: whether the host signed in to the mailbox with what it holds, and why not in the words of the connector.
final class MailTest {
  const MailTest({required this.ok, this.folders, this.problem});

  final bool ok;
  final int? folders;
  final String? problem;

  static MailTest parse(Object? json) {
    final map = json is Map ? json : const {};
    return MailTest(
      ok: map['ok'] == true,
      folders: map['folders'] is num ? (map['folders'] as num).toInt() : null,
      problem: map['problem'] is String ? map['problem'] as String : null,
    );
  }
}

/// What the app asks of the host about mail accounts. Nothing it returns holds a password.
abstract interface class MailHost {
  /// The accounts in canoa.cfg with the state of each.
  Future<List<MailAccountInfo>> list();

  /// Adds [account]: the host keeps its password in its account store and writes the account's table, which holds none.
  Future<MailAccountInfo> add(NewMailAccount account);

  /// Has the host sign in to the account with what it holds and list its folders.
  Future<MailTest> test(String name);

  /// Forgets the stored password and takes the account out of canoa.cfg. Nothing changes in the mailbox.
  Future<void> remove(String name);
}

/// The host's mail accounts over its control channel.
final class ControlMailHost implements MailHost {
  ControlMailHost(this._control);

  final HostControl _control;

  Future<Object?> _call(String method, Map<String, Object?> params, [Duration? wait]) async =>
      (await _control.ready(wait: false)).call(method, params, null, wait);

  @override
  Future<List<MailAccountInfo>> list() async {
    final result = await _call('mail/accounts', const {});
    final accounts = result is Map ? result['accounts'] : null;
    return [if (accounts is List) for (final each in accounts) ?MailAccountInfo.parse(each)];
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

/// The mail accounts Settings lists: what the host says of each, and the result of the last sign-in test of each.
class MailAccountsController extends ChangeNotifier {
  MailAccountsController({required this.host});

  final MailHost host;

  List<MailAccountInfo> _accounts = const [];
  final _tests = <String, MailTest>{};
  final _testing = <String>{};
  String? _problem;
  bool _loading = false, _disposed = false;

  List<MailAccountInfo> get accounts => _accounts;
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
      _accounts = await host.list();
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
