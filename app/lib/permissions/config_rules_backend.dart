import 'dart:convert';

import '../desktop/host_control.dart';
import '../ledger/session_ledger.dart' show LedgerBackend;
import 'permission_rules.dart' show LockableBackend;

/// The permission rules as the rules store reads them (one version 1 document of rules), kept where each part belongs: what the
/// person decided is `[permissions.rules.ID]` of canoa.cfg, which the host reads and writes (`rules/read`, `rules/write`), and what
/// changes at every use (how often a rule fired and when it last did) is a document of its own, [use].
///
/// A write that changes no rule's settings (a use) touches only [use], so canoa.cfg is written when the person decides, not when a
/// rule answers a request.
final class ConfigRulesBackend implements LedgerBackend, LockableBackend {
  ConfigRulesBackend(this._control, this._use);

  final HostControl _control;
  final LedgerBackend _use;

  /// The settings part as last read or written, to tell a use from a decision.
  String? _lastSettings;

  @override
  String get location => 'canoa.cfg [permissions.rules] and ${_use.location}';

  @override
  Future<T> exclusively<T>(Future<T> Function() body, {Duration timeout = const Duration(seconds: 2)}) {
    final use = _use;
    return use is LockableBackend ? (use as LockableBackend).exclusively(body, timeout: timeout) : body();
  }

  Future<Map<String, Object?>> _rules() async {
    final answer = await (await _control.ready()).call('rules/read', const {});
    final rules = answer is Map ? answer['rules'] : null;
    return rules is Map ? {for (final MapEntry(:key, :value) in rules.entries) '$key': value} : {};
  }

  Future<Map<String, Object?>> _counters() async {
    final text = await _use.read();
    if (text == null) return {};
    try {
      final json = jsonDecode(text);
      final use = json is Map ? json['use'] : null;
      return use is Map ? {for (final MapEntry(:key, :value) in use.entries) '$key': value} : {};
    } on FormatException {
      // Counters that cannot be read are lost, not the rules.
      return {};
    }
  }

  @override
  Future<String?> read() async {
    final rules = await _rules();
    final counters = await _counters();
    _lastSettings = jsonEncode(rules);
    return jsonEncode({
      'version': 1,
      'rules': [
        for (final MapEntry(:key, :value) in rules.entries)
          if (value is Map)
            {
              ...value.map((name, field) => MapEntry('$name', field)),
              'id': key,
              if (counters[key] case {'fired': final int fired}) 'fired': fired,
              if (counters[key] case {'last_used': final String used}) 'last_used': used,
            },
      ],
    });
  }

  @override
  Future<void> write(String document) async {
    final decoded = jsonDecode(document);
    final list = decoded is Map && decoded['rules'] is List ? decoded['rules'] as List : const [];
    final settings = <String, Object?>{};
    final counters = <String, Object?>{};
    for (final rule in list) {
      if (rule is! Map || rule['id'] is! String) continue;
      final id = rule['id'] as String;
      settings[id] = {
        for (final MapEntry(:key, :value) in rule.entries)
          if (key != 'id' && key != 'fired' && key != 'last_used') '$key': value,
      };
      final fired = rule['fired'];
      final used = rule['last_used'];
      if ((fired is int && fired > 0) || used is String) {
        counters[id] = {'fired': fired is int ? fired : 0, 'last_used': ?(used is String ? used : null)};
      }
    }
    if (_lastSettings != jsonEncode(settings)) {
      await (await _control.ready()).call('rules/write', {'rules': settings});
      _lastSettings = jsonEncode(settings);
    }
    await _use.write(jsonEncode({'version': 1, 'use': counters}));
  }
}
