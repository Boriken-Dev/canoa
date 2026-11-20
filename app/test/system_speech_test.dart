// The operating system's speech service as dictation uses it, on a stand-in for the platform plugin: no microphone, no recognizer.

import 'package:canoa/voice/speech_backend.dart';
import 'package:canoa/voice/system_speech.dart';
import 'package:flutter/services.dart' show MissingPluginException, PlatformException;
import 'package:flutter_test/flutter_test.dart';
import 'package:speech_to_text/speech_to_text.dart';
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart';

/// A platform plugin that answers `initialize` and `listen`, and says what the test tells it to.
final class _Platform extends SpeechToTextPlatform {
  bool ready = true;
  int listens = 0, stops = 0;
  bool endOnStop = true;
  Object? initializeThrows, listenThrows;

  @override
  Future<bool> hasPermission() async => true;
  @override
  Future<bool> initialize({debugLogging = false, List<SpeechConfigOption>? options}) async {
    if (initializeThrows != null) throw initializeThrows!;
    return ready;
  }

  @override
  Future<bool> listen({String? localeId, partialResults = true, onDevice = false, int listenMode = 0, sampleRate = 0, SpeechListenOptions? options}) async {
    if (listenThrows != null) throw listenThrows!;
    listens++;
    onStatus?.call('listening');
    return true;
  }

  @override
  Future<void> stop() async {
    stops++;
    if (endOnStop) onStatus?.call('notListening');
  }

  @override
  Future<void> cancel() async {}
  @override
  Future<List<dynamic>> locales() async => const [];
}

void main() {
  late _Platform platform;
  setUp(() => SpeechToTextPlatform.instance = platform = _Platform());

  Future<void> settle() => Future<void>.delayed(const Duration(milliseconds: 20));

  test('a second dictation ends when the system says it stopped listening, as the first did', () async {
    final speech = SystemSpeech('System', engine: SpeechToText.withMethodChannel());
    for (final round in [1, 2, 3]) {
      var closed = false;
      final heard = speech.start().listen((_) {}, onDone: () => closed = true);
      await settle();
      expect(platform.listens, round);
      await speech.stop();
      await settle();
      expect(closed, isTrue, reason: 'dictation $round never saw the stream end, so the composer stays on "Transcribing…" for ever');
      await heard.cancel();
    }
  });

  test('a system with no recognizer says so in words at once', () async {
    platform.ready = false;
    final speech = SystemSpeech('System', engine: SpeechToText.withMethodChannel());
    final failures = <Object>[];
    var closed = false;
    speech.start().listen((_) {}, onError: failures.add, onDone: () => closed = true);
    await settle();
    expect(closed, isTrue);
    expect(failures.single, isA<SpeechFailure>());
  });

  test('a system that is listening but never answers is ended with its reason when the person stops', () async {
    platform.endOnStop = false;
    final speech = SystemSpeech('System', engine: SpeechToText.withMethodChannel(), finishWithin: const Duration(milliseconds: 100));
    final failures = <Object>[];
    var closed = false;
    speech.start().listen((_) {}, onError: failures.add, onDone: () => closed = true);
    await settle();
    await speech.stop();
    expect(closed, isFalse);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    expect(closed, isTrue, reason: 'a service that never says it is done must not hold the composer');
    expect(failures.single, isA<SpeechFailure>());
    expect((failures.single as SpeechFailure).message, contains('did not finish'));
  });

  Future<String> failureOf(SpeechBackend speech) async {
    final failures = <Object>[];
    speech.start().listen((_) {}, onError: failures.add);
    await settle();
    return (failures.single as SpeechFailure).message;
  }

  test('a platform with no plugin for the service says so in words, not by the name of the exception', () async {
    platform.initializeThrows = MissingPluginException('No implementation found for method initialize');
    final message = await failureOf(SystemSpeech('System', engine: SpeechToText.withMethodChannel()));
    expect(message, 'The speech service could not start: the speech plugin of this system was not loaded when Canoa started.');
    expect(message, isNot(contains('Exception')));
  });

  test('a platform error that stops the listen gives its own message', () async {
    platform.listenThrows = PlatformException(code: 'listen', message: 'No microphone is connected');
    final message = await failureOf(SystemSpeech('System', engine: SpeechToText.withMethodChannel()));
    expect(message, 'The speech service could not start: No microphone is connected.');
  });
}
