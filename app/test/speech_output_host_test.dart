// The app's text-to-speech code against the real host's `@control`: `store_control`, an example of canoa-host, serves the control channel
// over standard streams with the canoa.cfg in its folder, and `host_say_standin` is the program a provider of that file runs. Text is
// spoken through it and its audio comes back in order; the choice of voice reaches the program; a provider's settings are written to
// canoa.cfg and read back without the key. Build it first: `cargo build -p canoa-host --bin canoa --examples`. Skipped without it.
@TestOn('vm')
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:canoa/voice/host_speech_output.dart';
import 'package:canoa/voice/speech_backend.dart' show SpeechFailure;
import 'package:canoa/voice/speech_output.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/control_process.dart';
import 'fixtures/examples.dart';

void main() {
  final skip = exampleMissing('store_control') ?? exampleMissing('host_say_standin') ?? exampleMissing('stdio_agent') ?? exampleMissing('version_standin');

  group('the real host', skip: skip, () {
    late Directory dir;
    late ControlProcess example;
    late HostControl control;
    late HostVoiceCatalog catalog;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-say-host');
      final program = exampleBinary('host_say_standin')!.replaceAll(r'\', '/');
      await File('${dir.path}/canoa.cfg').writeAsString(
        '[say.says]\nprovider = "speech_say_command"\nprogram = "$program"\nargs = ["lines"]\nvoices = ["ana", "luis"]\nmodels = ["small", "big"]\n'
        '\n[say.env]\nprovider = "speech_say_command"\nprogram = "$program"\nargs = ["env"]\nvoices = ["ana", "luis"]\nmodels = ["small", "big"]\nstyles = ["calm"]\nspeed = true\n',
      );
      example = ControlProcess(await Process.start(exampleBinary('store_control')!, [dir.path]));
      control = HostControl.dialing(() async => example.open());
      // The voice side shares the speech side's call, as the app wires it.
      catalog = HostVoiceCatalog(HostSpeechCatalog(control).call);
    });

    tearDown(() async {
      await control.close();
      await example.process.stdin.close();
      await example.process.exitCode.timeout(const Duration(seconds: 20), onTimeout: () {
        example.process.kill();
        return -1;
      });
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may hold a file a moment longer.
      }
    });

    SpeechOutput says() => catalog.outputs.firstWhere((o) => o.id == 'host:says');

    Future<String> spoken(SpeechUtterance utterance) async => [for (final chunk in await utterance.audio.toList()) utf8.decode(chunk)].join();

    test('lists the providers canoa.cfg defines with their voices and models, and the kinds with their forms', () async {
      await catalog.refresh();
      expect(catalog.problem, isNull);
      expect([for (final kind in catalog.types) kind.id], ['speech_say_system', 'speech_say_piper', 'speech_say_endpoint', 'speech_say_command']);
      expect(catalog.types.last.form['sections'], isA<List<Object?>>());
      final output = says();
      expect(await output.voices(), hasLength(2));
      expect((await output.voices()).map((v) => v.id), ['ana', 'luis']);
      expect(output.models.map((m) => m.id), ['small', 'big']);
      expect(output.where.leaves, TextLeaves.maybe);
      // The operating system's voices are offered with no table, and say that the text stays.
      final system = catalog.providers.firstWhere((p) => p.name == 'system');
      expect(system.builtin, isTrue);
      expect(system.where.leaves, TextLeaves.no);
    });

    test('text spoken through the provider comes back as its audio, in order', () async {
      await catalog.refresh();
      final utterance = says().speak('Good morning. Good night.', choices: const SpeakChoices(voice: 'luis'));
      expect(await utterance.format, const PcmFormat(rate: 22050));
      expect(await spoken(utterance), '[Good morning.][Good night.]');
      await utterance.done;
    });

    test('the voice, model, style, speed and language chosen reach the program', () async {
      await catalog.refresh();
      final env = catalog.outputs.firstWhere((o) => o.id == 'host:env');
      final told = await spoken(env.speak('Hi.', choices: const SpeakChoices(voice: 'luis', model: 'big', style: 'calm', speed: 1.5, language: 'es-PR')));
      expect(told, 'VOICE=luis;MODEL=big;STYLE=calm;SPEED=1.50;LANGUAGE=es-PR');
      final refused = says().speak('Hi.', choices: const SpeakChoices(voice: 'zed'));
      await expectLater(refused.done, throwsA(isA<SpeechFailure>().having((f) => f.message, 'message', contains('ana, luis'))));
    });

    test('stopping an utterance ends it at the host', () async {
      await catalog.refresh();
      final utterance = says().speak('One. Two. Three.');
      await utterance.format;
      await utterance.stop();
      await utterance.done;
      expect(await utterance.audio.toList(), isNotNull);
    });

    test('a provider written from the form is in canoa.cfg, listed without its key, and keeps the key when saved again', () async {
      await catalog.refresh();
      await catalog.save('cloud', 'speech_say_endpoint', {
        'url': 'https://api.openai.com/v1/audio/speech',
        'model': 'tts-1',
        'api_key': 'sk-test-0123456789',
      });
      var text = await File('${dir.path}/canoa.cfg').readAsString();
      expect(text, contains('[say.cloud]'));
      expect(text, contains('sk-test-0123456789'));
      final cloud = catalog.providers.firstWhere((p) => p.name == 'cloud');
      expect(cloud.settings['model'], 'tts-1');
      expect(cloud.settings.containsKey('api_key'), isFalse);
      expect(cloud.secretsSet, {'api_key'});
      // OpenAI's documented models, voices and styles are listed, and the text is said to go to its host with the key.
      expect(cloud.models.map((m) => m.id), ['gpt-4o-mini-tts', 'tts-1', 'tts-1-hd']);
      expect(cloud.voices, hasLength(13));
      expect(cloud.voices.firstWhere((v) => v.id == 'marin').models, ['gpt-4o-mini-tts']);
      expect(cloud.where.leaves, TextLeaves.yes);
      expect(cloud.where.words, contains('api.openai.com'));
      expect(cloud.where.words, contains('your key'));

      // The form never shows the key, so it sends none, and the host keeps the one it has.
      await catalog.save('cloud', 'speech_say_endpoint', {'url': 'https://api.openai.com/v1/audio/speech', 'model': 'gpt-4o-mini-tts'});
      text = await File('${dir.path}/canoa.cfg').readAsString();
      expect(text, contains('gpt-4o-mini-tts'));
      expect(text, contains('sk-test-0123456789'));
      await catalog.remove('cloud');
      expect(catalog.providers.map((p) => p.name), isNot(contains('cloud')));
      expect(await File('${dir.path}/canoa.cfg').readAsString(), isNot(contains('sk-test')));
      await expectLater(catalog.save('x', 'speech_say_command', {'nonsense': 1}), throwsA(isA<SpeechFailure>()));
    });
  });
}
