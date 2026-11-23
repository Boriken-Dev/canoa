// Reading aloud as it is drawn, at desktop and phone width: a conversation whose replies have Play beside Copy (one being read, with
// Stop) above a composer with voice mode on, and Settings > Voice's group for the voice that reads replies.
import 'dart:io';

import 'package:canoa/chat/composer.dart';
import 'package:canoa/chat/dictation.dart';
import 'package:canoa/chat/reply_voice.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/voice/outputs.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_output.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_speech.dart';
import 'fixtures/fake_voice.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

final _at = DateTime(2026, 11, 18, 9);

FakeOutput _cloud() => FakeOutput(
  'host:openai',
  'openai (Endpoint)',
  where: const OutputWhere(leaves: TextLeaves.yes, words: 'The text goes to the address below, and what it keeps is its own.', endpoint: 'api.example.org/v1/audio/speech'),
  uses: const {'voice', 'model', 'style', 'speed'},
  models: const [OutputChoice('tts-small', 'Small'), OutputChoice('tts-big', 'Big')],
  defaultModel: 'tts-small',
  styles: const [OutputChoice('calm', 'Calm'), OutputChoice('bright', 'Bright')],
  speed: const SpeedRange(0.25, 4),
  voiceList: const [OutputVoice(id: 'alloy', name: 'Alloy'), OutputVoice(id: 'nova', name: 'Nova')],
);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/voice_scenes_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      testWidgets('a conversation with Play beside Copy, one reply being read, and voice mode on', (tester) async {
        final output = FakeOutput('browser', 'Stand-in', hold: true);
        final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices()), player: FakePlayer());
        addTearDown(reader.dispose);
        final store = SessionStore();
        final session = store.createDraft()..agent = 'Desk';
        final input = TextEditingController();
        final dictation = Dictation(backend: () => FakeSpeech('system', 'System'), voice: () => const VoiceSettings());
        addTearDown(() {
          dictation.dispose();
          input.dispose();
          store.dispose();
        });
        reader.setVoiceMode(true);
        await tester.binding.setSurfaceSize(size);
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final narrow = size.width < 600;
        await tester.pumpWidget(MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: themeFor(const CanoaConfig().look, Brightness.dark),
          home: Scaffold(
            body: Column(children: [
              Expanded(
                child: ChatTranscript(
                  narrow: narrow,
                  renderers: replyRenderers(reader),
                  entries: [
                    ChatEntry(MessageRole.user, 'Why does the build fail on Windows?', at: _at),
                    ChatEntry(
                      MessageRole.agent,
                      'The linker cannot find the library. Here is the fix:\n\n```sh\ncargo clean\ncargo build\n```\n\nIt rebuilds from scratch.',
                      at: _at.add(const Duration(seconds: 3)),
                    ),
                    ChatEntry(MessageRole.agent, 'Anything else you would like checked?', at: _at.add(const Duration(seconds: 9))),
                  ],
                ),
              ),
              SessionComposer(store: store, session: session, controller: input, kit: ComposerKit(dictation: dictation, reader: reader), onSend: () {}),
            ]),
          ),
        ));
        await tester.pump();
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/voice_chat_idle_$label.png'));

        await tester.tap(find.byKey(const Key('reply-play')).first);
        await tester.pump();
        await tester.pump();
        expect(find.byKey(const Key('reply-stop')), findsOneWidget);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/voice_chat_reading_$label.png'));
        reader.stop();
        await tester.pump();
      });

      testWidgets('Settings > Voice, the group that reads replies aloud', (tester) async {
        final config = CanoaConfig.parse(
          '[voice]\noutput = "host:openai"\noutput_voice = "nova"\noutput_model = "tts-big"\noutput_style = "calm"\noutput_speed = 1.25\nread_replies = true\n',
        );
        await openSettings(
          tester,
          config: config,
          size: size,
          section: 'voice',
          voiceBackends: [FakeSpeech('system', 'System')],
          voiceOutputs: VoiceOutputs(browser: [_cloud()]),
          version: '0.0.1',
        );
        await tester.ensureVisible(find.byKey(const Key('speak-output')));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/voice_read_aloud_$label.png'));
      });
    });
  }
}
