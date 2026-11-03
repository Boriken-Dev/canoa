import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

/// Every application message inside the session a peer proof ended in. A plain message comes from whoever carries the
/// connection, so anything that does not open ends the channel, and so does a connection cut before the peer's close record,
/// which is how a relay would hide what it held back.
final class SealedChannel implements WebChannel {
  SealedChannel(
    this._source, {
    required Future<Uint8List> Function(Uint8List) seal,
    required Future<Uint8List?> Function(Uint8List) open,
    required Future<Uint8List> Function() closeRecord,
    required void Function() release,
  }) : _seal = seal,
       _open = open,
       _closeRecord = closeRecord,
       _release = release;

  final ByteWebChannel _source;
  final Future<Uint8List> Function(Uint8List) _seal;
  final Future<Uint8List?> Function(Uint8List) _open;
  final Future<Uint8List> Function() _closeRecord;
  final void Function() _release;
  Future<void> _sending = Future.value();
  bool _closed = false;

  @override
  WebChannelKind get kind => _source.kind;

  @override
  Stream<String> get messages async* {
    try {
      await for (final record in _source.bytes) {
        final Uint8List? opened;
        try {
          opened = await _open(record);
        } catch (_) {
          unawaited(close());
          throw const TransportProtocolFailure('A message was not from the proven peer');
        }
        // The peer's close: an orderly end, with nothing held back.
        if (opened == null) return;
        yield utf8.decode(opened);
      }
    } catch (_) {
      await _settled();
      rethrow;
    }
    if (!_closed) {
      unawaited(close());
      await _settled();
      throw const TransportUnavailable('The connection was cut before the peer closed it');
    }
  }

  /// Lets sends already handed to this channel settle, and their callers resume, before the link's end is reported: a caller
  /// still sending a request has not yet begun to wait for its answer, and an end reported first would reach no one. Bounded,
  /// so a seal that never returns cannot hold the end back.
  Future<void> _settled() async {
    await _sending.timeout(const Duration(seconds: 5), onTimeout: () {});
    await Future<void>.delayed(Duration.zero);
  }

  /// Sequence numbers go in call order, and records must reach the wire in that order.
  @override
  Future<void> send(String message) {
    if (_closed) return Future.error(const TransportUnavailable('The session is closed'));
    return _enqueue(_seal(utf8.encode(message)));
  }

  Future<void> _enqueue(Future<Uint8List> sealed) {
    final sent = _sending.then((_) async => _source.sendBytes(await sealed));
    _sending = sent.catchError((Object _) {});
    return sent;
  }

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    try {
      await _enqueue(_closeRecord());
    } catch (_) {
      // The connection may be gone already, which the peer sees as a cut.
    }
    _release();
    await _source.close();
  }
}
