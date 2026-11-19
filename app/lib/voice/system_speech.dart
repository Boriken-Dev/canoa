import 'dart:async';

import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform, kIsWeb;
import 'package:speech_to_text/speech_recognition_error.dart';
import 'package:speech_to_text/speech_recognition_result.dart';
import 'package:speech_to_text/speech_to_text.dart';

import '../config/canoa_config.dart' show defaultVoiceBackend;
import 'speech_backend.dart';

/// The speech backends this build can offer: the operating system's service on Windows and macOS, none elsewhere (a browser's
/// recognizer sends audio to its vendor, and no Linux desktop has a service to ask).
List<SpeechBackend> platformSpeechBackends() => [
  if (!kIsWeb && (defaultTargetPlatform == TargetPlatform.windows || defaultTargetPlatform == TargetPlatform.macOS))
    SystemSpeech(defaultTargetPlatform == TargetPlatform.windows ? 'Windows speech recognition' : 'macOS Speech'),
];

/// The operating system's own speech service, through the platform plugin. The plugin is started on the first [start], not
/// before, so the system asks for the microphone only when the person starts to dictate.
final class SystemSpeech implements SpeechBackend {
  /// [finishWithin] is how long the system may take, after [stop], to say it has finished before the dictation is ended with a
  /// reason: a recognizer that never says it is done must not hold the composer.
  SystemSpeech(this.name, {SpeechToText? engine, this.finishWithin = const Duration(seconds: 10)}) : _engine = engine ?? SpeechToText();

  final SpeechToText _engine;
  final Duration finishWithin;
  final _levels = StreamController<double>.broadcast();
  StreamController<Transcript>? _capture;
  Timer? _deadline;
  bool _ready = false;

  @override
  String get id => defaultVoiceBackend;

  @override
  final String name;

  @override
  SpeechWhere get where => const SpeechWhere(
    place: SpeechPlace.system,
    leaves: AudioLeaves.maybe,
    words: 'The operating system turns it into text. When its speech service is online, the system sends the audio to its '
        'maker; its own privacy settings say whether that is on.',
  );

  @override
  Future<List<SpeechChoice>> models() async => const [];

  @override
  Future<List<SpeechChoice>> languages() async {
    if (!_ready) return const [];
    return [for (final locale in await _engine.locales()) SpeechChoice(locale.localeId, locale.name)];
  }

  @override
  Future<List<SpeechChoice>> devices() async => const [];

  @override
  Stream<double> get levels => _levels.stream;

  @override
  Stream<Transcript> start({String? device, String? language, String? model, VoiceProfile? profile}) {
    _deadline?.cancel();
    final capture = StreamController<Transcript>();
    _capture = capture;
    _run(capture, language);
    return capture.stream;
  }

  /// Ends the capture now running, if any: the engine reports one status for whichever listen is current, so it is routed here
  /// and not to the capture that was current when the engine was first initialized.
  void _finish() {
    _deadline?.cancel();
    _deadline = null;
    final capture = _capture;
    _capture = null;
    if (capture != null && !capture.isClosed) unawaited(capture.close());
  }

  void _fail(String message) {
    final capture = _capture;
    if (capture != null && !capture.isClosed) capture.addError(SpeechFailure(message));
    _finish();
  }

  Future<void> _run(StreamController<Transcript> capture, String? language) async {
    try {
      if (!_ready) {
        _ready = await _engine.initialize(
          onError: (SpeechRecognitionError error) {
            if (error.permanent) _fail(_said(error.errorMsg));
          },
          onStatus: (status) {
            if (status == SpeechToText.doneStatus || status == SpeechToText.notListeningStatus) _finish();
          },
        );
        if (!_ready) {
          if (identical(_capture, capture)) {
            _fail('The system has no speech service ready. Check that the microphone is allowed and speech recognition is on.');
          }
          return;
        }
      }
      await _engine.listen(
        onResult: (SpeechRecognitionResult result) {
          if (!capture.isClosed && result.recognizedWords.isNotEmpty) {
            capture.add(Transcript(result.recognizedWords, isFinal: result.finalResult));
          }
        },
        onSoundLevelChange: (db) {
          if (!_levels.isClosed) _levels.add(((db + 2) / 12).clamp(0.0, 1.0));
        },
        listenOptions: SpeechListenOptions(listenMode: ListenMode.dictation, partialResults: true, cancelOnError: true, localeId: language),
      );
    } on Object catch (error) {
      if (identical(_capture, capture)) _fail('The speech service could not start: ${error.runtimeType}.');
    }
  }

  static String _said(String code) => switch (code) {
    'error_permission' || 'error_audio' => 'The microphone is not allowed or not available.',
    'error_network' || 'error_network_timeout' => 'The speech service could not be reached.',
    'error_no_match' || 'error_speech_timeout' => 'No speech was heard.',
    _ => 'The speech service stopped: $code.',
  };

  @override
  Future<void> stop() async {
    final capture = _capture;
    _deadline?.cancel();
    _deadline = Timer(finishWithin, () {
      if (identical(_capture, capture)) _fail('The system speech service did not finish within ${finishWithin.inSeconds} seconds of stopping.');
    });
    await _engine.stop();
  }

  @override
  Future<void> cancel() async {
    final capture = _capture;
    _deadline?.cancel();
    _deadline = null;
    await _engine.cancel();
    if (capture != null && !capture.isClosed) await capture.close();
    if (identical(_capture, capture)) _capture = null;
  }
}
