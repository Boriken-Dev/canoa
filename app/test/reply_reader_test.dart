// Replies read aloud, on stand-ins: a voice provider that keeps what it was asked and a player that keeps what it was given. Play
// reads a reply sentence by sentence, in order, with no code in it, and Stop ends it; voice mode follows a reply as it streams; a
// provider that fails stops the reading, and the text is never handed to another.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/voice/outputs.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_backend.dart' show SpeechFailure;
import 'package:canoa/voice/speech_output.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_voice.dart';

({ReplyReader reader, FakeOutput output, FakePlayer player}) _rig({FakeOutput? output, SpeakChoices choices = const SpeakChoices()}) {
  final out = output ?? FakeOutput('browser', 'Stand-in');
  final player = FakePlayer();
  final reader = ReplyReader(voice: () async => (output: out, choices: choices), player: player);
  addTearDown(reader.dispose);
  return (reader: reader, output: out, player: player);
}

void main() {
  group('Play', () {
    test('reads the prose of a reply in order, one sentence after another, and nothing of its code', () async {
      final rig = _rig();
      await rig.reader.toggle('a', 'Run this:\n\n```sh\nrm -rf build\n```\n\nIt is done. Is it? Yes!');
      expect(rig.reader.isReading('a'), isTrue);
      await pumpEventQueue();
      expect(rig.output.spoken, ['Run this:', 'It is done.', 'Is it?', 'Yes!']);
      expect(rig.player.written, ['Run this:', 'It is done.', 'Is it?', 'Yes!']);
      expect(rig.player.events.first, 'open 22050');
      expect(rig.player.events.last, 'drain');
      expect(rig.reader.busy, isFalse);
      expect(rig.reader.error, isNull);
    });

    test('presses of Play and Stop: reading the one being read stops it, and what was not said is never said', () async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final rig = _rig(output: output);
      await rig.reader.toggle('a', 'One. Two. Three.');
      await pumpEventQueue();
      expect(rig.reader.reading, 'a');
      // The sentence being read is held, and the next is already being made so there is no gap.
      expect(output.spoken, ['One.', 'Two.']);
      await rig.reader.toggle('a', 'One. Two. Three.');
      await pumpEventQueue();
      expect(rig.reader.busy, isFalse);
      expect(rig.reader.isReading('a'), isFalse);
      expect(output.utterances.every((each) => each.stopped), isTrue);
      expect(rig.player.events.last, 'stop');
      expect(rig.player.written, isEmpty);
      expect(output.spoken, ['One.', 'Two.'], reason: 'the third was never asked for');
    });

    test('reading another reply replaces the first', () async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final rig = _rig(output: output);
      await rig.reader.toggle('a', 'First reply.');
      await pumpEventQueue();
      output.hold = false;
      await rig.reader.toggle('b', 'Second reply.');
      await pumpEventQueue();
      expect(output.utterances.first.stopped, isTrue);
      expect(rig.player.written, ['Second reply.']);
      expect(rig.reader.busy, isFalse);
    });

    test('the choices Settings holds reach the provider with every sentence', () async {
      final output = FakeOutput(
        'host:says',
        'Says',
        uses: const {'voice', 'model', 'style', 'speed'},
        speed: const SpeedRange(0.5, 2),
      );
      const speak = SpeakSettings(output: 'host:says', voice: 'ana', model: 'big', style: 'calm', speed: 4);
      final rig = _rig(output: output, choices: choicesFor(output, speak));
      await rig.reader.toggle('a', 'One. Two.');
      await pumpEventQueue();
      for (final utterance in output.utterances) {
        expect((utterance.choices.voice, utterance.choices.model, utterance.choices.style, utterance.choices.speed), ('ana', 'big', 'calm', 2.0));
      }
    });

    test('a provider that fails stops the reading, says why under the reply, and nothing goes to another provider', () async {
      final failing = FakeOutput('host:a', 'A', failWith: const SpeechFailure('The voice is not installed.'));
      final other = FakeOutput('host:b', 'B');
      final rig = _rig(output: failing);
      await rig.reader.toggle('a', 'One. Two.');
      await pumpEventQueue();
      expect(rig.reader.error, 'The voice is not installed.');
      expect(rig.reader.errorKey, 'a');
      expect(rig.reader.busy, isFalse);
      expect(other.spoken, isEmpty);
      expect(rig.player.written, isEmpty);
      rig.reader.clearError();
      expect(rig.reader.error, isNull);
    });

    test('with no voice, it says so in words', () async {
      final reader = ReplyReader(voice: () async => null, whyNone: () => 'Nothing is set up.');
      addTearDown(reader.dispose);
      await reader.toggle('a', 'Hello there.');
      await pumpEventQueue();
      expect(reader.error, 'Nothing is set up.');
    });

    test('a provider that plays by itself is waited for, and the player is not used', () async {
      final output = FakeOutput('browser', 'Browser', pcm: null);
      final rig = _rig(output: output);
      await rig.reader.toggle('a', 'One. Two.');
      await pumpEventQueue();
      expect(output.spoken, ['One.', 'Two.']);
      expect(rig.player.events, isEmpty);
      expect(rig.reader.busy, isFalse);
    });

    test('a press unlocks the audio output from inside the press', () async {
      final rig = _rig();
      await rig.reader.toggle('a', 'One.');
      expect(rig.player.unlocks, 1);
    });
  });

  group('voice mode', () {
    ReplyText reply(String text, {bool complete = false}) => (key: 'r1', text: text, complete: complete);

    test('turned on, it reads no earlier reply, then each sentence of a new one as it streams, in order', () async {
      final rig = _rig();
      rig.reader.setVoiceMode(true);
      rig.reader.observe('s', [(key: 'old', text: 'An old reply.', complete: true)]);
      await pumpEventQueue();
      expect(rig.output.spoken, isEmpty);

      rig.reader.observe('s', [(key: 'old', text: 'An old reply.', complete: true), reply('Hello the')]);
      await pumpEventQueue();
      expect(rig.output.spoken, isEmpty);

      rig.reader.observe('s', [reply('Hello there. How are')]);
      await pumpEventQueue();
      expect(rig.output.spoken, ['Hello there.']);
      expect(rig.reader.busy, isTrue, reason: 'it waits for the rest of the reply');

      rig.reader.observe('s', [reply('Hello there. How are you? Fi')]);
      await pumpEventQueue();
      expect(rig.output.spoken, ['Hello there.', 'How are you?']);

      rig.reader.observe('s', [reply('Hello there. How are you? Fine.', complete: true)]);
      await pumpEventQueue();
      expect(rig.output.spoken, ['Hello there.', 'How are you?', 'Fine.']);
      expect(rig.player.written, ['Hello there.', 'How are you?', 'Fine.']);
      expect(rig.reader.busy, isFalse);
      expect(rig.player.events.last, 'drain');
    });

    test('stopping drops the rest of the reply being read, and the next reply is read again', () async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final rig = _rig(output: output);
      rig.reader.setVoiceMode(true);
      rig.reader.observe('s', const []);
      rig.reader.observe('s', [reply('One. Two. Three')]);
      await pumpEventQueue();
      expect(rig.reader.busy, isTrue);
      // The person types.
      rig.reader.stop();
      await pumpEventQueue();
      expect(rig.reader.busy, isFalse);
      output.hold = false;
      rig.reader.observe('s', [reply('One. Two. Three. Four.', complete: true)]);
      await pumpEventQueue();
      expect(output.spoken.where((text) => text == 'Four.'), isEmpty, reason: 'the stopped reply is not read any more');
      rig.reader.observe('s', [reply('One. Two. Three. Four.', complete: true), (key: 'r2', text: 'Next reply.', complete: false)]);
      rig.reader.observe('s', [reply('One. Two. Three. Four.', complete: true), (key: 'r2', text: 'Next reply. More.', complete: true)]);
      await pumpEventQueue();
      expect(output.spoken, contains('Next reply.'));
      expect(rig.reader.voiceMode, isTrue);
    });

    test('off, it reads nothing and stops what was being read; another conversation is not read from the start', () async {
      final output = FakeOutput('browser', 'Stand-in', hold: true);
      final rig = _rig(output: output);
      rig.reader.observe('s', [reply('Ignored. Entirely.', complete: true)]);
      await pumpEventQueue();
      expect(output.spoken, isEmpty);
      rig.reader.setVoiceMode(true);
      rig.reader.observe('s', const []);
      rig.reader.observe('s', [reply('One. Two. Three')]);
      await pumpEventQueue();
      expect(rig.reader.busy, isTrue);
      rig.reader.setVoiceMode(false);
      await pumpEventQueue();
      expect(rig.reader.busy, isFalse);
      rig.reader.setVoiceMode(true);
      rig.reader.observe('other', [reply('Already there. Said before.', complete: true)]);
      await pumpEventQueue();
      expect(rig.reader.busy, isFalse);
    });
  });

  group('the voices', () {
    test('a named output is the one used, and a named one that is gone is never replaced by another', () {
      final browser = FakeOutput('browser', 'Browser'), online = FakeOutput('browser:online', 'Online'), host = FakeOutput('host:system', 'System');
      final outputs = VoiceOutputs(browser: [browser, online, host]);
      expect(outputs.resolve(null), same(browser));
      expect(outputs.resolve('host:system'), same(host));
      expect(outputs.resolve('host:piper'), isNull);
      expect(VoiceOutputs(browser: [online, host]).resolve(null), same(host));
      expect(VoiceOutputs(browser: [online]).resolve(null), isNull, reason: 'online voices are only ever chosen by the person');
    });

    test('only the choices a provider takes are sent to it', () {
      final output = FakeOutput('x', 'X', uses: const {'voice'});
      final choices = choicesFor(output, const SpeakSettings(voice: 'v', model: 'm', style: 's', speed: 2));
      expect((choices.voice, choices.model, choices.style, choices.speed), ('v', null, null, null));
    });
  });
}
