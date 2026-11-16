import 'dart:async';

import 'package:canoa/voice/speech_backend.dart';

/// A speech backend that listens to nothing: a test says what it hears and how loud, and counts what the app asked of it.
final class FakeSpeech implements SpeechBackend {
  FakeSpeech(
    this.id,
    this.name, {
    this.where = const SpeechWhere(place: SpeechPlace.thisDevice, leaves: AudioLeaves.no, words: 'Everything stays here.'),
    this.modelChoices = const [],
    this.languageChoices = const [],
    this.deviceChoices = const [],
    this.failWith,
  });

  @override
  final String id, name;
  @override
  final SpeechWhere where;
  final List<SpeechChoice> modelChoices, languageChoices, deviceChoices;

  /// When set, a capture ends at once with this failure.
  final SpeechFailure? failWith;

  int starts = 0, stops = 0, cancels = 0;
  String? lastDevice, lastLanguage, lastModel;
  StreamController<Transcript>? _capture;
  final _levels = StreamController<double>.broadcast();

  void hear(String text, {bool isFinal = false}) => _capture!.add(Transcript(text, isFinal: isFinal));
  void level(double value) => _levels.add(value);

  @override
  Future<List<SpeechChoice>> models() async => modelChoices;
  @override
  Future<List<SpeechChoice>> languages() async => languageChoices;
  @override
  Future<List<SpeechChoice>> devices() async => deviceChoices;
  @override
  Stream<double> get levels => _levels.stream;

  @override
  Stream<Transcript> start({String? device, String? language, String? model}) {
    starts++;
    lastDevice = device;
    lastLanguage = language;
    lastModel = model;
    final capture = _capture = StreamController<Transcript>();
    if (failWith case final failure?) {
      scheduleMicrotask(() {
        capture.addError(failure);
        capture.close();
      });
    }
    return capture.stream;
  }

  @override
  Future<void> stop() async {
    stops++;
    await _capture?.close();
  }

  @override
  Future<void> cancel() async {
    cancels++;
    await _capture?.close();
  }
}
