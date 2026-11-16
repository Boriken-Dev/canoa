import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageReport;
import 'package:flutter/foundation.dart';

import '../accounts/account_models.dart';
import '../accounts/accounts_controller.dart';
import '../desktop/control_client.dart' show ControlFailed;
import '../desktop/host_control.dart';
import 'spend_model.dart';

/// A host that answers `usage/limits`: its name as the person knows it, and how to ask it.
abstract interface class UsageSource {
  String get name;

  /// The `accounts` array of the host's `usage/limits` answer.
  Future<List<Object?>> limits();

  /// The host's `usage/summary` answer for [params]: from, to, group_by and the filters.
  Future<Object?> summary(Map<String, Object?> params);
}

/// A host that can read an account's allowances from its provider now, with no session: what [UsageController] asks of each
/// account it lists, so an account no session has run on shows what the provider can say.
abstract interface class UsageReader {
  /// The row `usage/limits` would list for the account called [name], or null when the host has none.
  Future<Object?> read(String name);
}

/// The host this app's control link reaches.
final class ControlUsageSource implements UsageSource, UsageReader {
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

  @override
  Future<Object?> summary(Map<String, Object?> params) async =>
      (await _control.ready(wait: false)).call('usage/summary', params);

  @override
  Future<Object?> read(String name) async {
    final result = await (await _control.ready(wait: false)).call('usage/read', {'account': name});
    return result is Map ? result['account'] : null;
  }
}

/// The provider an account is listed under when nothing says which it is.
const otherProvider = 'Other';

/// Why an account shows no figures after its provider was asked with no session.
enum ReadNoteKind {
  /// The provider has no limits to report for this kind of account, and says why.
  notOffered,

  /// The provider answered and has no figures yet.
  notYet,

  /// The question itself failed.
  failed,
}

/// What asking an account's provider came to when it gave no figures: which of the three, and the words for it.
final class ReadNote {
  const ReadNote(this.kind, this.text);

  final ReadNoteKind kind;
  final String text;
}

/// What is known of one account's allowances: the newest reading among the hosts that reported its identity.
final class AccountLimits {
  const AccountLimits({
    required this.id,
    required this.name,
    required this.provider,
    required this.hosts,
    required this.report,
    required this.readAt,
    this.note,
  });

  /// Why there are no figures, when a host was asked and gave none; null when figures show or no host was asked.
  final ReadNote? note;

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
    _model = _session = null;
    if (account != null) {
      final found = all.where((each) => each.id == account).firstOrNull;
      if (found != null) _provider = found.provider;
    }
    _changed();
  }

  void selectHost(String? host) {
    _host = host;
    _clearMissing();
    _changed();
  }

  void selectProvider(String? provider) {
    _provider = provider;
    _clearMissing();
    _changed();
  }

  void selectAccount(String? account) {
    _account = account;
    _changed();
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
      _notify();
      await _readEach();
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

  /// What each host's provider said when asked of an account with no session, by the host's name and the account's identity.
  final _read = <String, Map<String, _Heard>>{};

  /// Why a host's provider gave no figures for an account, by the host's name and the account's identity.
  final _notes = <String, Map<String, ReadNote>>{};

  /// Asks each host that can for every account this computer lists, so an account no session has run on still says what its
  /// provider can report. A provider that reports only during a turn answers nothing, and an answer that fails leaves the account
  /// as it was.
  Future<void> _readEach() async {
    final listed = accounts?.accounts ?? const [];
    await Future.wait([
      for (final source in sources)
        if (source is UsageReader)
          for (final account in listed)
            () async {
              try {
                final row = await (source as UsageReader).read(account.name);
                final heard = _Heard.parse(row is Map ? {'account': account.id, ...row.cast<String, Object?>()} : null);
                if (heard == null) return;
                (_read[source.name] ??= {})[account.id] = heard;
                final note = heard.report.buckets.isNotEmpty
                    ? null
                    : heard.report.supported
                    ? const ReadNote(ReadNoteKind.notYet, 'The provider answered and has no figures for this account yet.')
                    : ReadNote(ReadNoteKind.notOffered, heard.report.unsupportedBecause ?? 'This provider reports no limits for this account.');
                final notes = _notes[source.name] ??= {};
                note == null ? notes.remove(account.id) : notes[account.id] = note;
              } on Object catch (error) {
                // The question failed: said in words, and what the sessions reported stands.
                (_notes[source.name] ??= {})[account.id] = ReadNote(ReadNoteKind.failed, _why(error));
              }
            }(),
    ]);
  }

  static String _why(Object error) => error is ControlFailed ? error.message : '$error'.replaceFirst(RegExp(r'^(Exception|StateError): '), '');

  /// Every account any host reported, and every account this computer lists that none did, one for each identity, the newest
  /// reading winning, in a stable order: provider, then name.
  List<AccountLimits> get all {
    final byId = <String, List<(String, _Heard)>>{};
    for (final source in sources) {
      for (final heard in _heard[source.name] ?? const <_Heard>[]) {
        byId.putIfAbsent(heard.id, () => []).add((source.name, heard));
      }
      for (final heard in (_read[source.name] ?? const <String, _Heard>{}).values) {
        // A reading with figures stands beside what sessions heard; one with none says nothing the account's row lacks.
        if (heard.report.buckets.isNotEmpty) byId.putIfAbsent(heard.id, () => []).add((source.name, heard));
      }
    }
    final merged = <AccountLimits>[];
    for (final each in accounts?.accounts ?? const <AccountInfo>[]) {
      if (byId.containsKey(each.id)) continue;
      ReadNote? note;
      for (final source in sources) {
        note ??= _notes[source.name]?[each.id];
      }
      merged.add(AccountLimits(id: each.id, name: each.name, provider: each.harness, hosts: const [], report: UsageReport.parse({'supported': true})!, readAt: 0, note: note));
    }
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
          hosts: {for (final (host, _) in value) host}.toList(),
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

  // ---- Tokens and Cost: sums the hosts keep, asked for by range and filter; no record is loaded here.

  UsageView _view = UsageView.limits;
  SpendRange _range = SpendRange.week;
  Breakdown _breakdown = Breakdown.account;
  int? _customFrom, _customTo;
  String? _model, _session;
  int _spendRun = 0;
  bool _spendLoading = false;
  SpendSummary _byDay = const SpendSummary(), _byAccount = const SpendSummary(), _byOther = const SpendSummary();
  final _perHost = <String, SpendRow>{};
  final _notCovered = <String, String>{};

  UsageView get view => _view;
  SpendRange get range => _range;
  Breakdown get breakdown => _breakdown;
  String? get model => _model;
  String? get session => _session;
  bool get spendLoading => _spendLoading;

  /// Everything the range holds, summed over the hosts that answered.
  SpendRow get total => _byDay.total;

  /// The zone days are cut in, as the hosts say it.
  String get timezone => _byDay.timezone;

  SpendSummary get byDay => _byDay;
  SpendSummary get byAccount => _byAccount;

  /// The hosts that answered nothing, by name, and why.
  Map<String, String> get notCovered => Map.unmodifiable(_notCovered);

  /// The groups of the chosen breakdown.
  List<SpendRow> get rows => switch (_breakdown) {
    Breakdown.account => _byAccount.groups,
    Breakdown.day => _byDay.groups,
    Breakdown.host => [for (final MapEntry(:key, :value) in _perHost.entries) value.plus(const SpendRow(), key: key)],
    _ => _byOther.groups,
  };

  /// The range as Unix seconds, `from <= time < to`: rolling for the presets, whole days in the hosts' zone for a custom one.
  (int, int) get span {
    final now = clock;
    if (_range == SpendRange.custom && _customFrom != null && _customTo != null) return (_customFrom!, _customTo!);
    final days = _range == SpendRange.custom ? SpendRange.week.days : _range.days;
    return (now - days * 86400, now + 1);
  }

  /// Chooses what the Limits, Tokens and Cost views show.
  void setView(UsageView view) {
    _view = view;
    _changed();
  }

  void setRange(SpendRange range) {
    _range = range;
    _changed();
  }

  /// A custom range of whole days: [from] to [lastDay], both inclusive, as Unix seconds of each day's start.
  void setCustom(int from, int lastDay) {
    _range = SpendRange.custom;
    _customFrom = from;
    _customTo = lastDay + 86400;
    _changed();
  }

  void setBreakdown(Breakdown breakdown) {
    _breakdown = breakdown;
    _changed();
  }

  void selectModel(String? model) {
    _model = model;
    _changed();
  }

  void selectSession(String? session) {
    _session = session;
    _changed();
  }

  /// An account's name as the person knows it; Unknown when no host or account list names it.
  String accountName(String? key) {
    if (key == null || key.isEmpty) return 'Unknown';
    final known = all.where((each) => each.id == key).firstOrNull?.name ?? accounts?.accounts.where((each) => each.id == key).firstOrNull?.name;
    return known ?? 'Unknown';
  }

  /// What every host recorded the session with this id spending, over all time; null when no host answered.
  Future<SpendRow?> sessionSpend(String id) async {
    final rows = <SpendRow>[];
    for (final source in sources) {
      try {
        final answer = SpendSummary.parse(await source.summary({'session': id, 'group_by': 'day'}));
        if (answer != null) rows.add(answer.total);
      } on Object {
        continue;
      }
    }
    return rows.isEmpty ? null : rows.reduce((a, b) => a.plus(b));
  }

  void _changed() {
    _notify();
    if (_view != UsageView.limits) unawaited(refreshSpend());
  }

  /// Asks the hosts for the sums the views show: by day, by account, and by the chosen breakdown when it is another.
  Future<void> refreshSpend() async {
    final run = ++_spendRun;
    _spendLoading = true;
    _notify();
    final (from, to) = span;
    final base = <String, Object?>{
      'from': from,
      'to': to,
      'account': ?_account,
      'provider': ?_provider,
      'model': ?_model,
      'session': ?_session,
    };
    final other = switch (_breakdown) {
      Breakdown.provider => 'provider',
      Breakdown.model => 'model',
      Breakdown.session => 'session',
      _ => null,
    };
    final asked = [for (final source in sources) if (_host == null || source.name == _host) source];
    final answers = <String, (SpendSummary, SpendSummary, SpendSummary?)>{};
    final failed = <String, String>{};
    Future<SpendSummary> ask(UsageSource source, String groupBy) async {
      final answer = SpendSummary.parse(await source.summary({...base, 'group_by': groupBy}));
      if (answer == null) throw const FormatException('the answer was not a summary');
      return answer;
    }

    await Future.wait([
      for (final source in asked)
        () async {
          try {
            final day = await ask(source, 'day');
            final account = await ask(source, 'account');
            final extra = other == null ? null : await ask(source, other);
            answers[source.name] = (day, account, extra);
          } on Object catch (error) {
            failed[source.name] = '$error';
          }
        }(),
    ]);
    if (run != _spendRun || _disposed) return;
    final ok = answers.values.toList();
    _byDay = SpendSummary.merge([for (final each in ok) each.$1]);
    _byAccount = SpendSummary.merge([for (final each in ok) each.$2]);
    _byOther = SpendSummary.merge([for (final each in ok) ?each.$3]);
    _perHost
      ..clear()
      ..addEntries([for (final MapEntry(:key, :value) in answers.entries) MapEntry(key, value.$1.total)]);
    _notCovered
      ..clear()
      ..addAll(failed);
    _spendLoading = false;
    _notify();
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
