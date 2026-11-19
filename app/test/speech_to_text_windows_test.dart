// The Windows speech plugin's Dart side and the system speech service over it, with the plugin's channel stubbed by a stand-in that
// speaks the native side's protocol: no microphone, no recognizer.

import 'dart:async';
import 'dart:convert';

import 'package:canoa/voice/speech_backend.dart';
import 'package:canoa/voice/system_speech.dart';
import 'package:flutter/foundation.dart' show TargetPlatform, debugDefaultTargetPlatformOverride;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:speech_to_text/speech_to_text.dart';
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart' show SpeechConfigOption, SpeechToTextPlatform;
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_windows/speech_to_text_windows.dart';

const _channel = MethodChannel('speech_to_text_windows');

/// What the native side does on each call, and a way to say what it would tell Dart on its own.
final class _Native {
  final calls = <MethodCall>[];
  bool initializes = true;
  String? initializeError;
  bool listens = true;
  bool throwsOnListen = false;
  List<String> locales = const ['en-US:English (United States)', 'es-PR:Spanish (Puerto Rico)'];
  String partial = '';

  Future<Object?> handle(MethodCall call) async {
    calls.add(call);
    switch (call.method) {
      case 'hasPermission':
        return true;
      case 'initialize':
        if (initializeError != null) await tell('notifyError', jsonEncode({'errorMsg': initializeError, 'permanent': true}));
        return initializes;
      case 'listen':
        if (throwsOnListen) throw PlatformException(code: 'listen', message: 'no microphone');
        if (listens) await tell('notifyStatus', 'listening');
        return listens;
      case 'stop':
        // The recognizer finishes the phrase it holds: the final text, then the two statuses.
        if (partial.isNotEmpty) await tell('textRecognition', _result(partial, isFinal: true));
        await tell('notifyStatus', 'notListening');
        await tell('notifyStatus', 'done');
        return null;
      case 'cancel':
        return null;
      case 'locales':
        return locales;
    }
    return null;
  }

  static String _result(String words, {required bool isFinal}) => jsonEncode({
    'alternates': [
      {'recognizedWords': words, 'confidence': -1},
    ],
    'resultType': isFinal ? 2 : 0,
  });

  Future<void> hear(String words) {
    partial = words;
    return tell('textRecognition', _result(words, isFinal: false));
  }

  Future<void> tell(String method, String argument) async {
    final done = Completer<void>();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.handlePlatformMessage(
      _channel.name,
      const StandardMethodCodec().encodeMethodCall(MethodCall(method, argument)),
      (_) => done.complete(),
    );
    await done.future;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late _Native native;

  setUp(() {
    native = _Native();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(_channel, native.handle);
    SpeechToTextPlatform.instance = SpeechToTextWindows();
  });
  tearDown(() => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(_channel, null));

  SystemSpeech speech({List<SpeechConfigOption>? options}) => SystemSpeech('Windows', engine: SpeechToText.withMethodChannel(), options: options);
  Future<void> settle() => Future<void>.delayed(const Duration(milliseconds: 20));

  test('what is heard shows as it comes and the words stand as one final text when dictation stops', () async {
    final service = speech();
    final heard = <Transcript>[];
    var closed = false;
    service.start().listen(heard.add, onDone: () => closed = true);
    await settle();
    await native.hear('hello');
    await native.hear('hello world');
    await settle();
    expect([for (final t in heard) (t.text, t.isFinal)], [('hello', false), ('hello world', false)]);
    await service.stop();
    await settle();
    expect(heard.last.text, 'hello world');
    expect(heard.last.isFinal, isTrue, reason: 'the composer takes only a final text into the message');
    expect(closed, isTrue);
  });

  test('a second dictation on the same service ends with its own final text', () async {
    final service = speech();
    for (final words in ['first one', 'second one']) {
      final heard = <Transcript>[];
      var closed = false;
      native.partial = '';
      service.start().listen(heard.add, onDone: () => closed = true);
      await settle();
      await native.hear(words);
      await service.stop();
      await settle();
      expect(heard.last, isA<Transcript>().having((t) => t.text, 'text', words).having((t) => t.isFinal, 'isFinal', isTrue));
      expect(closed, isTrue, reason: 'the stream of "$words" never ended');
    }
  });

  test('the installed recognizers\' languages are the choices, and the one picked goes to the plugin', () async {
    final service = speech();
    service.start().listen((_) {});
    await settle();
    final languages = await service.languages();
    expect([for (final l in languages) l.id], ['en-US', 'es-PR']);
    expect(languages.first.label, 'English (United States)');
    await service.cancel();
    final again = speech();
    again.start(language: 'es-PR').listen((_) {});
    await settle();
    expect(native.calls.lastWhere((c) => c.method == 'listen').arguments['localeId'], 'es-PR');
    await again.cancel();
  });

  test('on Windows the languages of the installed recognizers are listed before anything is dictated', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final languages = await speech().languages();
    expect([for (final l in languages) '${l.id} ${l.label}'], ['en-US English (United States)', 'es-PR Spanish (Puerto Rico)']);
    expect(native.calls.where((c) => c.method == 'listen'), isEmpty, reason: 'listing languages must not open the microphone');
  });

  test('an option for the plugin reaches its initialize, which is how a recorded WAV replaces the microphone', () async {
    final service = speech(options: [SpeechConfigOption('windows', 'inputFile', 'C:\\recordings\\hello.wav')]);
    service.start().listen((_) {});
    await settle();
    expect(native.calls.firstWhere((c) => c.method == 'initialize').arguments['inputFile'], 'C:\\recordings\\hello.wav');
    await service.cancel();
  });

  test('a native side that cannot start listening says so at once instead of leaving the composer waiting', () async {
    native.throwsOnListen = true;
    final service = speech();
    final failures = <Object>[];
    var closed = false;
    service.start().listen((_) {}, onError: failures.add, onDone: () => closed = true);
    await settle();
    expect(closed, isTrue);
    expect(failures.single, isA<SpeechFailure>());
  });

  test('a native side that answers listen with false says so at once', () async {
    native.listens = false;
    final service = speech();
    final failures = <Object>[];
    var closed = false;
    service.start().listen((_) {}, onError: failures.add, onDone: () => closed = true);
    await settle();
    expect(closed, isTrue);
    expect(failures.single, isA<SpeechFailure>());
  });

  test('the reason the native side gives for not starting is said in words', () async {
    native.initializes = false;
    native.initializeError = 'error_no_recognizer';
    final service = speech();
    final failures = <Object>[];
    var closed = false;
    service.start().listen((_) {}, onError: failures.add, onDone: () => closed = true);
    await settle();
    expect(closed, isTrue);
    expect(failures.first.toString(), 'Windows has no speech recognizer installed.');
  });
}
