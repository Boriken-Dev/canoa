// The chat's reading aloud, in widgets: Play beside Copy on an agent's reply (reading its prose, ending on Stop), voice mode's switch
// in the composer's + menu (which stops when the person types or dictates), and, in the whole app, a reply read as it arrives.
import 'package:canoa/chat/composer.dart';
import 'package:canoa/chat/dictation.dart';
import 'package:canoa/chat/reply_voice.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/voice/outputs.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_backend.dart' show SpeechFailure;
import 'package:canoa/voice/speech_output.dart' show SpeakChoices;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_speech.dart';
import 'fixtures/fake_voice.dart';
import 'fixtures/options_agent.dart';

final _at = DateTime(2026, 11, 18, 9);

Future<ReplyReader> _transcript(WidgetTester tester, ReplyReader reader, List<ChatEntry> entries) async {
  await tester.binding.setSurfaceSize(const Size(900, 700));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(const CanoaConfig().look, Brightness.dark),
    home: Scaffold(body: ChatTranscript(entries: entries, renderers: replyRenderers(reader))),
  ));
  await tester.pump();
  return reader;
}

ReplyReader _reader(FakeOutput output, FakePlayer player, {bool Function()? offered}) {
  final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices()), player: player, offered: offered);
  addTearDown(reader.dispose);
  return reader;
}

void main() {
  group('Play on a reply', () {
    testWidgets('sits beside Copy on an agent reply, and a person\'s own message has no Play', (tester) async {
      final reader = _reader(FakeOutput('browser', 'Stand-in'), FakePlayer());
      await _transcript(tester, reader, [
        ChatEntry(MessageRole.user, 'Question', at: _at),
        ChatEntry(MessageRole.agent, 'Answer.', at: _at.add(const Duration(seconds: 1))),
      ]);
      expect(find.byKey(const Key('reply-play')), findsOneWidget);
      expect(find.byTooltip('Read aloud'), findsOneWidget);
      expect(find.byTooltip('Copy message'), findsNWidgets(2));
      final play = tester.getCenter(find.byKey(const Key('reply-play'))), copy = tester.getCenter(find.byTooltip('Copy message').last);
      expect(play.dy, copy.dy);
      expect(play.dx, lessThan(copy.dx));
    });

    testWidgets('reads the reply\'s prose and not its code, and Stop ends it', (tester) async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final player = FakePlayer();
      final reader = _reader(output, player);
      await _transcript(tester, reader, [
        ChatEntry(MessageRole.agent, 'Run this:\n\n```sh\nrm -rf build\n```\n\nIt is done. Really.', at: _at),
      ]);
      await tester.tap(find.byKey(const Key('reply-play')));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const Key('reply-stop')), findsOneWidget);
      expect(find.byTooltip('Stop reading'), findsOneWidget);
      expect(output.spoken, ['Run this:', 'It is done.']);
      expect(player.unlocks, 1);
      await tester.tap(find.byKey(const Key('reply-stop')));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const Key('reply-play')), findsOneWidget);
      expect(output.utterances.every((each) => each.stopped), isTrue);
      expect(reader.busy, isFalse);
      expect(output.spoken, ['Run this:', 'It is done.'], reason: 'nothing after Stop is asked for');
    });

    testWidgets('reads through the player, in order, and ends by itself', (tester) async {
      final output = FakeOutput('browser', 'Stand-in');
      final player = FakePlayer();
      await _transcript(tester, _reader(output, player), [ChatEntry(MessageRole.agent, 'One. Two. Three.', at: _at)]);
      await tester.tap(find.byKey(const Key('reply-play')));
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      expect(player.written, ['One.', 'Two.', 'Three.']);
      expect(find.byKey(const Key('reply-play')), findsOneWidget);
    });

    testWidgets('a provider that fails is said in words under the page, and the reply goes nowhere else', (tester) async {
      final output = FakeOutput('browser', 'Stand-in', failWith: const SpeechFailure('The voice is not installed.'));
      await _transcript(tester, _reader(output, FakePlayer()), [ChatEntry(MessageRole.agent, 'One.', at: _at)]);
      await tester.tap(find.byKey(const Key('reply-play')));
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      expect(find.text('The voice is not installed.'), findsOneWidget);
    });

    testWidgets('where nothing can read aloud, the reply has only its Copy', (tester) async {
      final reader = _reader(FakeOutput('browser', 'Stand-in'), FakePlayer(), offered: () => false);
      await _transcript(tester, reader, [ChatEntry(MessageRole.agent, 'Answer.', at: _at)]);
      expect(find.byKey(const Key('reply-play')), findsNothing);
      expect(find.byTooltip('Copy message'), findsOneWidget);
    });
  });

  group('the voice mode switch', () {
    late TextEditingController input;
    late Dictation dictation;
    late FakeSpeech service;

    Future<ReplyReader> pump(WidgetTester tester, FakeOutput output, FakePlayer player, {List<bool>? changes}) async {
      final store = SessionStore();
      final session = store.createDraft()..agent = 'Desk';
      input = TextEditingController();
      service = FakeSpeech('stand-in', 'Stand-in');
      dictation = Dictation(backend: () => service, voice: () => const VoiceSettings(backend: 'stand-in', acknowledged: {'stand-in'}));
      final reader = _reader(output, player);
      addTearDown(() {
        dictation.dispose();
        input.dispose();
        store.dispose();
      });
      await tester.binding.setSurfaceSize(const Size(900, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(const CanoaConfig().look, Brightness.dark),
        home: Scaffold(
          body: Align(
            alignment: Alignment.bottomCenter,
            child: SessionComposer(
              store: store,
              session: session,
              controller: input,
              kit: ComposerKit(dictation: dictation, reader: reader, onVoiceMode: changes?.add),
              onSend: () {},
            ),
          ),
        ),
      ));
      await tester.pump();
      return reader;
    }

    testWidgets('is in the + menu, turns voice mode on and off, and says so in the line under the box while it is on', (tester) async {
      final changes = <bool>[];
      final player = FakePlayer();
      final reader = await pump(tester, FakeOutput('browser', 'Stand-in'), player, changes: changes);
      expect(find.byKey(const Key('dictate')), findsOneWidget);
      expect(find.byKey(const Key('voice-mode')), findsNothing, reason: 'it is in the menu, not beside the microphone');
      expect(find.byKey(const Key('voice-mode-on')), findsNothing);
      expect(reader.voiceMode, isFalse);
      await choosePlus(tester, 'voice-mode');
      expect(reader.voiceMode, isTrue);
      expect(changes, [true]);
      expect(player.unlocks, 1, reason: 'a browser lets a page make a sound only after a press');
      expect(find.byKey(const Key('voice-mode-on')), findsOneWidget);
      expect(find.text('Voice mode'), findsOneWidget);
      // The status in the line turns it off, as the menu entry does.
      await tester.tap(find.byKey(const Key('voice-mode-on')));
      await tester.pump();
      expect(reader.voiceMode, isFalse);
      expect(changes, [true, false]);
      expect(find.byKey(const Key('voice-mode-on')), findsNothing);
      await choosePlus(tester, 'voice-mode');
      await choosePlus(tester, 'voice-mode');
      expect(changes, [true, false, true, false]);
    });

    testWidgets('typing stops what is being read, and sending the message does not count as typing', (tester) async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final reader = await pump(tester, output, FakePlayer());
      reader.setVoiceMode(true);
      reader.observe('s', const []);
      reader.observe('s', [(key: 'r', text: 'One. Two. Three', complete: false)]);
      await tester.runAsync(() => pumpEventQueue());
      expect(reader.busy, isTrue);
      await tester.enterText(find.byType(TextField).last, 'wait');
      await tester.pump();
      expect(reader.busy, isFalse);
      expect(output.utterances.every((each) => each.stopped), isTrue);
      expect(reader.voiceMode, isTrue, reason: 'only the reading stops; the mode stays on');

      output.hold = true;
      reader.observe('s', [(key: 'r', text: 'One. Two. Three', complete: false), (key: 'q', text: 'Next. Reply', complete: false)]);
      await tester.runAsync(() => pumpEventQueue());
      expect(reader.busy, isTrue);
      input.clear();
      await tester.pump();
      expect(reader.busy, isTrue, reason: 'an emptied message is not typing');
    });

    testWidgets('starting to dictate stops what is being read', (tester) async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final reader = await pump(tester, output, FakePlayer());
      reader.setVoiceMode(true);
      reader.observe('s', const []);
      reader.observe('s', [(key: 'r', text: 'One. Two. Three', complete: false)]);
      await tester.runAsync(() => pumpEventQueue());
      expect(reader.busy, isTrue);
      await tester.tap(find.byKey(const Key('dictate')));
      await tester.pump();
      await tester.pump();
      expect(service.starts, 1);
      expect(reader.busy, isFalse);
      await tester.tap(find.byKey(const Key('dictate-cancel')));
      await tester.pump();
    });
  });

  group('in the app', () {
    testWidgets('voice mode reads a reply as it arrives, and Play reads one again; the switch is kept in canoa.cfg', (tester) async {
      final store = SessionStore();
      final agent = OptionsAgent();
      final output = FakeOutput('browser', 'Stand-in');
      final player = FakePlayer();
      final written = await pumpApp(
        tester,
        store,
        linked: agent.openLink,
        size: const Size(1280, 1000),
        voiceOutputs: VoiceOutputs(browser: [output]),
        pcmPlayer: player,
      );
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await choosePlus(tester, 'voice-mode');
      expect(written.last.voice.speak.readReplies, isTrue);
      expect(written.last.encode(), contains('read_replies = true'));

      await send(tester, 'First question');
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      expect(output.spoken, ['Done.']);
      expect(player.written, ['Done.']);

      store.selected!.chat.addMessage(MessageRole.agent, 'Here:\n\n```\nfinal x = 1;\n```\n\nThat sets x. Then more.');
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      expect(output.spoken, ['Done.', 'Here:', 'That sets x.', 'Then more.']);

      // Typing in the message ends what is being read.
      output.hold = true;
      store.selected!.chat.addMessage(MessageRole.agent, 'Another reply. With two parts.');
      await tester.runAsync(() => pumpEventQueue());
      await tester.enterText(find.byType(TextField).last, 'stop talking');
      await tester.pump();
      expect(output.utterances.last.stopped, isTrue);

      // Play on one reply reads it from its start.
      output.hold = false;
      await tester.tap(find.byKey(const Key('reply-play')).first);
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      expect(output.spoken.last, 'Done.');
      await finish(tester);
    });
  });
}
