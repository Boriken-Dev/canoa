import 'dart:async';
import 'dart:typed_data';

import 'package:record/record.dart';

import 'host_speech.dart';
import 'speech_backend.dart';

/// The microphone on every platform the app runs on: the browser's on a web page, the system's on Windows, macOS and Linux
/// (Linux records through `parecord`, which PulseAudio and PipeWire's Pulse layer provide).
///
/// The recorder is made when a dictation listens to [open] and released when it stops listening, so nothing is open between
/// dictations and nothing is written to disk: the audio goes from here to the host in chunks.
final class RecordCapture implements AudioCapture {
  const RecordCapture();

  @override
  Stream<Uint8List> open({required int rate}) async* {
    final recorder = AudioRecorder();
    try {
      if (!await recorder.hasPermission()) {
        throw const SpeechFailure('The microphone is not allowed for this app. Allow it in the browser or the system settings and try again.');
      }
      final Stream<Uint8List> audio;
      try {
        audio = await recorder.startStream(RecordConfig(encoder: AudioEncoder.pcm16bits, sampleRate: rate, numChannels: 1));
      } on Object catch (error) {
        throw SpeechFailure('The microphone could not be opened: $error');
      }
      yield* audio;
    } finally {
      try {
        await recorder.dispose();
      } on Object {
        // Releasing a recorder that never started has nothing to say.
      }
    }
  }
}
