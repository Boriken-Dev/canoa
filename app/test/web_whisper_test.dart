import 'dart:async';
import 'dart:typed_data';

import 'package:canoa/voice/host_speech.dart' show AudioCapture;
import 'package:canoa/voice/speech_backend.dart';
import 'package:canoa/voice/web_whisper.dart';
import 'package:flutter_test/flutter_test.dart';

/// The microphone: [seconds] of a constant sound.
final class _Microphone implements AudioCapture {
  _Microphone(this.seconds);

  final double seconds;
  int? rate;

  @override
  Stream<Uint8List> open({required int rate}) {
    this.rate = rate;
    final samples = (seconds * rate).round();
    final pcm = ByteData(samples * 2);
    for (var i = 0; i < samples; i++) {
      pcm.setInt16(i * 2, 16384, Endian.little);
    }
    // The audio stays open after its one chunk, until the dictation cancels it.
    final audio = StreamController<Uint8List>();
    audio.add(pcm.buffer.asUint8List());
    return audio.stream;
  }
}

final class _Runtime implements WhisperRuntime {
  final asked = <({String model, int samples, String? language, String? prompt})>[];
  var transcribed = 0;

  @override
  Future<List<SpeechChoice>> models() async => const [SpeechChoice('base', 'Base'), SpeechChoice('tiny', 'Tiny')];

  @override
  Future<WhisperHeard> transcribe({
    required String model,
    required Float32List samples,
    String? language,
    String? prompt,
    void Function(String words)? progress,
  }) async {
    transcribed++;
    asked.add((model: model, samples: samples.length, language: language, prompt: prompt));
    progress?.call('Loading $model');
    expect(samples.every((sample) => sample == 0.5), isTrue, reason: 'the 16-bit samples are scaled to -1..1');
    return const WhisperHeard('hello world', language: 'en');
  }
}

void main() {
  test('what is said is kept until the person stops, then read in the model and language asked for with their words as the prompt', () async {
    final runtime = _Runtime();
    final microphone = _Microphone(1);
    final whisper = WebWhisper(runtime, microphone);
    final heard = <Transcript>[];
    final done = Completer<void>();
    whisper
        .start(
          language: 'es-PR',
          model: 'tiny',
          profile: const VoiceProfile(vocabulary: ['Canoa', 'Boriken']),
        )
        .listen(heard.add, onDone: done.complete);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(heard, isEmpty, reason: 'nothing is read while the microphone is open');
    await whisper.stop();
    await done.future;
    expect(microphone.rate, 16000);
    expect(runtime.asked, [(model: 'tiny', samples: 16000, language: 'es', prompt: 'Canoa, Boriken')]);
    expect(heard.map((each) => (each.text, each.isFinal)), [('Loading tiny', false), ('hello world', true)]);
    expect(heard.last.frontMatter, {'language': 'en'});
    expect(whisper.where.place, SpeechPlace.thisDevice);
    expect(whisper.where.leaves, AudioLeaves.no);
  });

  test('a model the module does not list is the default, and a recording shorter than 100 ms has no words', () async {
    final runtime = _Runtime();
    final whisper = WebWhisper(runtime, _Microphone(1));
    final done = Completer<void>();
    whisper.start(model: 'huge').listen((_) {}, onDone: done.complete);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    await whisper.stop();
    await done.future;
    expect(runtime.asked.single.model, 'base');
    expect(runtime.asked.single.language, isNull);
    expect(runtime.asked.single.prompt, isNull);

    final quiet = _Runtime();
    final brief = WebWhisper(quiet, _Microphone(0.05));
    final ended = Completer<void>();
    final events = <Transcript>[];
    brief.start().listen(events.add, onDone: ended.complete);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    await brief.stop();
    await ended.future;
    expect(quiet.transcribed, 0);
    expect(events, isEmpty);
  });

  test('cancelling drops the recording unread, and a failure of the worker reaches the dictation as a failure', () async {
    final runtime = _Runtime();
    final whisper = WebWhisper(runtime, _Microphone(1));
    final done = Completer<void>();
    final events = <Transcript>[];
    whisper.start().listen(events.add, onDone: done.complete);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    await whisper.cancel();
    await done.future;
    expect(runtime.transcribed, 0);
    expect(events, isEmpty);

    final broken = WebWhisper(_Failing(), _Microphone(1));
    final failed = Completer<Object>();
    broken.start().listen((_) {}, onError: failed.complete);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    await broken.stop();
    expect(await failed.future, isA<SpeechFailure>().having((failure) => failure.message, 'message', contains('could not be downloaded')));
  });

  test('a language tag is its primary subtag in lower case, and 16-bit samples are scaled', () {
    expect(primaryLanguage('es-PR'), 'es');
    expect(primaryLanguage('PT_br'), 'pt');
    expect(primaryLanguage(''), isNull);
    expect(primaryLanguage(null), isNull);
    final pcm = ByteData(6)
      ..setInt16(0, 0, Endian.little)
      ..setInt16(2, -32768, Endian.little)
      ..setInt16(4, 16384, Endian.little);
    expect(samplesOf(pcm.buffer.asUint8List()), [0.0, -1.0, 0.5]);
  });
}

final class _Failing extends _Runtime {
  @override
  Future<WhisperHeard> transcribe({
    required String model,
    required Float32List samples,
    String? language,
    String? prompt,
    void Function(String words)? progress,
  }) => throw const SpeechFailure('The model could not be downloaded.');
}
