import 'dart:async';

import 'package:flutter/foundation.dart';

import '../acp/hub_wire.dart' show HubClient;

/// A session the host keeps running for several clients, as `sessions/running` lists it: any key the entry admits that the host's
/// policy lets attach may take part in it from any device.
final class RunningSession {
  const RunningSession({
    required this.entry,
    required this.sessionId,
    required this.cursor,
    this.title,
    this.cwd,
    this.clients = const [],
    this.lastActivity,
    this.turnRunning = false,
    this.permissionsWaiting = 0,
  });

  /// The entry of canoa.cfg whose agent runs it.
  final String entry;

  /// The agent's own id for it.
  final String sessionId;

  /// The head of the host's log of it: an attach from here is replayed nothing.
  final String cursor;

  /// What the agent last called it, when it has.
  final String? title;

  /// The folder its opener named, which an attach names again.
  final String? cwd;

  /// Who is attached now.
  final List<HubClient> clients;

  /// When anything last happened in it, by the host's clock.
  final DateTime? lastActivity;

  final bool turnRunning;

  /// How many permission requests wait for an answer.
  final int permissionsWaiting;

  /// The attached clients this app is not.
  List<HubClient> get others => [
    for (final client in clients)
      if (!client.isSelf) client,
  ];

  static RunningSession? parse(Object? json) {
    if (json is! Map) return null;
    String? text(String key) => json[key] is String && (json[key] as String).isNotEmpty ? json[key] as String : null;
    final entry = text('entry'), session = text('session');
    if (entry == null || session == null) return null;
    final at = json['lastActivity'];
    return RunningSession(
      entry: entry,
      sessionId: session,
      cursor: text('cursor') ?? '0',
      title: text('title'),
      cwd: text('cwd'),
      clients: [
        if (json['clients'] case final List<Object?> listed)
          for (final client in listed)
            if (client is Map && client['id'] is String)
              HubClient(
                id: client['id'] as String,
                label: client['label'] is String ? client['label'] as String : client['id'] as String,
                role: client['role'] is String ? client['role'] as String : 'client',
              ),
      ],
      lastActivity: at is int ? DateTime.fromMillisecondsSinceEpoch(at, isUtc: true) : null,
      turnRunning: json['turnRunning'] == true,
      permissionsWaiting: json['permissionsWaiting'] is int ? json['permissionsWaiting'] as int : 0,
    );
  }
}

/// The shared sessions running on the host this app is connected to, asked for again when something may have changed.
final class RunningSessions extends ChangeNotifier {
  RunningSessions(this._fetch);

  final Future<List<RunningSession>> Function() _fetch;

  List<RunningSession> _sessions = const [];
  Timer? _every;
  bool _refreshing = false;
  bool _disposed = false;

  /// What the host last listed, newest activity first; empty when it lists none or could not be asked.
  List<RunningSession> get sessions => _sessions;

  /// Asks the host now. A host that cannot be asked, or does not list sessions to this key, shows none.
  Future<void> refresh() async {
    if (_refreshing || _disposed) return;
    _refreshing = true;
    try {
      final listed = await _fetch();
      if (_disposed) return;
      if (!_same(_sessions, listed)) {
        _sessions = listed;
        notifyListeners();
      }
    } on Object {
      if (!_disposed && _sessions.isNotEmpty) {
        _sessions = const [];
        notifyListeners();
      }
    } finally {
      _refreshing = false;
    }
  }

  /// Asks again every [period] until [dispose].
  void keepFresh({Duration period = const Duration(seconds: 15)}) {
    _every?.cancel();
    _every = Timer.periodic(period, (_) => unawaited(refresh()));
    unawaited(refresh());
  }

  static bool _same(List<RunningSession> a, List<RunningSession> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      final x = a[i], y = b[i];
      if (x.entry != y.entry ||
          x.sessionId != y.sessionId ||
          x.title != y.title ||
          x.cursor != y.cursor ||
          x.turnRunning != y.turnRunning ||
          x.permissionsWaiting != y.permissionsWaiting ||
          !listEquals(x.clients, y.clients)) {
        return false;
      }
    }
    return true;
  }

  @override
  void dispose() {
    _disposed = true;
    _every?.cancel();
    super.dispose();
  }
}
