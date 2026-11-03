// The page's peer bridge, web/canoa_peer_bridge.mjs, run by Node for tests in the Dart VM, which has no WebCrypto.
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/auth/peer_proof.dart';

/// Node 22 or later: CANOA_TEST_NODE, else `node`.
final String nodeProgram = Platform.environment['CANOA_TEST_NODE'] ?? 'node';

/// The page's peer bridge in a Node process: one key for the life of the process.
final class NodePeerBridge implements PeerBridge {
  NodePeerBridge._(this._process) {
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
  }

  /// [script] is node_peer.mjs, by default as seen from the app's directory.
  static Future<NodePeerBridge> start({String script = 'test/fixtures/node_peer.mjs'}) async =>
      NodePeerBridge._(await Process.start(nodeProgram, [script]));

  final Process _process;
  final _waiting = <int, Completer<Object?>>{};
  var _next = 0;

  Future<Object?> _call(String op, List<Object?> args) {
    final id = _next++;
    final answer = Completer<Object?>();
    _waiting[id] = answer;
    _process.stdin.writeln(jsonEncode({'id': id, 'op': op, 'args': args}));
    return answer.future;
  }

  Uint8List _bytes(Object? text) => base64Decode(text as String);

  Future<String> localId() async => await _call('localId', []) as String;
  @override
  Future<String> begin(String expectedPeerId) async => await _call('begin', [expectedPeerId]) as String;
  @override
  Future<String> answer(String token, String challenge) async => await _call('answer', [token, challenge]) as String;
  @override
  Future<String> finish(String token, String accepted) async => await _call('finish', [token, accepted]) as String;
  @override
  void abort(String token) => unawaited(_call('abort', [token]));
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
  void closeSession(String session) => unawaited(_call('closeSession', [session]));

  void kill() => _process.kill();
}
