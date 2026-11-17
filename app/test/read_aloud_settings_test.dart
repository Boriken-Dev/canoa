// Settings > Voice, the part that reads replies aloud: the provider, voice, model, style and speed are chosen and written to
// [voice] in canoa.cfg, where the text goes is said before anything is spoken, and a sample can be played.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/voice/outputs.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_output.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_voice.dart';
import 'fixtures/settings_fixture.dart';

FakeOutput _local() => FakeOutput(
  'browser',
  "The browser's own voices",
  uses: const {'voice', 'speed'},
  speed: const SpeedRange(0.5, 2),
  voiceList: const [OutputVoice(id: 'v1', name: 'Zira', language: 'en-US'), OutputVoice(id: 'v2', name: 'Sabina', language: 'es-MX')],
);

FakeOutput _cloud() => FakeOutput(
  'host:openai',
  'openai (Endpoint)',
  where: const OutputWhere(leaves: TextLeaves.yes, words: 'The text goes to the address below.', endpoint: 'https://api.example.org/v1/audio/speech'),
  uses: const {'voice', 'model', 'style', 'speed'},
  models: const [OutputChoice('tts-small', 'Small'), OutputChoice('tts-big', 'Big')],
  defaultModel: 'tts-small',
  styles: const [OutputChoice('calm', 'Calm'), OutputChoice('bright', 'Bright')],
  speed: const SpeedRange(0.25, 4),
  voiceList: const [OutputVoice(id: 'alloy', name: 'Alloy'), OutputVoice(id: 'nova', name: 'Nova')],
);

void main() {
  group('[voice] in canoa.cfg, the reading keys', () {
    test('absent, nothing is chosen and nothing is written', () {
      final speak = CanoaConfig.parse('').voice.speak;
      expect((speak.output, speak.voice, speak.model, speak.style, speak.speed, speak.readReplies), (null, null, null, null, null, false));
      expect(speak.isDefault, isTrue);
      expect(CanoaConfig.parse('').encode(), isNot(contains('[voice]')));
    });

    test('every key is read and written back', () {
      final config = CanoaConfig.parse(
        '[voice]\noutput = "host:openai"\noutput_voice = "nova"\noutput_model = "tts-big"\noutput_style = "calm"\noutput_speed = 1.25\nread_replies = true\n',
      );
      final back = CanoaConfig.parse(config.encode()).voice.speak;
      expect((back.output, back.voice, back.model, back.style, back.speed, back.readReplies), ('host:openai', 'nova', 'tts-big', 'calm', 1.25, true));
      // Dictation's own keys are not disturbed by them.
      expect(CanoaConfig.parse(config.encode()).voice.backend, 'system');
    });

    test('a speed that is not a sensible number, and a key that is not text, are refused', () {
      expect(() => CanoaConfig.parse('[voice]\noutput_speed = 0\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[voice]\noutput_speed = "fast"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[voice]\noutput = 3\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[voice]\nread_replies = "yes"\n'), throwsFormatException);
    });
  });

  group('the Read replies aloud group', () {
    testWidgets('with no voice at all, it says so and what would give one', (tester) async {
      await openSettings(tester, section: 'voice', voiceOutputs: VoiceOutputs());
      expect(find.byKey(const Key('speak-none')), findsOneWidget);
      expect(find.byKey(const Key('speak-output')), findsNothing);
    });

    testWidgets('provider, voice, model, style and speed are chosen and written, and a provider that lacks one does not offer it', (tester) async {
      final outputs = VoiceOutputs(browser: [_local(), _cloud()]);
      final h = await openSettings(tester, section: 'voice', voiceOutputs: outputs);
      // Automatic is this page's own voices, which have a voice and a speed, and no model or style.
      expect(find.byKey(const Key('speak-voice')), findsOneWidget);
      expect(find.byKey(const Key('speak-speed')), findsOneWidget);
      expect(find.byKey(const Key('speak-model')), findsNothing);
      expect(find.byKey(const Key('speak-style')), findsNothing);
      expect(find.textContaining('It stays on this device or your host.'), findsOneWidget);

      await choose(tester, 'speak-output', 'openai (Endpoint)');
      expect(h.last.voice.speak.output, 'host:openai');
      expect(find.byKey(const Key('speak-model')), findsOneWidget);
      expect(find.byKey(const Key('speak-style')), findsOneWidget);
      expect(find.textContaining('It leaves this device for the provider.'), findsOneWidget);
      expect(find.text('https://api.example.org/v1/audio/speech'), findsOneWidget);

      await choose(tester, 'speak-voice', 'Nova');
      await choose(tester, 'speak-model', 'Big');
      await choose(tester, 'speak-style', 'Calm');
      await choose(tester, 'speak-speed', '1.5x');
      final speak = h.last.voice.speak;
      expect((speak.output, speak.voice, speak.model, speak.style, speak.speed), ('host:openai', 'nova', 'tts-big', 'calm', 1.5));
      expect(
        h.last.encode(),
        allOf(
          contains('output = "host:openai"'),
          contains('output_voice = "nova"'),
          contains('output_model = "tts-big"'),
          contains('output_style = "calm"'),
          contains('output_speed = 1.5'),
        ),
      );

      // Back to the provider's own choice, and to Normal speed, takes the keys out.
      await choose(tester, 'speak-voice', 'Provider default');
      await choose(tester, 'speak-speed', 'Normal');
      expect((h.last.voice.speak.voice, h.last.voice.speak.speed), (null, null));
      expect(h.last.encode(), isNot(contains('output_voice')));
      expect(h.last.encode(), isNot(contains('output_speed')));
    });

    testWidgets('changing the provider clears the voice, model and style of the one before, and keeps the speed', (tester) async {
      final config = CanoaConfig.parse(
        '[voice]\noutput = "host:openai"\noutput_voice = "nova"\noutput_model = "tts-big"\noutput_style = "calm"\noutput_speed = 1.5\n',
      );
      final h = await openSettings(tester, section: 'voice', config: config, voiceOutputs: VoiceOutputs(browser: [_local(), _cloud()]));
      await choose(tester, 'speak-output', "The browser's own voices");
      expect((h.last.voice.speak.output, h.last.voice.speak.voice, h.last.voice.speak.model, h.last.voice.speak.style, h.last.voice.speak.speed), ('browser', null, null, null, 1.5));
    });

    testWidgets('a provider named in canoa.cfg that is not here is said, and not replaced', (tester) async {
      final config = CanoaConfig.parse('[voice]\noutput = "host:gone"\n');
      final h = await openSettings(tester, section: 'voice', config: config, voiceOutputs: VoiceOutputs(browser: [_local()]));
      expect(find.byKey(const Key('speak-missing')), findsOneWidget);
      expect(find.text('host:gone (not available here)'), findsOneWidget);
      expect(h.applied, isEmpty);
    });

    testWidgets('the switch writes read_replies, and the sample is read through the chosen voice and stops', (tester) async {
      final output = _local()..hold = true;
      final player = FakePlayer();
      final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices(voice: 'v2')), player: player);
      addTearDown(reader.dispose);
      final h = await openSettings(tester, section: 'voice', voiceOutputs: VoiceOutputs(browser: [output]), reader: reader);
      await tester.tap(find.descendant(of: find.byKey(const Key('speak-replies')), matching: find.byType(Switch)));
      await tester.pumpAndSettle();
      expect(h.last.voice.speak.readReplies, isTrue);
      expect(h.last.encode(), contains('read_replies = true'));

      await tester.tap(find.byKey(const Key('speak-try')));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const Key('speak-try-stop')), findsOneWidget);
      expect(output.spoken.first, startsWith('This is how replies will sound.'));
      expect(output.utterances.first.choices.voice, 'v2');
      await tester.tap(find.byKey(const Key('speak-try-stop')));
      await tester.pump();
      expect(reader.busy, isFalse);
      expect(find.byKey(const Key('speak-try')), findsOneWidget);
    });
  });
}
