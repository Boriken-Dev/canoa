// The microphone of a dictation, against a stand-in for the platform's recorder: a recorded tone plays where the microphone would,
// and the test sees when the recorder opens and when it is released. No real microphone is opened.
import 'dart:async';
import 'dart:typed_data';

import 'package:canoa/voice/record_capture.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:record/record.dart';

import 'fixtures/record_standin.dart';

void main() {
  late StandInRecorder platform;

  Future<void> use(StandInRecorder next) async {
    platform = next;
    RecordPlatform.instance = next;
  }

  test('nothing is opened until a dictation listens, and the tone arrives as 16 kHz mono 16-bit PCM', () async {
    await use(StandInRecorder());
    final capture = RecordCapture();
    final stream = capture.open(rate: 16000);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(platform.calls, isEmpty, reason: 'the microphone must not open before the stream is listened to');

    final heard = <int>[];
    final done = Completer<void>();
    final sub = stream.listen((chunk) {
      heard.addAll(chunk);
      if (heard.length >= 16000 && !done.isCompleted) done.complete();
    });
    await done.future.timeout(const Duration(seconds: 5));
    expect(platform.config!.encoder, AudioEncoder.pcm16bits);
    expect(platform.config!.sampleRate, 16000);
    expect(platform.config!.numChannels, 1);
    expect(Uint8List.fromList(heard), recordedTone());
    expect(platform.recording, isTrue);

    await sub.cancel();
    expect(platform.recording, isFalse, reason: 'the microphone closes when the dictation ends');
    expect(platform.calls.last, 'dispose');
  });

  test('a microphone the person did not allow is a failure in words, and nothing records', () async {
    await use(StandInRecorder(allowed: false));
    await expectLater(
      RecordCapture().open(rate: 16000).first,
      throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('microphone'))),
    );
    expect(platform.calls, isNot(contains('startStream')));
    expect(platform.calls.last, 'dispose');
  });

  test('a recorder that cannot start is a failure that says why', () async {
    await use(StandInRecorder(failsToStart: true));
    await expectLater(
      RecordCapture().open(rate: 16000).first,
      throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('parecord'))),
    );
    expect(platform.calls.last, 'dispose');
  });
}
