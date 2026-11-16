import 'dart:async';

import '../desktop/host_control.dart';
import 'permission_rules.dart' show PathOracle;

/// Follows links and finds repositories on the computer the host runs on (`fs/realpath`, `fs/repo_root`, `fs/guards`), for the
/// remembered rules, which put their questions where nothing can wait. A question is answered from what the host last said;
/// [settle] finds out what a set of questions needs, asks the host for it, and has them asked again, so the answers are the
/// host's of that moment: a link changed since is followed afresh for every request.
final class HostPaths implements PathOracle {
  HostPaths(this._control);

  final HostControl _control;
  final Map<String, ({String? value, int round})> _real = {};
  final Map<String, ({String? value, int round})> _roots = {};
  final Set<String> _missedReal = {};
  final Set<String> _missedRoots = {};
  ({String? home, String? config})? _guards;
  int _round = 0;

  /// The round a probe in progress wants its answers from; none outside one.
  int? _probing;

  /// The most answers kept before the oldest are let go.
  static const _most = 4096;

  /// The most times a probe is run: asking what a rule's folder resolves to is one step, where its repository starts another.
  static const _passes = 6;

  /// The most paths one request names.
  static const _batch = 64;

  @override
  String? real(String path) => _lookup(_real, _missedReal, path);

  @override
  String? repoRoot(String folder) => _lookup(_roots, _missedRoots, folder);

  String? _lookup(Map<String, ({String? value, int round})> known, Set<String> missed, String key) {
    final held = known[key];
    final want = _probing;
    if (want != null && (held == null || held.round < want)) missed.add(key);
    return held?.value;
  }

  @override
  Future<({String? home, String? config})?> guards() async {
    if (_guards != null) return _guards;
    try {
      final said = await _call('fs/guards', const {});
      return _guards = (home: said['home'] as String?, config: said['config'] as String?);
    } on Object {
      return null;
    }
  }

  @override
  Future<void> settle(void Function() probe) async {
    final round = ++_round;
    if (_real.length > _most) _real.removeWhere((_, held) => held.round < round);
    if (_roots.length > _most) _roots.removeWhere((_, held) => held.round < round);
    for (var pass = 0; pass < _passes; pass++) {
      _missedReal.clear();
      _missedRoots.clear();
      _probing = round;
      try {
        probe();
      } finally {
        _probing = null;
      }
      if (_missedReal.isEmpty && _missedRoots.isEmpty) return;
      await Future.wait([
        _fill('fs/realpath', 'paths', 'real', _missedReal.toList(), _real, round),
        _fill('fs/repo_root', 'folders', 'root', _missedRoots.toList(), _roots, round),
      ]);
    }
  }

  /// Asks [method] for [keys] and keeps what it said; a path it could not say anything of, or a host that could not be asked, is
  /// kept as nothing, so no rule is applied on a guess.
  Future<void> _fill(String method, String list, String field, List<String> keys, Map<String, ({String? value, int round})> into, int round) async {
    for (var from = 0; from < keys.length; from += _batch) {
      final part = keys.sublist(from, from + _batch > keys.length ? keys.length : from + _batch);
      final said = <String, String?>{};
      try {
        final answer = await _call(method, {'paths': part});
        for (final each in (answer[list] as List<Object?>? ?? const [])) {
          if (each is Map<Object?, Object?> && each['path'] is String) said[each['path']! as String] = each[field] as String?;
        }
      } on Object {
        // Nothing said: each path below is kept as nothing.
      }
      for (final key in part) {
        into[key] = (value: said[key], round: round);
      }
    }
  }

  Future<Map<Object?, Object?>> _call(String method, Map<String, Object?> params) async {
    final client = await _control.ready();
    final result = await client.call(method, params, null, const Duration(seconds: 15));
    return result is Map<Object?, Object?> ? result : const {};
  }
}
