// Dictation in the composer, on stand-ins: a speech service that says what a test tells it to, and, for the host path, a control
// link that answers `speech/*` and a capture that plays a few chunks. No microphone is opened.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/chat/composer.dart';
import 'package:canoa/chat/dictation.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A service the test drives: it starts, hears what [say] gives it and ends on [stop] with what [finalText] holds.
final class _Service implements SpeechBackend {
  _Service({this.finalText = 'run the tests'});

  String finalText;
  StreamController<Transcript>? out;
  int started = 0, stopped = 0, cancelled = 0;
  Object? failOnStop;

  @override
  String get id => 'stand-in';
  @override
  String get name => 'Stand-in';
  @override
  SpeechWhere get where => const SpeechWhere(place: SpeechPlace.thisDevice, leaves: AudioLeaves.no, words: 'Nothing leaves this device.');
  @override
  Future<List<SpeechChoice>> models() async => const [];
  @override
  Future<List<SpeechChoice>> languages() async => const [];
  @override
  Future<List<SpeechChoice>> devices() async => const [];
  @override
  Stream<double> get levels => const Stream.empty();

  @override
  Stream<Transcript> start({String? device, String? language, String? model}) {
    started++;
    return (out = StreamController<Transcript>()).stream;
  }

  void say(String text) => out!.add(Transcript(text, isFinal: false));
  void fail(String message) => out!.addError(SpeechFailure(message));
  Future<void> end() => out!.close();

  @override
  Future<void> stop() async {
    stopped++;
    if (failOnStop != null) out!.addError(failOnStop!);
    if (finalText.isNotEmpty && failOnStop == null) out!.add(Transcript(finalText, isFinal: true));
    await out!.close();
  }

  @override
  Future<void> cancel() async {
    cancelled++;
    await out!.close();
  }
}

final class _Rig {
  _Rig(this.service, this.dictation, this.input, this.store, this.session);
  final _Service service;
  final Dictation dictation;
  final TextEditingController input;
  final SessionStore store;
  final ChatSession session;
  int sent = 0;
}

Future<_Rig> _pump(WidgetTester tester, {_Service? service, SpeechBackend? Function()? backend}) async {
  final stand = service ?? _Service();
  final dictation = Dictation(backend: backend ?? () => stand, voice: () => const VoiceSettings(backend: 'stand-in'));
  final store = SessionStore();
  final session = store.createDraft()..agent = 'Desk';
  final input = TextEditingController();
  final rig = _Rig(stand, dictation, input, store, session);
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
        child: SessionComposer(store: store, session: session, controller: input, kit: ComposerKit(dictation: dictation), onSend: () => rig.sent++),
      ),
    ),
  ));
  await tester.pump();
  return rig;
}

/// Presses the microphone and answers the first-use dialog.
Future<void> _begin(WidgetTester tester, {bool accept = true}) async {
  await tester.tap(find.byKey(const Key('dictate')));
  await tester.pump();
  if (find.byKey(const Key('dictate-accept')).evaluate().isNotEmpty) {
    await tester.tap(find.byKey(Key(accept ? 'dictate-accept' : 'dictate-decline')));
  }
  await tester.pump();
  await tester.pump();
}

void main() {
  testWidgets('a build with no way to dictate has no microphone', (tester) async {
    final store = SessionStore();
    final session = store.createDraft();
    final input = TextEditingController();
    addTearDown(store.dispose);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SessionComposer(store: store, session: session, controller: input, kit: const ComposerKit(), onSend: () {}))));
    expect(find.byKey(const Key('dictate')), findsNothing);
  });

  testWidgets('the first use says where the audio goes, and declining opens no microphone', (tester) async {
    final rig = await _pump(tester);
    await tester.tap(find.byKey(const Key('dictate')));
    await tester.pump();
    await tester.pump();
    expect(find.text('Dictate with Stand-in'), findsOneWidget);
    expect(find.textContaining('Nothing leaves this device.'), findsOneWidget);
    expect(rig.service.started, 0);
    await tester.tap(find.byKey(const Key('dictate-decline')));
    await tester.pump();
    await tester.pump();
    expect(rig.service.started, 0);
    expect(rig.dictation.state, DictationState.idle);
    // Asked again, since it was not accepted; once accepted it is not asked a second time.
    await _begin(tester);
    expect(rig.service.started, 1);
    await tester.tap(find.byKey(const Key('dictate-cancel')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('dictate')));
    await tester.pump();
    expect(find.byKey(const Key('dictate-accept')), findsNothing);
    expect(rig.service.started, 2);
    await tester.tap(find.byKey(const Key('dictate-cancel')));
    await tester.pump();
  });

  testWidgets('start shows the time and what is heard so far apart from the message, and stop puts the text at the cursor, unsent', (tester) async {
    final rig = await _pump(tester);
    rig.input.value = const TextEditingValue(text: 'please  now', selection: TextSelection.collapsed(offset: 7));
    await _begin(tester);
    expect(rig.dictation.state, DictationState.listening);
    expect(find.byKey(const Key('dictate-time')), findsOneWidget);
    await tester.pump(const Duration(seconds: 3));
    expect(find.text('0:03'), findsOneWidget);
    rig.service.say('run the te');
    await tester.pump();
    await tester.pump();
    expect(find.text('run the te'), findsOneWidget);
    expect(rig.input.text, 'please  now', reason: 'provisional text is not in the message');

    await tester.tap(find.byKey(const Key('dictate-stop')));
    await tester.pump();
    await tester.pump();
    expect(rig.service.stopped, 1);
    expect(rig.dictation.state, DictationState.idle);
    expect(rig.input.text, 'please run the tests now');
    expect(rig.input.selection.baseOffset, 'please run the tests'.length);
    expect(rig.sent, 0, reason: 'dictation never sends');
    expect(find.byKey(const Key('dictate')), findsOneWidget);
  });

  testWidgets('dictation into a message with an attachment leaves the attachment', (tester) async {
    final rig = await _pump(tester);
    expect(rig.store.addText(rig.session, 'notes from the call'), isNull);
    await tester.pump();
    await _begin(tester);
    await tester.tap(find.byKey(const Key('dictate-stop')));
    await tester.pump();
    await tester.pump();
    expect(rig.input.text, 'run the tests');
    expect(rig.session.attachments, hasLength(1));
  });

  testWidgets('Cancel puts nothing in the message', (tester) async {
    final rig = await _pump(tester);
    rig.input.text = 'draft';
    await _begin(tester);
    rig.service.say('something heard');
    await tester.pump();
    await tester.tap(find.byKey(const Key('dictate-cancel')));
    await tester.pump();
    await tester.pump();
    expect(rig.service.cancelled, 1);
    expect(rig.input.text, 'draft');
    expect(rig.dictation.state, DictationState.idle);
    expect(find.byKey(const Key('dictate')), findsOneWidget);
  });

  testWidgets('Stop with no speech puts nothing in the message', (tester) async {
    final rig = await _pump(tester, service: _Service(finalText: ''));
    rig.input.text = 'draft';
    await _begin(tester);
    await tester.tap(find.byKey(const Key('dictate-stop')));
    await tester.pump();
    await tester.pump();
    expect(rig.input.text, 'draft');
    expect(rig.dictation.state, DictationState.idle);
  });

  testWidgets('a service that fails ends the capture and says why in words, and the microphone can be tried again', (tester) async {
    final rig = await _pump(tester);
    rig.input.text = 'draft';
    await _begin(tester);
    rig.service.fail('The microphone is not allowed for this app. Allow it in the system settings and try again.');
    await rig.service.end();
    await tester.pump();
    await tester.pump();
    expect(rig.dictation.state, DictationState.error);
    expect(find.textContaining('The microphone is not allowed'), findsOneWidget);
    expect(rig.input.text, 'draft');
    expect(find.byKey(const Key('dictate')), findsOneWidget, reason: 'the recovery is one press away');

    await tester.tap(find.byKey(const Key('dictate-dismiss')));
    await tester.pump();
    expect(find.byKey(const Key('dictate-error')), findsNothing);
    await _begin(tester);
    expect(rig.dictation.state, DictationState.listening);
    await tester.tap(find.byKey(const Key('dictate-cancel')));
    await tester.pump();
  });

  testWidgets('a failure while the service finishes keeps its words and inserts nothing', (tester) async {
    final rig = await _pump(tester, service: _Service()..failOnStop = const SpeechFailure('The host did not answer in time.'));
    await _begin(tester);
    await tester.tap(find.byKey(const Key('dictate-stop')));
    await tester.pump();
    await tester.pump();
    expect(find.text('The host did not answer in time.'), findsOneWidget);
    expect(rig.input.text, isEmpty);
  });

  testWidgets('no service of the chosen name says so and opens nothing', (tester) async {
    final rig = await _pump(tester, backend: () => null);
    await tester.tap(find.byKey(const Key('dictate')));
    await tester.pump();
    await tester.pump();
    expect(rig.dictation.state, DictationState.error);
    expect(find.textContaining('Choose one in Settings'), findsOneWidget);
    expect(rig.service.started, 0);
  });

  testWidgets('the shortcut starts and ends a dictation like the button', (tester) async {
    final rig = await _pump(tester);
    await _begin(tester);
    await rig.dictation.toggle();
    await tester.pump();
    await tester.pump();
    expect(rig.input.text, 'run the tests');
  });

  testWidgets('through the host: the capture streams to the provider and its transcript lands in the message', (tester) async {
    final link = _Link();
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => link), capture: _Tone());
    await tester.runAsync(catalog.refresh);
    final rig = await _pump(tester, backend: () => catalog.backends.single);
    await _begin(tester);
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pump();
    expect(rig.dictation.state, DictationState.listening);
    await tester.tap(find.byKey(const Key('dictate-stop')));
    await tester.pump();
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pump();
    expect(rig.input.text, 'hello from the host');
    expect(link.pushed, greaterThan(0));
    expect(link.finished, 1);
    expect(link.cancelled, 0);
    expect(rig.sent, 0);
  });

  testWidgets('through the host: a host that is gone ends the dictation with its words', (tester) async {
    final link = _Link()..refuseOpen = true;
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => link), capture: _Tone());
    await tester.runAsync(catalog.refresh);
    final rig = await _pump(tester, backend: () => catalog.backends.single);
    await _begin(tester);
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pump();
    expect(rig.dictation.state, DictationState.error);
    expect(find.textContaining('no speech here'), findsOneWidget);
    expect(rig.input.text, isEmpty);
  });
}

/// A capture that plays two chunks and then waits for the listener to cancel it, as a microphone does.
final class _Tone implements AudioCapture {
  @override
  Stream<Uint8List> open({required int rate}) {
    late StreamController<Uint8List> out;
    out = StreamController<Uint8List>(
      onListen: () {
        out.add(Uint8List(3200));
        out.add(Uint8List(3200));
      },
    );
    return out.stream;
  }
}

/// A host with one provider that answers `speech/*` the way the real one does.
final class _Link implements HostLink {
  final _out = StreamController<String>();
  int pushed = 0, finished = 0, cancelled = 0;
  bool refuseOpen = false;

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final Object result;
    switch (request['method']) {
      case 'speech/providers':
        result = {
          'types': const [],
          'providers': [
            {'name': 'stand-in', 'provider': 'speech_command', 'title': 'Command', 'description': '', 'settings': <String, Object?>{}, 'secrets_set': <String>[], 'partials': false},
          ],
        };
      case 'speech/open':
        if (refuseOpen) {
          _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32000, 'message': 'The provider has no speech here.'}}));
          return;
        }
        result = {'session': 's1'};
      case 'speech/push':
        pushed++;
        result = <String, Object?>{};
      case 'speech/finish':
        finished++;
        result = {'text': 'hello from the host'};
      case 'speech/cancel':
        cancelled++;
        result = <String, Object?>{};
      default:
        result = <String, Object?>{};
    }
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() async {
    await _out.close();
  }
}
