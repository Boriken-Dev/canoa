import 'dart:async';
import 'dart:convert';

import 'package:crypto/crypto.dart';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../desktop/host_supervisor.dart' show HostSupervisor;
import 'session_ledger.dart';

/// The session list kept by the host, over its control channel: one record for each session, and one for what is not a session
/// (the sources' memory) with the sessions' order. The ledger still reads and writes one document; this turns it into
/// the records the host keeps, and puts only those that changed.
final class HostLedgerBackend implements LedgerBackend {
  HostLedgerBackend(HostSupervisor host, {Duration wait = const Duration(seconds: 20)}) : _control = HostControl(host, wait: wait);

  /// The list kept by a host reached through [control]: a web page keeps its sessions in the host that served it.
  HostLedgerBackend.over(this._control);

  final HostControl _control;

  /// What the host holds, by record id, as last read or written: what a write need not send again.
  final _held = <String, String>{};

  static const _meta = 'meta';

  @override
  String get location => "the host's session list";

  /// The id of the record of [session]: its key, hashed, since a key has line breaks and may be longer than an id.
  static String idOf(Map<Object?, Object?> session) =>
      's/${sha256.convert(utf8.encode('${session['peerId']}\n${session['sessionId']}')).toString().substring(0, 40)}';

  Future<ControlClient> _ready() => _control.ready();

  @override
  Future<String?> read() async {
    final client = await _ready();
    final sessions = <String, Object?>{};
    Map<Object?, Object?>? meta;
    String? after;
    do {
      final page = await client.call('ledger/list', {'limit': 500, 'after': ?after}) as Map<Object?, Object?>;
      for (final each in page['records'] as List<Object?>) {
        final record = each! as Map<Object?, Object?>;
        final id = record['id']! as String;
        _held[id] = jsonEncode(record['record']);
        if (id == _meta) {
          meta = record['record'] as Map<Object?, Object?>;
        } else if (id.startsWith('s/')) {
          sessions[id] = record['record'];
        }
      }
      after = page['next'] as String?;
    } while (after != null);
    if (meta == null && sessions.isEmpty) return null;
    final order = [if (meta?['order'] case final List<Object?> kept) ...kept.whereType<String>()];
    final ordered = [
      for (final id in order)
        ?sessions.remove(id),
      ...sessions.values,
    ];
    return jsonEncode({
      'version': 1,
      'sessions': ordered,
      if (meta?['sources'] case final Map<Object?, Object?> sources when sources.isNotEmpty) 'sources': sources,
    });
  }

  @override
  Future<void> write(String document) async {
    final client = await _ready();
    final decoded = jsonDecode(document) as Map<Object?, Object?>;
    final sessions = [for (final each in decoded['sessions'] as List<Object?>) each! as Map<Object?, Object?>];
    final records = <String, Map<Object?, Object?>>{
      for (final session in sessions) idOf(session): session,
      _meta: {
        'order': [for (final session in sessions) idOf(session)],
        if (decoded['sources'] != null) 'sources': decoded['sources'],
      },
    };
    // The oldest first, so the host's clock keeps their order too.
    for (final MapEntry(:key, :value) in records.entries.toList().reversed) {
      final text = jsonEncode(value);
      if (_held[key] == text) continue;
      await client.call('ledger/put', {'id': key, 'record': value});
      _held[key] = text;
    }
    for (final id in _held.keys.toList()) {
      if (records.containsKey(id)) continue;
      await client.call('ledger/remove', {'id': id});
      _held.remove(id);
    }
  }

  Future<void> close() async => _control.close();
}
