import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:coqui_web/coqui_web.dart';

/// Opens the pipe or socket [url] names; see `openPipeChannel`.
Future<ByteWebChannel> openPipeChannel(String url, Duration timeout, int maxMessage) async {
  final named = url.startsWith('pipe:');
  final name = url.substring(5);
  if (Platform.isWindows) {
    if (!named) throw const TransportUnavailable('A Unix socket is not used on Windows: this host listens on a pipe:NAME');
    return _WindowsPipe.open(name, timeout, maxMessage);
  }
  return _openSocket(named ? _runtimeSocket(name) : name, timeout, maxMessage);
}

/// Where `pipe:NAME` is on Unix: `NAME.sock` in `XDG_RUNTIME_DIR`, else `TMPDIR`, whichever is an absolute path first.
String _runtimeSocket(String name) {
  if (name.isEmpty || name.startsWith('.') || !RegExp(r'^[A-Za-z0-9._-]+$').hasMatch(name)) {
    throw const TransportUnavailable('Not the name of a pipe');
  }
  for (final variable in const ['XDG_RUNTIME_DIR', 'TMPDIR']) {
    final directory = Platform.environment[variable];
    if (directory != null && directory.startsWith('/')) return '${directory.replaceFirst(RegExp(r'/+$'), '')}/$name.sock';
  }
  throw const TransportUnavailable('No runtime directory to find the pipe in');
}

/// A byte stream as a channel of whole messages: a little-endian `u32` length, then the body.
abstract base class _FramedChannel implements ByteWebChannel {
  _FramedChannel(this._maxMessage) : _decoder = FrameDecoder(maxBytes: _maxMessage);

  final int _maxMessage;
  final FrameDecoder _decoder;
  final _incoming = StreamController<Uint8List>();
  bool _closed = false;

  /// What arrived, in any chunks.
  void received(Uint8List chunk) {
    if (_closed) return;
    try {
      _decoder.push(chunk);
      for (var message = _decoder.nextBytes(); message != null; message = _decoder.nextBytes()) {
        _incoming.add(message);
      }
    } on TransportProtocolFailure catch (failure) {
      _incoming.addError(failure);
      unawaited(close());
    }
  }

  /// The stream ended or failed: nothing more comes.
  void ended([Object? error]) {
    if (_incoming.isClosed) return;
    if (error != null) _incoming.addError(TransportUnavailable('The pipe failed: $error'));
    unawaited(_incoming.close());
  }

  Future<void> write(Uint8List frame);
  Future<void> closeTransport();

  @override
  WebChannelKind get kind => WebChannelKind.webSocket;

  @override
  Stream<Uint8List> get bytes => _incoming.stream;

  @override
  Stream<String> get messages => _incoming.stream.map(utf8.decode);

  @override
  Future<void> send(String message) => sendBytes(Uint8List.fromList(utf8.encode(message)));

  @override
  Future<void> sendBytes(Uint8List message) {
    if (_closed) return Future.error(const TransportUnavailable('The pipe is closed'));
    return write(encodeBytesFrame(message, maxBytes: _maxMessage));
  }

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    await closeTransport();
    ended();
  }
}

Future<ByteWebChannel> _openSocket(String path, Duration timeout, int maxMessage) async {
  try {
    final socket = await Socket.connect(InternetAddress(path, type: InternetAddressType.unix), 0, timeout: timeout);
    return _SocketChannel(socket, maxMessage);
  } on SocketException catch (error) {
    throw TransportUnavailable('Cannot open the socket: ${error.osError?.message ?? error.message}');
  } on TimeoutException {
    throw const TransportUnavailable('The socket did not answer in time');
  }
}

final class _SocketChannel extends _FramedChannel {
  _SocketChannel(this._socket, super.maxMessage) {
    _socket.listen(
      (chunk) => received(Uint8List.fromList(chunk)),
      onError: (Object error) => ended(error),
      onDone: ended,
      cancelOnError: true,
    );
  }

  final Socket _socket;

  @override
  Future<void> write(Uint8List frame) async {
    try {
      _socket.add(frame);
      await _socket.flush();
    } on SocketException catch (error) {
      throw TransportUnavailable('Cannot write to the socket: ${error.message}');
    }
  }

  @override
  Future<void> closeTransport() async => _socket.destroy();
}

// Windows: a named pipe opened for overlapped I/O, since a synchronous handle holds a write behind a read waiting on the
// same handle. A read waits in an isolate of its own and a write in another, so neither holds the app's.

const int _genericRead = 0x80000000;
const int _genericWrite = 0x40000000;
const int _openExisting = 3;
// FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION: the host may learn who this is and may not act as it.
const int _openFlags = 0x40000000 | 0x00100000 | 0x00010000;
const int _errorAccessDenied = 5;
const int _statusPending = 0x103;
const int _chunk = 64 * 1024;

final class _Kernel32 {
  _Kernel32() : _library = DynamicLibrary.open('kernel32.dll');

  final DynamicLibrary _library;

  late final createFile = _library
      .lookupFunction<
        IntPtr Function(Pointer<Uint16>, Uint32, Uint32, Pointer<Void>, Uint32, Uint32, IntPtr),
        int Function(Pointer<Uint16>, int, int, Pointer<Void>, int, int, int)
      >('CreateFileW');
  late final readFile = _library
      .lookupFunction<
        Int32 Function(IntPtr, Pointer<Uint8>, Uint32, Pointer<Uint32>, Pointer<Uint8>),
        int Function(int, Pointer<Uint8>, int, Pointer<Uint32>, Pointer<Uint8>)
      >('ReadFile');
  late final writeFile = _library
      .lookupFunction<
        Int32 Function(IntPtr, Pointer<Uint8>, Uint32, Pointer<Uint32>, Pointer<Uint8>),
        int Function(int, Pointer<Uint8>, int, Pointer<Uint32>, Pointer<Uint8>)
      >('WriteFile');
  late final overlappedResult = _library
      .lookupFunction<
        Int32 Function(IntPtr, Pointer<Uint8>, Pointer<Uint32>, Int32),
        int Function(int, Pointer<Uint8>, Pointer<Uint32>, int)
      >('GetOverlappedResult');
  late final createEvent = _library
      .lookupFunction<
        IntPtr Function(Pointer<Void>, Int32, Int32, Pointer<Void>),
        int Function(Pointer<Void>, int, int, Pointer<Void>)
      >('CreateEventW');
  late final closeHandle = _library.lookupFunction<Int32 Function(IntPtr), int Function(int)>('CloseHandle');
  late final cancelIoEx = _library
      .lookupFunction<Int32 Function(IntPtr, Pointer<Uint8>), int Function(int, Pointer<Uint8>)>('CancelIoEx');
  late final lastError = _library.lookupFunction<Uint32 Function(), int Function()>('GetLastError');
  late final _alloc = _library
      .lookupFunction<Pointer<Uint8> Function(Uint32, IntPtr), Pointer<Uint8> Function(int, int)>('LocalAlloc');
  late final _free = _library.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>(
    'LocalFree',
  );

  /// Zeroed native memory.
  Pointer<Uint8> allocate(int bytes) => _alloc(0x40, bytes);

  void free(Pointer<Uint8> memory) => _free(memory.cast());
}

/// An OVERLAPPED with its event: what one blocking overlapped call at a time needs.
final class _Operation {
  _Operation(this._k)
    : overlapped = _k.allocate(3 * sizeOf<IntPtr>() + 8),
      count = _k.allocate(4).cast<Uint32>(),
      buffer = _k.allocate(_chunk),
      event = _k.createEvent(nullptr, 1, 0, nullptr);

  final _Kernel32 _k;
  final Pointer<Uint8> overlapped;
  final Pointer<Uint32> count;
  final Pointer<Uint8> buffer;
  final int event;

  /// Runs [call] on the handle and waits for it: the bytes it moved, or null when it failed or was cancelled.
  int? run(int handle, int Function(Pointer<Uint8> overlapped) call) {
    final size = 3 * sizeOf<IntPtr>() + 8;
    overlapped.asTypedList(size).fillRange(0, size, 0);
    // hEvent follows Internal, InternalHigh and the 8 bytes of Offset and OffsetHigh.
    (overlapped + (2 * sizeOf<IntPtr>() + 8)).cast<IntPtr>().value = event;
    // GetLastError is not reliable between calls from Dart, so a call that did not finish is told from one that is pending by
    // the OVERLAPPED's own status.
    if (call(overlapped) == 0 && overlapped.cast<IntPtr>().value != _statusPending) return null;
    if (_k.overlappedResult(handle, overlapped, count, 1) == 0) return null;
    return count.value;
  }

  void release() {
    _k.closeHandle(event);
    _k.free(overlapped);
    _k.free(count.cast());
    _k.free(buffer);
  }
}

/// Reads until the pipe ends, sending each piece to [args]`[0]` and `null` at the end.
void _readLoop(List<Object> args) {
  final port = args[0] as SendPort;
  final handle = args[1] as int;
  final k = _Kernel32();
  final operation = _Operation(k);
  try {
    while (true) {
      final read = operation.run(
        handle,
        (overlapped) => k.readFile(handle, operation.buffer, _chunk, nullptr, overlapped),
      );
      if (read == null) break;
      if (read == 0) continue;
      port.send(TransferableTypedData.fromList([Uint8List.fromList(operation.buffer.asTypedList(read))]));
    }
  } finally {
    operation.release();
    port.send(null);
  }
}

/// Writes each frame it is sent and answers `true`, or `false` when the pipe failed; `null` ends it.
void _writeLoop(List<Object> args) {
  final replies = args[0] as SendPort;
  final handle = args[1] as int;
  final k = _Kernel32();
  final operation = _Operation(k);
  final commands = ReceivePort();
  replies.send(commands.sendPort);
  commands.listen((Object? message) {
    if (message == null) {
      operation.release();
      commands.close();
      replies.send(null);
      return;
    }
    final frame = (message as TransferableTypedData).materialize().asUint8List();
    var offset = 0;
    var failed = false;
    while (offset < frame.length) {
      final size = math.min(_chunk, frame.length - offset);
      operation.buffer.asTypedList(size).setAll(0, Uint8List.sublistView(frame, offset, offset + size));
      final written = operation.run(
        handle,
        (overlapped) => k.writeFile(handle, operation.buffer, size, nullptr, overlapped),
      );
      if (written == null || written == 0) {
        failed = true;
        break;
      }
      offset += written;
    }
    replies.send(!failed);
  });
}

final class _WindowsPipe extends _FramedChannel {
  _WindowsPipe._(this._k, this._handle, super.maxMessage);

  final _Kernel32 _k;
  final int _handle;
  final _reads = ReceivePort();
  final _writes = ReceivePort();
  final _readerDone = Completer<void>();
  final _writerDone = Completer<void>();
  final _answers = <Completer<void>>[];
  SendPort? _commands;
  Future<void> _writing = Future<void>.value();

  static Future<ByteWebChannel> open(String name, Duration timeout, int maxMessage) async {
    if (name.isEmpty || name.contains('/') || name.startsWith(r'\') || name.split(r'\').any((part) => part == '.' || part == '..')) {
      throw const TransportUnavailable('Not the name of a pipe');
    }
    final k = _Kernel32();
    final text = r'\\.\pipe\' + name;
    final units = k.allocate((text.length + 1) * 2).cast<Uint16>();
    units.asTypedList(text.length).setAll(0, text.codeUnits);
    final deadline = DateTime.now().add(timeout);
    try {
      while (true) {
        final handle = k.createFile(units, _genericRead | _genericWrite, 0, nullptr, _openExisting, _openFlags, 0);
        if (handle != -1) return await _WindowsPipe._(k, handle, maxMessage)._start();
        // GetLastError may not say what failed, so the pipe's being there is asked of the file system: a pipe with every
        // instance in use is waited for, and one that is not there is not.
        if (FileSystemEntity.typeSync(text) == FileSystemEntityType.notFound) {
          throw const TransportUnavailable('Nothing listens on the pipe');
        }
        if (k.lastError() == _errorAccessDenied) throw const TransportUnavailable('The system refused this user the pipe');
        if (DateTime.now().isAfter(deadline)) throw const TransportUnavailable('The pipe stayed busy');
        await Future<void>.delayed(const Duration(milliseconds: 20));
      }
    } finally {
      k.free(units.cast());
    }
  }

  Future<ByteWebChannel> _start() async {
    final ready = Completer<SendPort>();
    _writes.listen((Object? message) {
      if (message is SendPort) {
        ready.complete(message);
      } else if (message == null) {
        if (!_writerDone.isCompleted) _writerDone.complete();
      } else if (_answers.isNotEmpty) {
        final waiting = _answers.removeAt(0);
        if (message == true) {
          waiting.complete();
        } else {
          waiting.completeError(const TransportUnavailable('Cannot write to the pipe'));
        }
      }
    });
    _reads.listen((Object? message) {
      if (message == null) {
        if (!_readerDone.isCompleted) _readerDone.complete();
        ended();
      } else {
        received((message as TransferableTypedData).materialize().asUint8List());
      }
    });
    await Isolate.spawn(_readLoop, <Object>[_reads.sendPort, _handle]);
    await Isolate.spawn(_writeLoop, <Object>[_writes.sendPort, _handle]);
    _commands = await ready.future;
    return this;
  }

  @override
  Future<void> write(Uint8List frame) {
    final commands = _commands;
    if (commands == null) return Future.error(const TransportUnavailable('The pipe is closed'));
    final answer = Completer<void>();
    // One frame at a time, in the order they were sent.
    _writing = _writing.catchError((Object _) {}).then((_) {
      _answers.add(answer);
      commands.send(TransferableTypedData.fromList([frame]));
      return answer.future.catchError((Object _) {});
    });
    return answer.future;
  }

  @override
  Future<void> closeTransport() async {
    _k.cancelIoEx(_handle, nullptr);
    _commands?.send(null);
    _commands = null;
    const grace = Duration(seconds: 2);
    await Future.wait([_readerDone.future, _writerDone.future]).timeout(grace, onTimeout: () => const []);
    _k.closeHandle(_handle);
    _reads.close();
    _writes.close();
    for (final waiting in _answers) {
      if (!waiting.isCompleted) waiting.completeError(const TransportUnavailable('The pipe is closed'));
    }
    _answers.clear();
  }
}
