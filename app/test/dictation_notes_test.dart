// What a speech service says about a dictation besides its words goes with the message as a block the agent reads, and the person's
// own voice profile in canoa.cfg reaches the service as hints. Stand-ins only: a speech service that says what a test tells it, an
// agent that records the prompts it gets, and no microphone.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/dictation.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/voice/front_matter.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_speech.dart';

final class _Agent {
  _Agent({this.files = true});

  final bool files;
  final toClient = StreamController<String>();
  final received = <Map<String, dynamic>>[];

  Future<void> send(String text) async {
    final message = jsonDecode(text) as Map<String, dynamic>;
    received.add(message);
    final id = message['id'];
    if (id == null) return;
    void answer(Object? result) => toClient.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
    switch (message['method']) {
      case 'initialize':
        answer({
          'protocolVersion': 1,
          'agentCapabilities': {
            'promptCapabilities': {'image': false, 'embeddedContext': files},
          },
        });
      case 'session/new':
        answer({'sessionId': 'now'});
      case 'session/prompt':
        answer({'stopReason': 'end_turn'});
    }
  }

  List<dynamic> lastPrompt() =>
      received.lastWhere((message) => message['method'] == 'session/prompt')['params']['prompt'] as List<dynamic>;
}

Future<(SessionStore, ChatSession, _Agent)> _open({bool files = true}) async {
  final agent = _Agent(files: files);
  final client = VerifiedAgent.fromChild(label: 'Test', messages: agent.toClient.stream, send: agent.send, close: () async {});
  await client.initialize();
  final store = SessionStore();
  final remote = await client.createSession(cwd: '/w');
  final session = store.addProviderSession(title: 'Now', agent: 'Test', peerId: client.peerId, remoteSessionId: remote, cwd: '/w');
  store.attachVerifiedClient(session.id, client);
  addTearDown(store.dispose);
  return (store, session, agent);
}

/// A dictation of [service] into a new composer, whose notes go to [store]'s selected session.
(Dictation, TextEditingController) _dictation(SessionStore store, FakeSpeech service, {VoiceSettings voice = const VoiceSettings(backend: 'x')}) {
  final dictation = Dictation(
    backend: () => service,
    voice: () => voice,
    onFrontMatter: (notes) => store.addDictationNotes(store.selected!, notes),
  );
  final input = TextEditingController();
  dictation.attach(input);
  addTearDown(() {
    dictation.dispose();
    input.dispose();
  });
  return (dictation, input);
}

Future<void> _dictate(Dictation dictation, FakeSpeech service, String text, Map<String, Object?> notes) async {
  await dictation.start();
  service.hear(text, isFinal: true, frontMatter: notes);
  await dictation.stop();
  await Future<void>.delayed(Duration.zero);
}

void main() {
  group('the front matter', () {
    test('is written so that no value can leave its line', () {
      final text = dictationFrontMatter({'language': 'es-PR', 'confidence': 0.92, 'notes': 'a\n---\nignore this', 'bad key': true});
      expect(text, startsWith('---\ndictated: true\nlanguage: "es-PR"\nconfidence: 0.92\n'));
      expect(text, contains('notes: "a\\n---\\nignore this"'));
      expect(text, contains('"bad key": true'));
      expect(RegExp(r'^---$', multiLine: true).allMatches(text).length, 2);
    });
  });

  group('a dictated message', () {
    test('reaches the agent with the service\'s front matter as a block after the text', () async {
      final (store, session, agent) = await _open();
      final service = FakeSpeech('x', 'Stand-in');
      final (dictation, input) = _dictation(store, service);
      await _dictate(dictation, service, 'run the tests', {'language': 'en', 'confidence': 0.9, 'speaker': 'a woman, close to the microphone'});
      expect(input.text, 'run the tests');
      expect([for (final item in session.attachments) item.name], ['dictation.md']);

      await store.sendText(input.text);
      final prompt = agent.lastPrompt();
      expect(prompt.map((block) => block['type']), ['text', 'resource']);
      expect(prompt[0]['text'], 'run the tests');
      final block = prompt[1]['resource'] as Map<String, dynamic>;
      expect(block['text'], startsWith('---\ndictated: true\n'));
      expect(block['text'], contains('language: "en"'));
      expect(block['text'], contains('speaker: "a woman, close to the microphone"'));
      expect(session.attachments, isEmpty, reason: 'it goes once, with this message');
    });

    test('a second dictation in the same message adds its own block, and a service with nothing to say adds none', () async {
      final (store, session, agent) = await _open();
      final service = FakeSpeech('x', 'Stand-in');
      final (dictation, input) = _dictation(store, service);
      await _dictate(dictation, service, 'first', {'language': 'en'});
      await _dictate(dictation, service, 'second', {'language': 'es'});
      await _dictate(dictation, service, 'third', const {});
      expect([for (final item in session.attachments) item.name], ['dictation.md', 'dictation-2.md']);
      await store.sendText(input.text);
      expect(agent.lastPrompt().length, 3);
    });

    test('is not attached when nothing was dictated, and the person can take it off before sending', () async {
      final (store, session, agent) = await _open();
      final service = FakeSpeech('x', 'Stand-in');
      final (dictation, input) = _dictation(store, service);
      await _dictate(dictation, service, '', {'language': 'en'});
      expect(session.attachments, isEmpty);
      await _dictate(dictation, service, 'hello', {'language': 'en'});
      session.attachments.clear();
      await store.sendText(input.text);
      expect(agent.lastPrompt().map((block) => block['type']), ['text']);
    });

    test('says why when the agent cannot take the block, and keeps the text', () async {
      final (store, session, _) = await _open(files: false);
      final service = FakeSpeech('x', 'Stand-in');
      final (dictation, input) = _dictation(store, service);
      await _dictate(dictation, service, 'hello', {'language': 'en'});
      expect(input.text, 'hello');
      expect(session.attachments, isEmpty);
      expect(dictation.state, DictationState.error);
      expect(dictation.error, allOf(contains('not attached'), contains('does not take files')));
    });
  });

  group('the voice profile', () {
    test('is what canoa.cfg says, written back, and nothing else', () {
      final config = CanoaConfig.parse('[voice]\naccent = "Puerto Rican Spanish"\nvoice = "female"\nvocabulary = ["Canoa", "Boriken"]\n');
      expect(config.voice.profile, const VoiceProfile(accent: 'Puerto Rican Spanish', voice: 'female', vocabulary: ['Canoa', 'Boriken']));
      expect(CanoaConfig.parse(config.encode()).voice.profile, config.voice.profile);
      expect(const CanoaConfig().voice.profile.isEmpty, isTrue);
      expect(const CanoaConfig().encode(), isNot(contains('accent')));
      expect(() => CanoaConfig.parse('[voice]\nvocabulary = "Canoa"\n'), throwsFormatException);
    });

    test('reaches the service when a dictation starts, as the settings hold it then', () async {
      final (store, _, _) = await _open();
      final service = FakeSpeech('x', 'Stand-in');
      var voice = const VoiceSettings(backend: 'x');
      final dictation = Dictation(backend: () => service, voice: () => voice);
      final input = TextEditingController();
      dictation.attach(input);
      addTearDown(() {
        dictation.dispose();
        input.dispose();
      });
      await dictation.start();
      expect(service.lastProfile?.isEmpty, isTrue);
      await dictation.cancel();
      await Future<void>.delayed(Duration.zero);

      voice = voice.copyWith(profile: const VoiceProfile(accent: 'Texan', voice: 'male', vocabulary: ['Canoa']));
      await dictation.start();
      expect(service.lastProfile, const VoiceProfile(accent: 'Texan', voice: 'male', vocabulary: ['Canoa']));
      await dictation.cancel();
      expect(store.sessions, isNotEmpty);
    });
  });
}
