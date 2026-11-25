import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'host_speech.dart' show AudioCapture;
import 'speech_backend.dart';

/// What the Whisper that runs in the page heard.
final class WhisperHeard {
  const WhisperHeard(this.text, {this.language});

  final String text;

  /// The language code the model read the recording in: the one asked for, or the one it detected.
  final String? language;
}

/// Whisper running inside this page, as a web worker over WebAssembly (`whisper_worker.js`, built from `app/web_whisper`). The page
/// holds no model of its own: the worker fetches a model's files from the pinned addresses its module lists, checks each against its
/// SHA-256 and keeps them in the browser's cache.
abstract interface class WhisperRuntime {
  /// The models the module reads, the default first.
  Future<List<SpeechChoice>> models();

  /// The text of 16 kHz mono [samples] between -1 and 1 in [language] (a code), or the language the model detects; [prompt] is the
  /// words the person says that a model might get wrong. [progress] hears what the worker is doing (downloading, loading).
  Future<WhisperHeard> transcribe({
    required String model,
    required Float32List samples,
    String? language,
    String? prompt,
    void Function(String words)? progress,
  });
}

/// The id `[voice] backend` names Whisper in the page by.
const webWhisperId = 'web-whisper';

/// The model used when the person chose none, or one the module does not list.
const _fallbackModel = 'base';

/// The sample rate Whisper reads.
const _rate = 16000;

/// The shortest recording, in samples, that is turned into text: 100 ms.
const _shortest = 1600;

/// Whisper in the page as a speech service: the microphone's audio is kept in memory until the person stops, then a worker turns it
/// into text with a model fetched once from its publisher. The audio never leaves the page.
final class WebWhisper implements SpeechBackend {
  WebWhisper(this._runtime, this._capture);

  final WhisperRuntime _runtime;
  final AudioCapture _capture;
  final _levels = StreamController<double>.broadcast();
  StreamSubscription<Uint8List>? _audio;
  Completer<void> _ended = Completer<void>();
  bool _cancelled = false;

  @override
  String get id => webWhisperId;

  @override
  String get name => 'Whisper in this browser';

  @override
  SpeechWhere get where => const SpeechWhere(
    place: SpeechPlace.thisDevice,
    leaves: AudioLeaves.no,
    words: 'Whisper runs in this page, on this device\'s processor, and the audio is never sent anywhere. The first time, the model is '
        'downloaded from its publisher (Hugging Face) and kept in the browser; the host\'s page policy has to allow that address.',
  );

  @override
  Future<List<SpeechChoice>> models() => _runtime.models();

  @override
  Future<List<SpeechChoice>> languages() async => const [];

  @override
  Future<List<SpeechChoice>> devices() async => const [];

  @override
  Stream<double> get levels => _levels.stream;

  @override
  Stream<Transcript> start({String? device, String? language, String? model, VoiceProfile? profile}) {
    final out = StreamController<Transcript>();
    unawaited(_run(out, language, model, profile));
    return out.stream;
  }

  Future<void> _run(StreamController<Transcript> out, String? language, String? model, VoiceProfile? profile) async {
    _ended = Completer<void>();
    _cancelled = false;
    final heard = BytesBuilder(copy: false);
    try {
      _audio = _capture.open(rate: _rate).listen(
        (chunk) {
          heard.add(chunk);
          if (!_levels.isClosed) _levels.add(_level(chunk));
        },
        onError: (Object error) {
          if (!out.isClosed) out.addError(error is SpeechFailure ? error : const SpeechFailure('The microphone failed.'));
          if (!_ended.isCompleted) _ended.complete();
        },
        onDone: () {
          if (!_ended.isCompleted) _ended.complete();
        },
        cancelOnError: false,
      );
      // `stop` and `cancel` end the audio.
      await _ended.future;
      await _audio?.cancel().timeout(const Duration(seconds: 5), onTimeout: () {});
      _audio = null;
      if (!_cancelled) {
        final samples = samplesOf(heard.takeBytes());
        if (samples.length >= _shortest) {
          final wanted = await _model(model);
          final said = await _runtime.transcribe(
            model: wanted,
            samples: samples,
            language: primaryLanguage(language),
            prompt: profile == null || profile.vocabulary.isEmpty ? null : profile.vocabulary.join(', '),
            progress: (words) {
              if (!out.isClosed && !_cancelled) out.add(Transcript(words, isFinal: false));
            },
          );
          if (!out.isClosed && !_cancelled) {
            out.add(Transcript(said.text, isFinal: true, frontMatter: {'language': ?said.language}));
          }
        }
      }
    } on SpeechFailure catch (failure) {
      if (!out.isClosed) out.addError(failure);
    } on Object catch (error) {
      if (!out.isClosed) out.addError(SpeechFailure('Whisper could not turn the recording into text: $error'));
    }
    if (!out.isClosed) await out.close();
  }

  /// The model asked for when the module lists it, else the default.
  Future<String> _model(String? asked) async {
    final listed = await _runtime.models();
    if (asked != null && listed.any((each) => each.id == asked)) return asked;
    return listed.any((each) => each.id == _fallbackModel) ? _fallbackModel : listed.first.id;
  }

  @override
  Future<void> stop() async {
    if (!_ended.isCompleted) _ended.complete();
  }

  @override
  Future<void> cancel() async {
    _cancelled = true;
    if (!_ended.isCompleted) _ended.complete();
  }
}

/// The primary subtag of a BCP 47 tag (`es` for `es-PR`), or null for none; the module's engine knows languages by it.
String? primaryLanguage(String? tag) {
  if (tag == null) return null;
  final primary = tag.split(RegExp('[-_]')).first.trim();
  return primary.isEmpty ? null : primary.toLowerCase();
}

/// 16-bit little-endian mono PCM as samples between -1 and 1.
Float32List samplesOf(Uint8List pcm) {
  final count = pcm.length ~/ 2;
  final view = ByteData.sublistView(pcm);
  final samples = Float32List(count);
  for (var i = 0; i < count; i++) {
    samples[i] = view.getInt16(i * 2, Endian.little) / 32768.0;
  }
  return samples;
}

/// How loud a chunk is, from 0 to 1.
double _level(Uint8List chunk) {
  final count = chunk.length ~/ 2;
  if (count == 0) return 0;
  final view = ByteData.sublistView(chunk);
  var sum = 0.0;
  for (var i = 0; i < count; i++) {
    final sample = view.getInt16(i * 2, Endian.little) / 32768.0;
    sum += sample * sample;
  }
  return math.min(1.0, math.sqrt(sum / count) * 4);
}
