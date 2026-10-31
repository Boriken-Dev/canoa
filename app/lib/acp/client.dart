import 'dart:async';
import 'dart:convert';

import '../transport/agent_transport.dart';

/// Must cryptographically verify the remote peer, not trust a supplied ID.
typedef VerifyAcpPeer = Future<String> Function(
    AgentConnection channel, String expectedPeerId);

class AcpFailure implements Exception {
  const AcpFailure(this.message);
  final String message;
  @override
  String toString() => 'ACP: $message';
}

final class AcpSessionUpdate {
  AcpSessionUpdate(this.sessionId, this.raw);
  final String sessionId;
  final Map<String, dynamic> raw;
  String get kind => raw['sessionUpdate'] is String ? raw['sessionUpdate'] as String : '';
  String? get messageId => raw['messageId'] is String ? raw['messageId'] as String : null;
  String? get state => raw['state'] is String ? raw['state'] as String : null;
  String? get reason => raw['stopReason'] is String ? raw['stopReason'] as String : null;
  String? get text {
    final value = raw['content'];
    if (value is Map && value['type'] == 'text' && value['text'] is String) {
      return value['text'] as String;
    }
    if (value is List) {
      return value.whereType<Map>().where((entry) => entry['type'] == 'text')
          .map((entry) => entry['text']).whereType<String>().join();
    }
    return null;
  }
}

/// JSON-RPC ACP v2 controller. It is not a network or authentication provider.
/// It cannot be constructed until a trusted integration verifies the channel.
final class AcpV2Client {
  AcpV2Client._(this._channel, this.peerId) {
    _subscription = _channel.messages.listen(_receive,
      onError: _fail,
      onDone: () => _fail(const AcpFailure('Connection lost; pending outcomes unknown')),
    );
  }

  final AgentConnection _channel;
  final String peerId;
  final Map<int, Completer<Map<String, dynamic>>> _pending = {};
  final _events = StreamController<AcpSessionUpdate>.broadcast(sync: true);
  late final StreamSubscription<String> _subscription;
  int _counter = 0;
  bool _closed = false;
  bool _initialized = false;

  Stream<AcpSessionUpdate> get updates => _events.stream;
  bool get isClosed => _closed;

  static Future<AcpV2Client> open(
    AgentConnection channel, {
    required String expectedPeerId,
    required VerifyAcpPeer verifyPeer,
    Duration verificationTimeout = const Duration(seconds: 10),
  }) async {
    if (expectedPeerId.isEmpty) throw ArgumentError.value(expectedPeerId, 'expectedPeerId');
    try {
      final actual =
          await verifyPeer(channel, expectedPeerId).timeout(verificationTimeout);
      if (actual != expectedPeerId) {
        throw const AcpFailure('Verified peer does not match expected identity');
      }
      return AcpV2Client._(channel, actual);
    } catch (_) {
      await channel.close();
      rethrow;
    }
  }

  void _fail(Object error, [StackTrace? trace]) {
    if (_closed) return;
    _closed = true;
    for (final completer in _pending.values) {
      if (!completer.isCompleted) completer.completeError(error, trace);
    }
    _pending.clear();
    unawaited(_events.close());
  }

  void _receive(String message) {
    if (_closed) return;
    try {
      final envelope = jsonDecode(message);
      if (envelope is! Map<String, dynamic> || envelope['jsonrpc'] != '2.0') {
        throw const FormatException('Not JSON-RPC 2.0');
      }
      final method = envelope['method'];
      final id = envelope['id'];
      if (method is String) {
        if (id != null) {
          // Deny unsolicited privileged client methods, including permissions.
          final result = method == 'session/request_permission'
              ? <String, Object?>{'result': {'outcome': {'outcome': 'cancelled'}}}
              : <String, Object?>{'error': {'code': -32601, 'message': 'Unsupported method'}};
          unawaited(_channel.send(jsonEncode({
            'jsonrpc': '2.0', 'id': id, ...result,
          })).catchError((Object _) {}));
        } else if (method == 'session/update') {
          final params = envelope['params'];
          if (params is Map && params['sessionId'] is String && params['update'] is Map) {
            _events.add(AcpSessionUpdate(params['sessionId'] as String,
                Map<String, dynamic>.from(params['update'] as Map)));
          }
        }
        return;
      }
      if (id is! int) return;
      final completer = _pending.remove(id);
      if (completer == null || completer.isCompleted) return;
      if (envelope['error'] is Map) {
        completer.completeError(AcpFailure('Agent error: ' + (envelope['error'] as Map).toString()));
      } else if (envelope['result'] is Map) {
        completer.complete(Map<String, dynamic>.from(envelope['result'] as Map));
      } else {
        completer.completeError(const AcpFailure('Malformed agent response'));
      }
    } on FormatException catch (error) {
      _fail(AcpFailure('Invalid agent message: $error'));
      unawaited(_channel.close());
    } on TypeError catch (error) {
      _fail(AcpFailure('Invalid message fields: $error'));
      unawaited(_channel.close());
    }
  }

  Future<Map<String, dynamic>> _request(String method, Map<String, Object?> params,
      {Duration timeout = const Duration(seconds: 20)}) async {
    if (_closed) throw const AcpFailure('Connection closed');
    final id = ++_counter;
    final pending = Completer<Map<String, dynamic>>();
    _pending[id] = pending;
    try {
      await _channel.send(jsonEncode({
        'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params,
      })).timeout(timeout);
      return await pending.future.timeout(timeout);
    } finally {
      _pending.remove(id);
    }
  }

  Future<void> initialize() async {
    if (_initialized) return;
    final reply = await _request('initialize', {
      'protocolVersion': 2,
      'capabilities': <String, Object?>{},
      'info': {'name': 'canoa', 'title': 'Canoa', 'version': '0.0.1'},
    });
    if (reply['protocolVersion'] != 2 || reply['capabilities'] is! Map ||
        (reply['capabilities'] as Map)['session'] is! Map) {
      throw const AcpFailure('Agent does not support ACP v2 sessions');
    }
    _initialized = true;
  }

  void _ready() {
    if (!_initialized) throw StateError('Initialize ACP before session operations');
  }

  void _checkCwd(String cwd) {
    if (!(cwd.startsWith('/') || RegExp(r'^[a-zA-Z]:[\\/]').hasMatch(cwd))) {
      throw ArgumentError.value(cwd, 'cwd', 'Must be an absolute path');
    }
  }

  Future<String> createSession({required String cwd}) async {
    _ready();
    _checkCwd(cwd);
    final reply = await _request('session/new', {'cwd': cwd, 'mcpServers': <Object>[]});
    final id = reply['sessionId'];
    if (id is! String || id.isEmpty) throw const AcpFailure('Missing session ID');
    return id;
  }

  Future<void> resumeSession(String id, {required String cwd, bool replayHistory = false}) async {
    _ready();
    _checkCwd(cwd);
    await _request('session/resume', {
      'sessionId': id, 'cwd': cwd, 'mcpServers': <Object>[],
      if (replayHistory) 'replayFrom': {'type': 'start'},
    });
  }

  /// Acknowledges insertion only. Completion arrives via state_update.
  Future<String> prompt(String sessionId, String text) async {
    _ready();
    if (text.trim().isEmpty) throw ArgumentError.value(text, 'text');
    final reply = await _request('session/prompt', {
      'sessionId': sessionId, 'prompt': [{'type': 'text', 'text': text}],
    });
    final id = reply['messageId'];
    if (id is! String || id.isEmpty) throw const AcpFailure('Missing inserted message ID');
    return id;
  }

  Future<void> cancel(String sessionId) async {
    _ready();
    if (_closed) throw const AcpFailure('Connection closed');
    await _channel.send(jsonEncode({
      'jsonrpc': '2.0', 'method': 'session/cancel', 'params': {'sessionId': sessionId},
    }));
  }

  Future<void> closeSession(String sessionId) async {
    _ready();
    await _request('session/close', {'sessionId': sessionId});
  }

  Future<void> close() async {
    _fail(const AcpFailure('ACP client closed'));
    await _subscription.cancel();
    await _channel.close();
  }
}
