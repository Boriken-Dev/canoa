import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

/// Every application message inside the session a peer proof ended in. A plain message comes from whoever carries the
/// connection, so anything that does not open ends the channel.
final class SealedChannel implements WebChannel {
  SealedChannel(
    this._source, {
    required Future<Uint8List> Function(Uint8List) seal,
    required Future<Uint8List> Function(Uint8List) open,
    required void Function() release,
  }) : _seal = seal,
       _open = open,
       _release = release;

  final ByteWebChannel _source;
  final Future<Uint8List> Function(Uint8List) _seal;
  final Future<Uint8List> Function(Uint8List) _open;
  final void Function() _release;
  Future<void> _sending = Future.value();
  bool _closed = false;

  @override
  WebChannelKind get kind => _source.kind;

  @override
  Stream<String> get messages => _source.bytes.asyncMap((record) async {
    try {
      return utf8.decode(await _open(record));
    } catch (_) {
      unawaited(close());
      throw const TransportProtocolFailure('A message was not from the proven peer');
    }
  });

  /// Sequence numbers go in call order, and records must reach the wire in that order.
  @override
  Future<void> send(String message) {
    if (_closed) return Future.error(const TransportUnavailable('The session is closed'));
    final sealed = _seal(utf8.encode(message));
    final sent = _sending.then((_) async => _source.sendBytes(await sealed));
    _sending = sent.catchError((Object _) {});
    return sent;
  }

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    _release();
    await _source.close();
  }
}
