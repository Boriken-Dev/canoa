// The page's peer bridge as crates/canoa-host's web_peer example, for tests in the Dart VM, which has no WebCrypto.
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/auth/peer_proof.dart';

import 'examples.dart';

/// The page's peer bridge in a web_peer process: one key of each method for the life of the process.
final class WebPeerBridge implements PeerBridge {
  WebPeerBridge._(this._process) {
    _process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final answer = jsonDecode(line) as Map<String, dynamic>;
      final waiting = _waiting.remove(answer['id']);
      if (waiting == null) return;
      if (answer.containsKey('error')) {
        waiting.completeError(StateError(answer['error'] as String));
      } else {
        waiting.complete(answer['result']);
      }
    });
    _process.stderr.transform(utf8.decoder).listen(stderr.write);
    unawaited(_process.stdin.done.then<void>((_) {}, onError: (Object _) {}));
  }

  /// Starts the built web_peer example; see [exampleBinary].
  static Future<WebPeerBridge> start() async {
    final program = exampleBinary('web_peer');
    if (program == null) throw StateError(exampleMissing('web_peer') ?? 'web_peer is not built');
    return WebPeerBridge._(await Process.start(program, const []));
  }

  final Process _process;
  final _waiting = <int, Completer<Object?>>{};
  var _next = 0;

  Future<Object?> _call(String op, List<Object?> args) {
    final id = _next++;
    final answer = Completer<Object?>();
    _waiting[id] = answer;
    try {
      _process.stdin.writeln(jsonEncode({'id': id, 'op': op, 'args': args}));
    } on Object catch (error) {
      _waiting.remove(id);
      answer.completeError(StateError('web_peer has gone: $error'));
    }
    return answer.future;
  }

  /// A call nobody waits for: a process already ended is no failure of the test.
  void _forget(Future<Object?> call) => unawaited(call.then<void>((_) {}, onError: (Object _) {}));

  Uint8List _bytes(Object? text) => base64Decode(text as String);

  @override
  Future<String?> unreachable(PeerMethod method) async => await _call('unreachable', [method.id]) as String?;
  @override
  Future<String> localId(PeerMethod method) async => await _call('localId', [method.id]) as String;
  @override
  Future<String> begin(PeerMethod method, String expectedPeerId) async =>
      await _call('begin', [method.id, expectedPeerId]) as String;
  @override
  Future<String> pair(String pin) async => await _call('pair', [pin]) as String;
  @override
  Future<PeerStep> step(String token, Uint8List? received) async {
    final step = await _call('step', [token, received == null ? null : base64Encode(received)]) as Map<String, dynamic>;
    return PeerStep(
      send: [for (final message in step['send'] as List<dynamic>) _bytes(message)],
      peerId: step['peerId'] as String?,
      session: step['session'] as String?,
    );
  }

  @override
  void abort(String token) => _forget(_call('abort', [token]));
  @override
  Future<Uint8List> seal(String session, Uint8List message) async =>
      _bytes(await _call('seal', [session, base64Encode(message)]));
  @override
  Future<Uint8List?> open(String session, Uint8List record) async {
    final opened = await _call('open', [session, base64Encode(record)]);
    return opened == null ? null : _bytes(opened);
  }

  @override
  Future<Uint8List> closeRecord(String session) async => _bytes(await _call('closeRecord', [session]));
  @override
  void closeSession(String session) => _forget(_call('closeSession', [session]));

  void kill() => _process.kill();
}
