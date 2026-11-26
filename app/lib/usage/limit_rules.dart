import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../settings/advanced_config.dart';

/// Where a limit stands, as the host reads it.
enum LimitState { ok, warn, stop, unmeasured }

/// Whose use a limit counts.
enum LimitScope { all, account, profile }

/// What a limit counts.
enum LimitKind { bucket, usd, tokens }

/// One limit as `limits/state` lists it: what it counts, how much of it is used and in which state.
final class LimitRow {
  const LimitRow({
    required this.name,
    required this.scope,
    this.scopeName,
    required this.kind,
    this.used,
    this.limit,
    this.permille,
    this.bucket,
    required this.state,
    required this.warnPermille,
    this.stopPermille,
    this.windowStart,
    this.resetsAt,
    this.why,
  });

  factory LimitRow.fromJson(Map<Object?, Object?> raw) {
    final figure = raw['figure'] is Map ? (raw['figure'] as Map).cast<Object?, Object?>() : const <Object?, Object?>{};
    final window = raw['window'] is Map ? (raw['window'] as Map).cast<Object?, Object?>() : const <Object?, Object?>{};
    int? whole(Object? value) => value is num ? value.toInt() : null;
    return LimitRow(
      name: '${raw['name']}',
      scope: switch (raw['scope']) {
        'account' => LimitScope.account,
        'profile' => LimitScope.profile,
        _ => LimitScope.all,
      },
      scopeName: raw['name_of_scope'] as String?,
      kind: switch (figure['kind']) {
        'usd' => LimitKind.usd,
        'tokens' => LimitKind.tokens,
        _ => LimitKind.bucket,
      },
      used: whole(figure['used']),
      limit: whole(figure['limit']),
      permille: whole(figure['permille']),
      bucket: figure['bucket'] as String?,
      state: switch (raw['state']) {
        'warn' => LimitState.warn,
        'stop' => LimitState.stop,
        'unmeasured' => LimitState.unmeasured,
        _ => LimitState.ok,
      },
      warnPermille: whole(raw['warn_permille']) ?? 800,
      stopPermille: whole(raw['stop_permille']),
      windowStart: whole(window['start']),
      resetsAt: whole(window['resets_at']),
      why: raw['why'] as String?,
    );
  }

  final String name;
  final LimitScope scope;
  final String? scopeName;
  final LimitKind kind;

  /// Millionths of a dollar for [LimitKind.usd], tokens for [LimitKind.tokens], else what the bucket counts.
  final int? used;
  final int? limit;
  final int? permille;

  /// The bucket that decided, for a limit that counts one.
  final String? bucket;
  final LimitState state;
  final int warnPermille;
  final int? stopPermille;
  final int? windowStart;
  final int? resetsAt;

  /// Why the host cannot measure it.
  final String? why;
}

/// What the app asks the host with: `limits/state` and `limits/allow`.
typedef LimitsCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// The limits of the host: what each says now (`limits/state`) and the entries of `[host.usage.limits]` the person's own file holds,
/// which are added, changed and removed through the host's configuration store as `config/advanced_set` does for any table.
class LimitsController extends ChangeNotifier {
  LimitsController(this._call, this._config);

  final LimitsCall _call;
  final AdvancedConfig _config;
  bool _disposed = false, _loaded = false, _loading = false;
  List<LimitRow> _rows = const [];
  String? _problem;

  /// The path of the table in canoa.cfg.
  static const path = ['host', 'usage', 'limits'];

  List<LimitRow> get rows => _rows;
  bool get loaded => _loaded;
  bool get loading => _loading;

  /// Why the limits could not be read, in the host's words.
  String? get problem => _problem;

  /// What the person's own file holds of each limit, by name, as the form edits it.
  Map<String, Map<String, Object?>> get own {
    for (final table in _config.tables) {
      if (table.id == path.join('.')) return table.ownEntries;
    }
    return const {};
  }

  /// The names of limits a lower layer gives, which are shown and not edited from here.
  Set<String> get inherited {
    for (final table in _config.tables) {
      if (table.id == path.join('.')) return table.inherited.keys.toSet();
    }
    return const {};
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads the limits and the person's entries again.
  Future<void> refresh() async {
    _loading = true;
    notifyListeners();
    try {
      final answer = await _call('limits/state', const {});
      final raw = answer is Map ? answer['limits'] : null;
      _rows = [if (raw is List) for (final each in raw) if (each is Map) LimitRow.fromJson(each.cast<Object?, Object?>())];
      _problem = null;
      await _config.load();
    } on ControlFailed catch (error) {
      _problem = error.message;
    } finally {
      _loaded = true;
      _loading = false;
      notifyListeners();
    }
  }

  /// Makes the limit [name] hold [entry] in the person's own file, as a new limit or a change. Null when it was written, else the
  /// host's words.
  Future<String?> save(String name, Map<String, Object?> entry) async {
    final refused = await _config.set([...path, name], entry);
    if (refused == null) await refresh();
    return refused;
  }

  /// Takes the limit [name] out of the person's own file. Null when it was, else the host's words.
  Future<String?> remove(String name) async {
    final refused = await _config.set([...path, name], null);
    if (refused == null) await refresh();
    return refused;
  }

  /// Lets one turn through the stop of the limit [name], for [session] or any. Null when the host took it, else its words.
  Future<String?> allow(String name, {String? session}) async {
    try {
      await _call('limits/allow', {'name': name, 'session': ?session});
    } on ControlFailed catch (error) {
      return error.message;
    }
    return null;
  }
}

/// The entry `[host.usage.limits.NAME]` a form's choices make.
Map<String, Object?> limitEntry({
  required LimitScope scope,
  String? scopeName,
  required LimitKind kind,
  required String figure,
  String? window,
  int? warn,
  int? stop,
}) {
  final amount = kind == LimitKind.bucket ? null : num.tryParse(figure.trim());
  return {
    if (scope == LimitScope.account && scopeName != null && scopeName.isNotEmpty) 'account': scopeName,
    if (scope == LimitScope.profile && scopeName != null && scopeName.isNotEmpty) 'profile': scopeName,
    if (kind == LimitKind.bucket) 'bucket': figure.trim(),
    if (kind == LimitKind.usd) 'usd': amount,
    if (kind == LimitKind.tokens) 'tokens': amount?.toInt(),
    if (kind != LimitKind.bucket && window != null) 'window': window,
    'warn_permille': ?warn,
    'stop_permille': ?stop,
  };
}

/// The form values an own entry holds, to edit it.
({LimitScope scope, String scopeName, LimitKind kind, String figure, String window, int? warn, int? stop}) readEntry(Map<String, Object?> held) {
  final scope = held['account'] != null
      ? LimitScope.account
      : held['profile'] != null
      ? LimitScope.profile
      : LimitScope.all;
  final kind = held['usd'] != null
      ? LimitKind.usd
      : held['tokens'] != null
      ? LimitKind.tokens
      : LimitKind.bucket;
  int? whole(Object? value) => value is num ? value.toInt() : null;
  return (
    scope: scope,
    scopeName: '${held['account'] ?? held['profile'] ?? ''}',
    kind: kind,
    figure: '${held['usd'] ?? held['tokens'] ?? held['bucket'] ?? ''}',
    window: '${held['window'] ?? 'week'}',
    warn: whole(held['warn_permille']),
    stop: whole(held['stop_permille']),
  );
}
