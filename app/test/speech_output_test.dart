// The app's text-to-speech code against a stand-in host: the providers the host lists with their voices, models, styles and where the
// text goes; an utterance whose audio comes back in pieces and in order; the choices that reach the provider; stopping; a failure;
// and the browser's voices, the ones on the device and the ones that are not.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/voice/browser_voices.dart';
import 'package:canoa/voice/host_speech_output.dart';
import 'package:canoa/voice/speech_backend.dart' show SpeechFailure;
import 'package:canoa/voice/speech_output.dart';
import 'package:flutter_test/flutter_test.dart';

/// A host that answers `speech/voices` with [listing], and speaks what [pieces] hold, one `speech/audio` answer at a time.
final class _Host {
  _Host({List<Map<String, Object?>>? providers, this.pieces = const []}) : listing = providers ?? [_provider()];

  final List<Map<String, Object?>> listing;
  List<Map<String, Object?>> pieces;
  final calls = <(String, Map<String, Object?>)>[];
  int next = 0;
  Object? sayFails;

  Future<Map<String, Object?>> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'speech/voices':
        return {
          'types': [
            {'id': 'speech_say_command', 'title': 'Command', 'description': 'Runs a program', 'schema': {'properties': {}}, 'form': {'sections': []}},
          ],
          'providers': listing,
        };
      case 'speech/say':
        final failure = sayFails;
        if (failure != null) throw failure;
        return {'utterance': 'say-1', 'encoding': 'pcm_s16le', 'rate': 22050, 'channels': 1};
      case 'speech/audio':
        if (next >= pieces.length) return {'audio': '', 'done': true};
        return pieces[next++];
      case 'speech/stop':
        return {};
      case 'speech/say_configure':
        return {'saved': true};
    }
    throw SpeechFailure('no method $method');
  }

  Iterable<Map<String, Object?>> sent(String method) => [
    for (final (name, params) in calls)
      if (name == method) params,
  ];
}

Map<String, Object?> _provider({String name = 'standin', String? problem, String leaves = 'maybe'}) => {
  'name': name,
  'provider': 'speech_say_command',
  'builtin': false,
  'title': 'Command',
  'description': 'Runs a program that reads text and writes audio',
  'settings': {'program': 'say-it'},
  'secrets_set': ['api_key'],
  'encodings': ['pcm_s16le'],
  'models': [
    {'id': 'small', 'label': 'Small'},
    {'id': 'big', 'label': 'Big', 'description': 'Slower'},
  ],
  'default_model': 'small',
  'styles': [
    {'id': 'calm', 'label': 'calm'},
  ],
  'speed': {'min': 0.5, 'max': 2.0},
  'uses': ['voice', 'model', 'style', 'speed'],
  'leaves': leaves,
  'endpoint': 'https://tts.example.org/v1/audio/speech',
  'words': 'The text goes to the program you named.',
  'max_chars': 4096,
  'voices': [
    {'id': 'ana', 'name': 'Ana', 'language': 'es-MX', 'gender': 'female'},
    {'id': 'luis', 'name': 'Luis', 'language': 'en-US', 'models': ['big']},
    {'id': 'any', 'name': 'Any'},
  ],
  'problem': ?problem,
};

Map<String, Object?> _piece(String text, {bool done = false, String? error}) => {'audio': base64Encode(utf8.encode(text)), 'done': done, 'error': ?error};

Future<List<String>> _heard(SpeechUtterance utterance) async => [for (final chunk in await utterance.audio.toList()) utf8.decode(chunk)];

void main() {
  group('the host\'s voice providers', () {
    test('are listed with their voices, models, styles, speeds and where the text goes', () async {
      final host = _Host(providers: [_provider(), _provider(name: 'broken', problem: 'it names no provider')]);
      final catalog = HostVoiceCatalog(host.call);
      await catalog.refresh();
      expect(catalog.problem, isNull);
      expect([for (final kind in catalog.types) kind.id], ['speech_say_command']);
      expect(catalog.providers, hasLength(2));
      // A provider that cannot be built is listed with its reason and is no way of speaking.
      expect(catalog.providers.last.problem, 'it names no provider');
      final output = catalog.outputs.single;
      expect(output.id, 'host:standin');
      expect(output.models.map((m) => m.id), ['small', 'big']);
      expect(output.models.last.description, 'Slower');
      expect(output.defaultModel, 'small');
      expect(output.styles.single.id, 'calm');
      expect(output.speed!.clamp(9), 2);
      expect(output.speed!.clamp(0.1), 0.5);
      expect(output.uses, {'voice', 'model', 'style', 'speed'});
      expect(output.where.leaves, TextLeaves.maybe);
      expect(output.where.endpoint, 'https://tts.example.org/v1/audio/speech');
      expect(output.where.words, contains('program you named'));
      final provider = catalog.providers.first;
      expect(provider.secretsSet, {'api_key'});
      expect(provider.settings['program'], 'say-it');
      expect(provider.maxChars, 4096);
      final voices = await output.voices();
      expect(voices.map((v) => v.id), ['ana', 'luis', 'any']);
      expect(voices[1].worksWith('big'), isTrue);
      expect(voices[1].worksWith('small'), isFalse);
      expect(voices[2].worksWith('small'), isTrue, reason: 'a voice that lists no model works with every one');
      // By language: the first part of the tag decides, and a voice with no language is kept.
      expect((await output.voices(language: 'es')).map((v) => v.id), ['ana', 'any']);
      expect((await output.voices(language: 'en_GB')).map((v) => v.id), ['luis', 'any']);
    });

    test('are not listed when the host refuses, and the reason is kept', () async {
      final catalog = HostVoiceCatalog((method, params) async => throw const SpeechFailure('this key has no voice grant'));
      await catalog.refresh();
      expect(catalog.problem, 'this key has no voice grant');
      expect(catalog.outputs, isEmpty);
      expect(catalog.loaded, isTrue);
    });

    test('are written and removed through the host and read again', () async {
      final host = _Host();
      final catalog = HostVoiceCatalog(host.call);
      await catalog.save('mine', 'speech_say_command', {'program': 'x'}, clear: ['api_key']);
      expect(host.sent('speech/say_configure').single, {
        'name': 'mine',
        'provider': 'speech_say_command',
        'settings': {'program': 'x'},
        'clear': ['api_key'],
      });
      await catalog.remove('mine');
      expect(host.sent('speech/say_configure').last, {'name': 'mine', 'remove': true});
      expect(host.sent('speech/voices'), hasLength(2), reason: 'each write is followed by a read');
    });
  });

  group('an utterance through the host', () {
    Future<(_Host, SpeechOutput)> setUp(List<Map<String, Object?>> pieces) async {
      final host = _Host(pieces: pieces);
      final catalog = HostVoiceCatalog(host.call);
      await catalog.refresh();
      return (host, catalog.outputs.single);
    }

    test('delivers its audio in pieces and in order, in the format the host says', () async {
      final (host, output) = await setUp([_piece('[One.]'), {'audio': '', 'done': false}, _piece('[Two.]'), _piece('[Three.]', done: true)]);
      final utterance = output.speak('One. Two. Three.');
      expect(await utterance.format, const PcmFormat(rate: 22050));
      expect(await _heard(utterance), ['[One.]', '[Two.]', '[Three.]']);
      await utterance.done;
      // It is over at the host, so nothing is stopped there.
      expect(host.sent('speech/stop'), isEmpty);
      expect(host.sent('speech/audio').first, {'utterance': 'say-1', 'wait_ms': 1000, 'max_bytes': 65536});
    });

    test('carries the voice, model, style, speed and language the person chose, and leaves out what they did not', () async {
      final (host, output) = await setUp([_piece('x', done: true)]);
      final utterance = output.speak('Hola.', choices: const SpeakChoices(voice: 'ana', model: 'big', style: 'calm', speed: 1.5, language: 'es-PR'));
      await utterance.audio.drain<void>();
      expect(host.sent('speech/say').single, {
        'provider': 'standin',
        'text': 'Hola.',
        'voice': 'ana',
        'model': 'big',
        'style': 'calm',
        'speed': 1.5,
        'language': 'es-PR',
      });
      final (other, plain) = await setUp([_piece('x', done: true)]);
      await plain.speak('Hi.').audio.drain<void>();
      expect(other.sent('speech/say').single, {'provider': 'standin', 'text': 'Hi.'});
    });

    test('stops at the host when told to, drops what was not spoken and completes without a failure', () async {
      final (host, output) = await setUp([_piece('one'), _piece('two'), _piece('three')]);
      final utterance = output.speak('One two three.');
      final first = Completer<void>();
      final heard = <String>[];
      final listen = utterance.audio.listen((chunk) {
        heard.add(utf8.decode(chunk));
        if (!first.isCompleted) first.complete();
      });
      await first.future;
      await utterance.stop();
      await utterance.done;
      await listen.cancel();
      expect(heard, isNotEmpty);
      expect(host.sent('speech/stop').single, {'utterance': 'say-1'});
    });

    test('says why it could not start, to the format and to done', () async {
      final (host, output) = await setUp(const []);
      host.sayFails = const SpeechFailure('this provider has no voice "zed"; it has ana, luis');
      final utterance = output.speak('Hi.', choices: const SpeakChoices(voice: 'zed'));
      await expectLater(utterance.format, throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('ana, luis'))));
      await expectLater(utterance.done, throwsA(isA<SpeechFailure>()));
      await expectLater(utterance.audio.toList(), throwsA(isA<SpeechFailure>()));
    });

    test('delivers the audio a failed utterance made and then its reason', () async {
      final (host, output) = await setUp([_piece('[One.]'), _piece('', done: true, error: 'the program exited with 3: boom')]);
      final utterance = output.speak('One. Two.');
      final heard = <String>[];
      Object? failure;
      final ended = Completer<void>();
      utterance.audio.listen((chunk) => heard.add(utf8.decode(chunk)), onError: (Object error) => failure = error, onDone: ended.complete);
      await ended.future;
      expect(heard, ['[One.]']);
      expect(failure, isA<SpeechFailure>().having((f) => f.message, 'message', contains('boom')));
      await expectLater(utterance.done, throwsA(isA<SpeechFailure>()));
      expect(host.sent('speech/stop'), isEmpty, reason: 'the host ended it itself');
    });

    test('is held back while the player is behind', () async {
      final (host, output) = await setUp([for (var i = 0; i < 6; i++) _piece('p$i'), _piece('', done: true)]);
      final utterance = output.speak('Long text.');
      final heard = <String>[];
      late StreamSubscription<Uint8List> listen;
      listen = utterance.audio.listen((chunk) {
        heard.add(utf8.decode(chunk));
        if (heard.length == 1) listen.pause();
      });
      await Future<void>.delayed(const Duration(milliseconds: 300));
      // Paused after the first piece: no more were asked for than one call in flight.
      expect(host.sent('speech/audio').length, lessThanOrEqualTo(2), reason: '${host.sent('speech/audio').length}');
      listen.resume();
      await utterance.done;
      expect(heard, ['p0', 'p1', 'p2', 'p3', 'p4', 'p5']);
    });
  });

  group('the browser\'s voices', () {
    final voices = const [
      BrowserVoice(uri: 'local-en', name: 'Alex', lang: 'en-US', localService: true, isDefault: true),
      BrowserVoice(uri: 'local-es', name: 'Paulina', lang: 'es_MX', localService: true),
      BrowserVoice(uri: 'online-en', name: 'Google US English', lang: 'en-US', localService: false),
    ];

    test('the ones on the device are one way of speaking whose text stays, and the online ones another that says where it goes', () async {
      final synthesis = _FakeSynthesis(voices);
      final outputs = browserSpeechOutputs(synthesis: synthesis);
      expect(outputs.map((o) => o.id), ['browser', 'browser:online']);
      expect(outputs[0].where.leaves, TextLeaves.no);
      expect(outputs[0].where.words, contains('sent nowhere'));
      expect(outputs[1].where.leaves, TextLeaves.yes);
      expect(outputs[1].where.words, contains("browser's maker"));
      expect((await outputs[0].voices()).map((v) => v.id), ['local-en', 'local-es']);
      expect((await outputs[0].voices(language: 'es')).single.language, 'es-MX');
      expect((await outputs[1].voices()).map((v) => v.id), ['online-en']);
      expect(outputs[0].uses, {'voice', 'speed', 'language'});
      expect(outputs[0].models, isEmpty);
    });

    test('speak with the chosen voice and speed, play by themselves and finish', () async {
      final synthesis = _FakeSynthesis(voices);
      final output = browserSpeechOutputs(synthesis: synthesis).first;
      final utterance = output.speak('Hola.', choices: const SpeakChoices(voice: 'local-es', speed: 9));
      expect(await utterance.format, isNull, reason: 'the browser plays it itself');
      expect(await utterance.audio.toList(), isEmpty);
      await synthesis.untilSpoken;
      synthesis.spoken.single.finish();
      await utterance.done;
      final said = synthesis.spoken.single;
      expect((said.text, said.voiceUri, said.rate), ('Hola.', 'local-es', 2.0), reason: 'a speed is held to what is taken');
    });

    test('pick a voice by the language asked when none is named, and refuse one the device does not have', () async {
      final synthesis = _FakeSynthesis(voices);
      final output = browserSpeechOutputs(synthesis: synthesis).first;
      final utterance = output.speak('Hola.', choices: const SpeakChoices(language: 'es-PR'));
      await synthesis.untilSpoken;
      expect(synthesis.spoken.single.voiceUri, 'local-es');
      synthesis.spoken.single.finish();
      await utterance.done;

      final refused = output.speak('Hi.', choices: const SpeakChoices(voice: 'online-en'));
      await expectLater(refused.done, throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('online voices'))));
      expect(synthesis.spoken, hasLength(1), reason: 'nothing was said with a voice the person did not choose to send text to');
    });

    test('stop ends what the browser is saying', () async {
      final synthesis = _FakeSynthesis(voices);
      final output = browserSpeechOutputs(synthesis: synthesis).first;
      final utterance = output.speak('A long text.');
      await synthesis.untilSpoken;
      await utterance.stop();
      await utterance.done;
      expect(synthesis.spoken.single.cancelled, isTrue);
    });

    test('a browser with no voice on the device says so', () async {
      final output = browserSpeechOutputs(synthesis: _FakeSynthesis(const [])).first;
      await expectLater(output.speak('Hi.').done, throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('no voices on this device'))));
    });

    test('there are none where the page has no speech synthesis', () {
      expect(browserSpeechOutputs(), isEmpty, reason: 'the test runs outside a browser');
    });
  });
}

final class _Said implements BrowserSpeaking {
  _Said(this.text, this.voiceUri, this.rate);
  final String text;
  final String? voiceUri;
  final double rate;
  final _done = Completer<void>();
  bool cancelled = false;

  void finish() => _done.complete();

  @override
  Future<void> get done => _done.future;

  @override
  void cancel() {
    cancelled = true;
    if (!_done.isCompleted) _done.complete();
  }
}

final class _FakeSynthesis implements BrowserSynthesis {
  _FakeSynthesis(this._voices);
  final List<BrowserVoice> _voices;
  final spoken = <_Said>[];
  final Completer<void> _heard = Completer<void>();

  Future<void> get untilSpoken => spoken.isNotEmpty ? Future.value() : _heard.future;

  @override
  Future<List<BrowserVoice>> voices() async => _voices;

  @override
  BrowserSpeaking speak(String text, {String? voiceUri, String? lang, double rate = 1}) {
    final said = _Said(text, voiceUri, rate);
    spoken.add(said);
    if (!_heard.isCompleted) _heard.complete();
    return said;
  }
}
