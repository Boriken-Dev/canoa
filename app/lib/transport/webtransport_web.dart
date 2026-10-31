import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'channel.dart';

@JS('WebTransport')
extension type _WebTransport._(JSObject _) implements JSObject {
  external factory _WebTransport(String url, JSObject options);
  external JSPromise<JSAny?> get ready;
  external JSPromise<JSAny?> get closed;
  external JSPromise<_BidirectionalStream> createBidirectionalStream();
  external void close();
}

extension type _BidirectionalStream._(JSObject _) implements JSObject {
  external _ReadableStream get readable;
  external _WritableStream get writable;
}

extension type _ReadableStream._(JSObject _) implements JSObject {
  external _Reader getReader();
}

extension type _Reader._(JSObject _) implements JSObject {
  external JSPromise<_ReadResult> read();
  external JSPromise<JSAny?> cancel();
}

extension type _ReadResult._(JSObject _) implements JSObject {
  external bool get done;
  external JSUint8Array? get value;
}

extension type _WritableStream._(JSObject _) implements JSObject {
  external _Writer getWriter();
}

extension type _Writer._(JSObject _) implements JSObject {
  external JSPromise<JSAny?> write(JSUint8Array chunk);
  external JSPromise<JSAny?> close();
}

extension type _Hash._(JSObject _) implements JSObject {
  external factory _Hash({String algorithm, JSUint8Array value});
}

extension type _Options._(JSObject _) implements JSObject {
  external factory _Options({JSArray<_Hash> serverCertificateHashes});
}

@JS('globalThis.WebTransport')
external JSAny? get _webTransportClass;

/// Bounded message size for Canoa's ACP WebTransport profile, 4 MiB.
const int maxAcpMessageBytes = 4 * 1024 * 1024;

Uint8List _certificateHash(String pin) {
  final match = RegExp(r'^sha256:([0-9a-fA-F]{64})$').firstMatch(pin);
  if (match == null) throw ArgumentError.value(pin, 'pin', 'Invalid SHA-256 certificate fingerprint');
  final hex = match[1]!;
  return Uint8List.fromList([
    for (int i = 0; i < 64; i += 2)
      int.parse(hex.substring(i, i + 2), radix: 16),
  ]);
}

/// Canoa ACP WebTransport profile: one reliable bidirectional stream, with
/// a little-endian u32 UTF-8 frame length before each complete JSON-RPC body.
/// ACP does not currently standardize this WebTransport profile.
Future<AcpMessageChannel> openAcpWebTransport(
  Uri endpoint,
  Duration timeout,
  List<String> certificatePins,
) async {
  if (_webTransportClass == null) {
    throw UnsupportedError('WebTransport is not supported by this browser');
  }
  final hashes = [
    for (final pin in certificatePins)
      _Hash(algorithm: 'sha-256', value: _certificateHash(pin).toJS),
  ];
  final _WebTransport wt = _WebTransport(
    endpoint.toString(),
    hashes.isEmpty ? JSObject() : _Options(serverCertificateHashes: hashes.toJS),
  );
  wt.closed.toDart.ignore();
  try {
    final stream = await wt.ready.toDart
        .then((_) => wt.createBidirectionalStream().toDart)
        .timeout(timeout);
    final channel = _WebTransportMessages(
      wt,
      stream.readable.getReader(),
      stream.writable.getWriter(),
    );
    return channel;
  } catch (_) {
    try { wt.close(); } catch (_) {}
    rethrow;
  }
}

final class _WebTransportMessages implements AcpMessageChannel {
  _WebTransportMessages(this._transport, this._reader, this._writer) {
    unawaited(_pump());
    _transport.closed.toDart.then(
      (_) => _finish(),
      onError: (Object error) => _finish(error),
    );
  }

  final _WebTransport _transport;
  final _Reader _reader;
  final _Writer _writer;
  final StreamController<String> _events = StreamController<String>();
  Future<void> _sending = Future<void>.value();
  bool _closing = false;
  bool _finished = false;

  @override
  Stream<String> get messages => _events.stream;

  void _finish([Object? error]) {
    if (_finished) return;
    _finished = true;
    if (error != null) _events.addError(error);
    unawaited(_events.close());
  }

  Future<void> _pump() async {
    var pending = Uint8List(0);
    try {
      for (;;) {
        final read = await _reader.read().toDart;
        if (read.done) break;
        final bytes = read.value?.toDart ?? Uint8List(0);
        if (bytes.isEmpty) continue;
        // Never buffer unbounded partial frames; a legal maximum-size frame
        // includes its 4-byte prefix.
        if (pending.length + bytes.length > maxAcpMessageBytes + 4) {
          throw FormatException('ACP WebTransport frame exceeds the size limit');
        }
        final next = Uint8List(pending.length + bytes.length);
        next.setRange(0, pending.length, pending);
        next.setRange(pending.length, next.length, bytes);
        pending = next;
        var offset = 0;
        while (pending.length - offset >= 4) {
          final size = ByteData.sublistView(pending, offset, offset + 4)
              .getUint32(0, Endian.little);
          if (size > maxAcpMessageBytes) {
            throw FormatException('ACP WebTransport message too large');
          }
          if (pending.length - offset < 4 + size) break;
          final json = utf8.decode(pending.sublist(offset + 4, offset + 4 + size));
          _events.add(json);
          offset += 4 + size;
        }
        pending = Uint8List.fromList(pending.sublist(offset));
      }
      if (pending.isNotEmpty) throw FormatException('Truncated ACP WebTransport frame');
      _finish();
    } catch (error) {
      _finish(error);
      try { _transport.close(); } catch (_) {}
    }
  }

  @override
  Future<void> send(String message) {
    if (_closing || _finished) return Future.error(StateError('WebTransport is closed'));
    final bytes = utf8.encode(message);
    if (bytes.length > maxAcpMessageBytes) {
      return Future.error(ArgumentError.value(bytes.length, 'message', 'ACP frame too large'));
    }
    final frame = Uint8List(4 + bytes.length);
    ByteData.sublistView(frame, 0, 4).setUint32(0, bytes.length, Endian.little);
    frame.setRange(4, frame.length, bytes);
    // Writes are ordered even when callers do not await their send().
    final sent = _sending.then((_) async { await _writer.write(frame.toJS).toDart; });
    _sending = sent;
    return sent;
  }

  @override
  Future<void> close() async {
    if (_closing) return;
    _closing = true;
    try { await _sending; } catch (_) {}
    try { await _writer.close().toDart.timeout(const Duration(seconds: 2)); } catch (_) {}
    try { await _reader.cancel().toDart; } catch (_) {}
    try { _transport.close(); } catch (_) {}
    _finish();
  }
}
