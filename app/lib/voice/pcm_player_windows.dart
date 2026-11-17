import 'dart:async';
import 'dart:ffi';
import 'dart:typed_data';

import 'pcm_player.dart';
import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart' show PcmFormat;

/// WAVEFORMATEX, which Windows packs to one byte.
@Packed(1)
final class _WaveFormat extends Struct {
  @Uint16()
  external int tag;
  @Uint16()
  external int channels;
  @Uint32()
  external int rate;
  @Uint32()
  external int bytesPerSecond;
  @Uint16()
  external int blockAlign;
  @Uint16()
  external int bits;
  @Uint16()
  external int extra;
}

/// WAVEHDR: one buffer handed to the device.
final class _WaveHeader extends Struct {
  external Pointer<Uint8> data;
  @Uint32()
  external int length;
  @Uint32()
  external int recorded;
  @IntPtr()
  external int user;
  @Uint32()
  external int flags;
  @Uint32()
  external int loops;
  external Pointer<Void> next;
  @IntPtr()
  external int reserved;
}

/// The bit of a header's flags the device sets when it has played the buffer.
const _done = 0x1;

typedef _Open = int Function(Pointer<IntPtr>, int, Pointer<_WaveFormat>, int, int, int);
typedef _OpenC = Uint32 Function(Pointer<IntPtr>, Uint32, Pointer<_WaveFormat>, IntPtr, IntPtr, Uint32);
typedef _Header = int Function(int, Pointer<_WaveHeader>, int);
typedef _HeaderC = Uint32 Function(IntPtr, Pointer<_WaveHeader>, Uint32);
typedef _Handle = int Function(int);
typedef _HandleC = Uint32 Function(IntPtr);

/// Windows' own audio output, `waveOut` from winmm.dll, which every Windows has: buffers are handed to the device as they arrive and
/// it plays them one after another. No plugin and nothing downloaded.
final class WaveOutPlayer implements PcmPlayer {
  WaveOutPlayer({this.deviceId = 0xFFFFFFFF});

  /// The device to play on; the default is the one the person chose in Windows' sound settings.
  final int deviceId;

  late final DynamicLibrary _winmm = DynamicLibrary.open('winmm.dll');
  late final DynamicLibrary _kernel = DynamicLibrary.open('kernel32.dll');
  late final _Open _waveOpen = _winmm.lookupFunction<_OpenC, _Open>('waveOutOpen');
  late final _Header _prepare = _winmm.lookupFunction<_HeaderC, _Header>('waveOutPrepareHeader');
  late final _Header _unprepare = _winmm.lookupFunction<_HeaderC, _Header>('waveOutUnprepareHeader');
  late final _Header _write = _winmm.lookupFunction<_HeaderC, _Header>('waveOutWrite');
  late final _Handle _reset = _winmm.lookupFunction<_HandleC, _Handle>('waveOutReset');
  late final _Handle _close = _winmm.lookupFunction<_HandleC, _Handle>('waveOutClose');
  late final Pointer<Uint8> Function(int, int) _alloc =
      _kernel.lookupFunction<Pointer<Uint8> Function(Uint32, IntPtr), Pointer<Uint8> Function(int, int)>('LocalAlloc');
  late final Pointer<Void> Function(Pointer<Void>) _localFree =
      _kernel.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>('LocalFree');

  void _free(Pointer<NativeType> memory) => _localFree(memory.cast());

  int _device = 0;
  PcmFormat? _format;
  final _inFlight = <Pointer<_WaveHeader>>[];
  int _queued = 0;
  Timer? _poll;
  Completer<void>? _idle;
  Completer<void>? _room;
  Uint8List _carry = Uint8List(0);

  @override
  void unlock() {}

  @override
  Future<void> open(PcmFormat format) async {
    if (_format == format) return;
    if (_format != null) await drain();
    final handle = _alloc(0x40, 8).cast<IntPtr>();
    final wave = _alloc(0x40, sizeOf<_WaveFormat>()).cast<_WaveFormat>();
    try {
      wave.ref
        ..tag = 1
        ..channels = format.channels
        ..rate = format.rate
        ..bytesPerSecond = bytesPerSecond(format)
        ..blockAlign = format.channels * 2
        ..bits = 16
        ..extra = 0;
      final answer = _waveOpen(handle, deviceId, wave, 0, 0, 0);
      if (answer != 0) {
        throw SpeechFailure(
          answer == 2 || answer == 6
              ? 'Windows has no audio output to play on.'
              : 'Windows could not open the audio output (code $answer).',
        );
      }
      _device = handle.value;
    } finally {
      _free(handle);
      _free(wave);
    }
    _format = format;
    _carry = Uint8List(0);
  }

  @override
  Future<void> write(Uint8List pcm) async {
    final format = _format;
    if (format == null) return;
    final block = format.channels * 2;
    var bytes = pcm;
    if (_carry.isNotEmpty) {
      bytes = Uint8List(_carry.length + pcm.length)
        ..setRange(0, _carry.length, _carry)
        ..setRange(_carry.length, _carry.length + pcm.length, pcm);
    }
    final whole = bytes.length - bytes.length % block;
    _carry = Uint8List.fromList(bytes.sublist(whole));
    if (whole == 0) return;
    final data = _alloc(0x40, whole);
    data.asTypedList(whole).setRange(0, whole, bytes);
    final header = _alloc(0x40, sizeOf<_WaveHeader>()).cast<_WaveHeader>();
    header.ref
      ..data = data
      ..length = whole;
    final size = sizeOf<_WaveHeader>();
    if (_prepare(_device, header, size) != 0 || _write(_device, header, size) != 0) {
      _unprepare(_device, header, size);
      _free(data);
      _free(header);
      throw const SpeechFailure('Windows could not play the audio.');
    }
    _inFlight.add(header);
    _queued += whole;
    _poll ??= Timer.periodic(const Duration(milliseconds: 30), (_) => _reap());
    if (_queued > bytesPerSecond(format) * pcmAheadSeconds) {
      final room = _room = Completer<void>();
      await room.future;
    }
  }

  /// Frees the buffers the device has played, and wakes whoever waits for room or for the end.
  void _reap() {
    final format = _format;
    final size = sizeOf<_WaveHeader>();
    while (_inFlight.isNotEmpty && (_inFlight.first.ref.flags & _done) != 0) {
      final header = _inFlight.removeAt(0);
      _queued -= header.ref.length;
      _unprepare(_device, header, size);
      _free(header.ref.data);
      _free(header);
    }
    if (_room != null && (format == null || _queued <= bytesPerSecond(format) * pcmAheadSeconds)) {
      _room!.complete();
      _room = null;
    }
    if (_inFlight.isEmpty) {
      _poll?.cancel();
      _poll = null;
      _idle?.complete();
      _idle = null;
    }
  }

  @override
  Future<void> drain() async {
    if (_format == null) return;
    if (_inFlight.isNotEmpty) {
      final idle = _idle ??= Completer<void>();
      await idle.future;
    }
    _closeDevice();
  }

  @override
  Future<void> stop() async {
    if (_format == null) return;
    _reset(_device);
    _reap();
    _closeDevice();
  }

  void _closeDevice() {
    _poll?.cancel();
    _poll = null;
    _close(_device);
    _device = 0;
    _format = null;
    _queued = 0;
    _carry = Uint8List(0);
    _room?.complete();
    _room = null;
    _idle?.complete();
    _idle = null;
  }
}

