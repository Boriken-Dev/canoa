// A stand-in for the platform's recorder: a recorded tone plays where the microphone would, and the test sees when the recorder
// opens and when it is released. No real microphone is opened.
import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:record/record.dart';

/// Half a second of a 440 Hz tone, 16-bit little-endian mono at 16 kHz.
Uint8List recordedTone() {
  final samples = ByteData(16000);
  for (var i = 0; i < 8000; i++) {
    samples.setInt16(i * 2, (math.sin(2 * math.pi * 440 * i / 16000) * 12000).round(), Endian.little);
  }
  return samples.buffer.asUint8List();
}

final class StandInRecorder extends RecordPlatform {
  StandInRecorder({this.allowed = true, this.failsToStart = false});

  final bool allowed;
  final bool failsToStart;
  final calls = <String>[];
  RecordConfig? config;
  StreamController<Uint8List>? _stream;

  bool get recording => _stream != null;

  @override
  Future<void> create(String recorderId) async => calls.add('create');

  @override
  Future<bool> hasPermission(String recorderId, {bool request = true}) async {
    calls.add('permission');
    return allowed;
  }

  @override
  Future<Stream<Uint8List>> startStream(String recorderId, RecordConfig config) async {
    calls.add('startStream');
    if (failsToStart) throw StateError('parecord: command not found at /usr/bin/parecord');
    this.config = config;
    final stream = _stream = StreamController<Uint8List>();
    final tone = recordedTone();
    Timer(const Duration(milliseconds: 20), () {
      for (var at = 0; at < tone.length; at += 4096) {
        stream.add(Uint8List.sublistView(tone, at, math.min(at + 4096, tone.length)));
      }
    });
    return stream.stream;
  }

  @override
  Future<bool> isRecording(String recorderId) async => recording;

  @override
  Future<String?> stop(String recorderId) async {
    calls.add('stop');
    await _stream?.close();
    _stream = null;
    return null;
  }

  @override
  Future<void> cancel(String recorderId) async => stop(recorderId);

  @override
  Future<void> dispose(String recorderId) async => calls.add('dispose');

  @override
  Stream<RecordState> onStateChanged(String recorderId) => const Stream.empty();

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
