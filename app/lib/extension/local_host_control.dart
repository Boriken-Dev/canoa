// The Canoa on this computer's control channel, reached through the extension's service worker: the worker proves the host's key
// and the page never holds a link of its own. The session list the panel keeps is the host's.
import 'dart:async';
import 'dart:convert';

import '../desktop/host_control.dart';
import '../desktop/host_tool.dart';
import 'local_host.dart';

/// A [HostLink] that answers each control request by asking the worker to run it: the control channel's JSON-RPC lines in,
/// its answers out. A refusal of the host is its error object; one of the worker is an error with `-32000`.
final class BridgeControlLink implements HostLink {
  BridgeControlLink(this._bridge);

  final LocalHostBridge _bridge;
  final _messages = StreamController<String>();
  bool _closed = false;

  @override
  Stream<String> get messages => _messages.stream;

  @override
  Future<void> send(String message) async {
    if (_closed) throw StateError('the link to the host has ended');
    final request = jsonDecode(message) as Map<String, Object?>;
    final id = request['id'];
    unawaited(() async {
      Map<String, Object?> answer;
      try {
        final result = await _bridge.control('${request['method']}', (request['params'] as Map?)?.cast<String, Object?>() ?? const {});
        answer = {'jsonrpc': '2.0', 'id': id, 'result': result};
      } on Object catch (error) {
        answer = {
          'jsonrpc': '2.0',
          'id': id,
          'error': {'code': -32000, 'message': '$error'.replaceFirst('Exception: ', '')},
        };
      }
      if (!_closed) _messages.add(jsonEncode(answer));
    }());
  }

  @override
  Future<void> close() async {
    _closed = true;
    await _messages.close();
  }
}

/// The control channel of the Canoa on this computer, as the panel uses any host's.
HostControl localHostControl(LocalHostBridge bridge) => HostControl.dialing(() async => BridgeControlLink(bridge));
