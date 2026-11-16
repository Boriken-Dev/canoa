import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageReport;
import 'package:flutter/foundation.dart';

import '../accounts/accounts_controller.dart';
import '../desktop/host_control.dart';

/// A host that answers `usage/limits`: its name as the person knows it, and how to ask it.
abstract interface class UsageSource {
  String get name;

  /// The `accounts` array of the host's `usage/limits` answer.
  Future<List<Object?>> limits();
}

/// The host this app's control link reaches.
final class ControlUsageSource implements UsageSource {
  ControlUsageSource(this.name, this._control);

  @override
  final String name;
  final HostControl _control;

  @override
  Future<List<Object?>> limits() async {
    final result = await (await _control.ready(
      wait: false,
    )).call('usage/limits', const {});
    final accounts = result is Map ? result['accounts'] : null;
    return accounts is List ? accounts : const [];
  }
}

/// The provider an account is listed under when nothing says which it is.
const otherProvider = 'Other';

/// What is known of one account's allowances: the newest reading among the hosts that reported its identity.
final class AccountLimits {
  const AccountLimits({
    required this.id,
    required this.name,
    required this.provider,
    required this.hosts,
    required this.report,
    required this.readAt,
  });

  /// The identity the agent reported; two hosts reporting the same one are the same allowance.
  final String id;
  final String name, provider;

  /// The hosts that reported it, in the order they were asked.
  final List<String> hosts;
  final UsageReport report;

  /// Unix seconds the figures were read from the provider, or the host heard them when that is not said.
  final int readAt;
}

/// Which accounts the Usage section shows and the figures of each, over every host it asks. The index, the toolbar's filters and
/// the main view read the same selection.
class UsageController extends ChangeNotifier {
  UsageController({required this.sources, this.accounts, this.now});

  final List<UsageSource> sources;

  /// Names and providers of accounts this computer's host lists; an identity it does not list is shown by what was reported.
  final AccountsController? accounts;

  /// Unix seconds; the clock's when null.
  final int Function()? now;

  final _heard = <String, List<_Heard>>{};
  final _problems = <String, String>{};
  bool _loading = false, _disposed = false;

  String? _host, _provider, _account;

  /// The filters: a host's name, a provider, an account's identity; null is all.
  String? get host => _host;
  String? get provider => _provider;
  String? get account => _account;
  bool get loading => _loading;

  /// Why a host answered nothing, by its name.
  Map<String, String> get problems => Map.unmodifiable(_problems);

  int get clock => now?.call() ?? DateTime.now().millisecondsSinceEpoch ~/ 1000;

  /// Chooses what to show. An account fixes the provider it is listed under; a provider alone clears the account.
  void select({String? host, String? provider, String? account}) {
    _host = host;
    _provider = provider;
    _account = account;
    if (account != null) {
      final found = all.where((each) => each.id == account).firstOrNull;
      if (found != null) _provider = found.provider;
    }
    _notify();
  }

  void selectHost(String? host) {
    _host = host;
    _clearMissing();
    _notify();
  }

  void selectProvider(String? provider) {
    _provider = provider;
    _clearMissing();
    _notify();
  }

  void selectAccount(String? account) {
    _account = account;
    _notify();
  }

  // An account the other filters now leave out is dropped, so a choice made before never leaves the view empty.
  void _clearMissing() {
    if (_account != null && !visible.any((each) => each.id == _account)) {
      _account = null;
    }
  }

  /// Asks every host again.
  Future<void> refresh() async {
    if (_loading) return;
    _loading = true;
    _notify();
    try {
      await Future.wait([for (final source in sources) _ask(source)]);
    } finally {
      _loading = false;
      _notify();
    }
  }

  Future<void> _ask(UsageSource source) async {
    try {
      final entries = await source.limits();
      _heard[source.name] = [for (final each in entries) ?_Heard.parse(each)];
      _problems.remove(source.name);
    } on Object catch (error) {
      _problems[source.name] = '$error';
    }
  }

  /// Every account any host reported, one for each identity, the newest reading winning, in a stable order: provider, then name.
  List<AccountLimits> get all {
    final byId = <String, List<(String, _Heard)>>{};
    for (final source in sources) {
      for (final heard in _heard[source.name] ?? const <_Heard>[]) {
        byId.putIfAbsent(heard.id, () => []).add((source.name, heard));
      }
    }
    final merged = <AccountLimits>[];
    for (final MapEntry(:key, :value) in byId.entries) {
      var newest = value.first.$2;
      for (final (_, heard) in value) {
        if (heard.readAt > newest.readAt) newest = heard;
      }
      final info = accounts?.accounts
          .where((each) => each.id == key)
          .firstOrNull;
      merged.add(
        AccountLimits(
          id: key,
          name: info?.name ?? key,
          provider: info?.harness ?? otherProvider,
          hosts: [for (final (host, _) in value) host],
          report: newest.report,
          readAt: newest.readAt,
        ),
      );
    }
    merged.sort((a, b) {
      final provider = a.provider == b.provider
          ? 0
          : a.provider == otherProvider
          ? 1
          : b.provider == otherProvider
          ? -1
          : a.provider.compareTo(b.provider);
      return provider != 0
          ? provider
          : a.name.toLowerCase().compareTo(b.name.toLowerCase());
    });
    return merged;
  }

  /// What the filters leave.
  List<AccountLimits> get visible => [
    for (final each in all)
      if ((_host == null || each.hosts.contains(_host)) &&
          (_provider == null || each.provider == _provider) &&
          (_account == null || each.id == _account))
        each,
  ];

  /// The providers any account is listed under.
  List<String> get providers =>
      {for (final each in all) each.provider}.toList();

  List<AccountLimits> of(String provider) => [
    for (final each in all)
      if (each.provider == provider) each,
  ];

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

final class _Heard {
  const _Heard(this.id, this.report, this.readAt);

  static _Heard? parse(Object? json) {
    if (json is! Map || json['account'] is! String) return null;
    final id = json['account'] as String;
    final report = UsageReport.parse({
      'supported': true,
      ...json.cast<String, Object?>(),
    });
    if (report == null) return null;
    final observed = json['observed_at'], updated = json['updated_at'];
    final at = observed is int && observed > 0
        ? observed
        : (updated is int ? updated : 0);
    return _Heard(id, report, at);
  }

  final String id;
  final UsageReport report;
  final int readAt;
}
