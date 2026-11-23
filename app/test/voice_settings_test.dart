// Settings > Voice: where speech is turned into text is chosen and stated before anything is recorded, a test of the
// microphone shows a level, and [voice] in canoa.cfg is read and written back.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_speech.dart';
import 'fixtures/settings_fixture.dart';

FakeSpeech local() => FakeSpeech(
  'system',
  'Local recognizer',
  modelChoices: const [SpeechChoice('tiny', 'Tiny'), SpeechChoice('small', 'Small')],
  languageChoices: const [SpeechChoice('en-US', 'English (US)'), SpeechChoice('es-PR', 'Espanol')],
  deviceChoices: const [SpeechChoice('usb', 'USB microphone')],
);

FakeSpeech hosted() => FakeSpeech(
  'hosted',
  'Transcription service',
  where: const SpeechWhere(
    place: SpeechPlace.cloud,
    leaves: AudioLeaves.yes,
    words: 'The audio goes to the service for transcription.',
    endpoint: 'speech.example.invalid, account work',
  ),
);

void main() {
  group('[voice] in canoa.cfg', () {
    test('absent, it is the operating system service, automatic, click to toggle, and writes nothing', () {
      final config = CanoaConfig.parse('');
      expect(config.voice.backend, 'system');
      expect(config.voice.language, isNull);
      expect(config.voice.mode, DictationMode.toggle);
      expect(config.encode(), isNot(contains('[voice]')));
    });

    test('every key is read and written back', () {
      final config = CanoaConfig.parse('[voice]\nbackend = "x"\nmodel = "m"\nlanguage = "es-PR"\ndevice = "usb"\nmode = "hold"\n');
      expect(config.voice.mode, DictationMode.hold);
      final back = CanoaConfig.parse(config.encode()).voice;
      expect((back.backend, back.model, back.language, back.device, back.mode), ('x', 'm', 'es-PR', 'usb', DictationMode.hold));
    });

    test('the services whose notice was accepted are kept, sorted, and written only when there are some', () {
      final config = CanoaConfig.parse('[voice]\nacknowledged = ["system", "host"]\n');
      expect(config.voice.acknowledged, {'system', 'host'});
      expect(config.encode(), contains('acknowledged = ["host","system"]'));
      expect(CanoaConfig.parse(config.encode()).voice.acknowledged, {'system', 'host'});
      expect(CanoaConfig.parse('').encode(), isNot(contains('acknowledged')));
      expect(() => CanoaConfig.parse('[voice]\nacknowledged = "system"\n'), throwsFormatException);
    });

    test('a mode that is neither toggle nor hold is refused, and a key it does not have is kept', () {
      expect(() => CanoaConfig.parse('[voice]\nmode = "always"\n'), throwsFormatException);
      expect(CanoaConfig.parse('[voice]\nmicrophone = "on"\n').encode(), allOf(contains('microphone'), contains('on')));
    });
  });

  group('the Voice page', () {
    testWidgets('lists the service, its models and where it is processed, and the microphone is off', (tester) async {
      final system = local();
      await openSettings(tester, section: 'voice', voiceBackends: [system, hosted()]);
      expect(find.byKey(const Key('voice-where')), findsOneWidget);
      expect(find.textContaining('This device. Everything stays here.'), findsOneWidget);
      expect(find.textContaining('The audio stays on this device. Canoa keeps no recording'), findsOneWidget);
      expect(find.byKey(const Key('voice-model')), findsOneWidget);
      await tester.tap(find.byKey(const Key('voice-model')));
      await tester.pumpAndSettle();
      expect(find.text('Tiny'), findsWidgets);
      expect(find.text('Small'), findsWidgets);
      expect(system.starts, 0);
    });

    testWidgets('switching the service changes the destination it states, and starts no capture', (tester) async {
      final system = local(), cloud = hosted();
      final h = await openSettings(tester, section: 'voice', voiceBackends: [system, cloud]);
      await choose(tester, 'voice-backend', 'Transcription service');
      expect(h.last.voice.backend, 'hosted');
      expect(find.textContaining('A named cloud service. The audio goes to the service'), findsOneWidget);
      expect(find.textContaining('The audio leaves this device for the service above'), findsOneWidget);
      expect(find.byKey(const Key('voice-endpoint')), findsOneWidget);
      expect(find.text('speech.example.invalid, account work'), findsOneWidget);
      expect(find.byKey(const Key('voice-model')), findsNothing);
      expect(system.starts + cloud.starts, 0);
    });

    testWidgets('language, device and how dictation is held are written to [voice]', (tester) async {
      final h = await openSettings(tester, section: 'voice', voiceBackends: [local()]);
      await choose(tester, 'voice-language', 'Espanol');
      await choose(tester, 'voice-device', 'USB microphone');
      await choose(tester, 'voice-mode', 'Push to talk');
      expect((h.last.voice.language, h.last.voice.device, h.last.voice.mode), ('es-PR', 'usb', DictationMode.hold));
      expect(h.last.encode(), contains('[voice]'));
    });

    testWidgets('the person\'s own voice profile is written to [voice], and cleared when its fields are emptied', (tester) async {
      final system = local();
      final h = await openSettings(tester, section: 'voice', voiceBackends: [system]);
      await choose(tester, 'voice-profile-voice', 'female');
      await tester.enterText(find.byKey(const Key('voice-profile-accent')), 'Puerto Rican Spanish');
      await tester.enterText(find.byKey(const Key('voice-profile-vocabulary')), 'Canoa, Boriken ,, ');
      await tester.pump();
      expect(h.last.voice.profile, const VoiceProfile(accent: 'Puerto Rican Spanish', voice: 'female', vocabulary: ['Canoa', 'Boriken']));
      expect(h.last.encode(), allOf(contains('voice = "female"'), contains('accent = "Puerto Rican Spanish"'), contains('vocabulary = ["Canoa","Boriken"]')));
      expect(system.starts, 0, reason: 'writing a profile opens no microphone');
      await tester.enterText(find.byKey(const Key('voice-profile-accent')), '');
      await tester.enterText(find.byKey(const Key('voice-profile-vocabulary')), '');
      await choose(tester, 'voice-profile-voice', 'Not said');
      expect(h.last.voice.profile.isEmpty, isTrue);
      expect(h.last.encode(), isNot(contains('[voice]')));
    });

    testWidgets('the test opens the microphone only when pressed, shows a level and what was heard, and stops', (tester) async {
      final system = local();
      await openSettings(tester, section: 'voice', voiceBackends: [system]);
      expect(system.starts, 0);
      LinearProgressIndicator bar() => tester.widget(find.byKey(const Key('voice-level')));
      expect(bar().value, 0);
      await tester.tap(find.byKey(const Key('voice-test')));
      await tester.pump();
      expect(system.starts, 1);
      system.level(0.6);
      system.hear('hello there');
      await tester.pump();
      expect(bar().value, 0.6);
      expect(find.text('hello there'), findsOneWidget);
      await tester.tap(find.byKey(const Key('voice-test-stop')));
      await tester.pumpAndSettle();
      expect(system.stops, 1);
      expect(find.byKey(const Key('voice-test')), findsOneWidget);
      expect(bar().value, 0);
    });

    testWidgets('a service that fails says so and no other service is tried', (tester) async {
      final broken = FakeSpeech('system', 'Broken', failWith: const SpeechFailure('The service could not be reached.'));
      final spare = hosted();
      await openSettings(tester, section: 'voice', voiceBackends: [broken, spare]);
      await tester.tap(find.byKey(const Key('voice-test')));
      await tester.pumpAndSettle();
      expect(find.text('The service could not be reached.'), findsOneWidget);
      expect(find.byKey(const Key('voice-test')), findsOneWidget);
      expect((broken.starts, spare.starts), (1, 0));
    });

    testWidgets('leaving the page while it listens ends the capture', (tester) async {
      final system = local();
      await openSettings(tester, section: 'voice', voiceBackends: [system]);
      await tester.tap(find.byKey(const Key('voice-test')));
      await tester.pump();
      await goTo(tester, 'general');
      expect(system.cancels, 1);
    });

    testWidgets('a service the file names that this build lacks is said, and nothing is sent', (tester) async {
      final h = await openSettings(
        tester,
        config: CanoaConfig.parse('[voice]\nbackend = "gone"\n'),
        section: 'voice',
        voiceBackends: [local()],
      );
      expect(find.text('Not available here'), findsOneWidget);
      expect(find.byKey(const Key('voice-test')), findsNothing);
      expect(h.applied, isEmpty);
    });

    testWidgets('with no speech service it reads "No speech service here" and offers the setup', (tester) async {
      await openSettings(tester, section: 'voice');
      expect(find.text('No speech service here'), findsOneWidget);
      expect(find.byKey(const Key('voice-test')), findsNothing);
      await tester.tap(find.byKey(const Key('voice-setup')));
      await tester.pumpAndSettle();
      expect(find.text('Set up dictation'), findsOneWidget);
    });
  });
}
