// Reading aloud in the places the main chat's buttons do not reach: a conversation in a window of its own, which plays no sound and asks
// the main window to (so a reply is read the same way wherever it is shown), and a child's conversation, which voice mode follows
// as it follows the one the person started.
import 'dart:async';

import 'package:canoa/chat/child_thread.dart';
import 'package:canoa/chat/reply_voice.dart';
import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_launch.dart';
import 'package:canoa/popout/read_aloud.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/voice/reply_reader.dart';
import 'package:canoa/voice/speech_output.dart' show SpeakChoices;
import 'package:coqui_acp_chat/experimental.dart' show ChatSessionView, MemoryViewLink;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_voice.dart';
import 'fixtures/popout_windows.dart';
import 'fixtures/scripted_agent.dart';

/// Lets what happens outside the test's clock (the agent's lines, a link's messages, the share's timer) happen, then draws.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 120)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 300));
}

Map<String, Object?> _said(String text) => {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': text}};

/// A conversation with a reply in it and the main window's reader, ready to pop out.
final class _Scene {
  _Scene(this.store, this.agent, this.session, this.hub, this.windows, this.reader, this.output, this.voiceModes);

  final SessionStore store;
  final ScriptedAgent agent;
  final ChatSession session;
  final PopoutHub hub;
  final FakeWindows windows;
  final ReplyReader reader;
  final FakeOutput output;

  /// What the windows told the app about voice mode, for it to keep.
  final List<bool> voiceModes;
}

Future<_Scene> _scene(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(900, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  addTearDown(store.dispose);
  final agent = ScriptedAgent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  unawaited(store.sendText('Fix addNumbers'));
  agent.update(_said('It is done. Really.'));
  await _settle(tester);
  final output = FakeOutput('stand-in', 'Stand-in', hold: true);
  final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices()), player: FakePlayer());
  addTearDown(reader.dispose);
  final voiceModes = <bool>[];
  final windows = FakeWindows();
  final hub = PopoutHub(store, windows, shareInterval: const Duration(milliseconds: 10), reader: reader, onVoiceMode: voiceModes.add);
  addTearDown(hub.dispose);
  return _Scene(store, agent, session, hub, windows, reader, output, voiceModes);
}

/// The window's own program on screen: attached by a link that carries what it asks of the main window.
Future<RemoteReplySpeaker> _show(WidgetTester tester, FakeWindow window) async {
  // Attached on the real clock, as the window's own program would be, so what the owner sends reaches it.
  final (link, view) = (await tester.runAsync(() async {
    final link = ReadAloudLink(window.far);
    return (link, await ChatSessionView.attach(link, clientId: 'window-1', name: 'the pop-out window'));
  }))!;
  final speaker = RemoteReplySpeaker(link);
  addTearDown(speaker.dispose);
  await tester.pumpWidget(PopoutApp(launch: PopoutLaunch(link: 'test', title: window.title), view: view, onClose: window.closeByUser, speaker: speaker));
  await _settle(tester);
  return speaker;
}

void main() {
  group('a conversation in a window of its own', () {
    testWidgets('has Play beside Copy on a reply, which the main window reads, and Stop ends it', (tester) async {
      final scene = await _scene(tester);
      await tester.runAsync(() => scene.hub.popOut(scene.session));
      await _show(tester, scene.windows.opened.single);
      expect(find.byKey(const Key('reply-play')), findsOneWidget);
      expect(find.byTooltip('Copy message'), findsWidgets);

      await tester.tap(find.byKey(const Key('reply-play')));
      await _settle(tester);
      expect(scene.output.spoken, contains('It is done.'), reason: 'the main window\'s voice reads the reply');
      expect(scene.reader.isReading(scene.reader.reading!), isTrue);
      expect(find.byKey(const Key('reply-stop')), findsOneWidget, reason: 'the window shows what the main window is doing');

      await tester.tap(find.byKey(const Key('reply-stop')));
      await _settle(tester);
      expect(scene.reader.busy, isFalse);
      expect(find.byKey(const Key('reply-play')), findsOneWidget);
      expect(scene.output.utterances.every((each) => each.stopped), isTrue);
    });

    testWidgets('has the voice mode switch, which follows that window\'s conversation and is kept', (tester) async {
      final scene = await _scene(tester);
      await tester.runAsync(() => scene.hub.popOut(scene.session));
      await _show(tester, scene.windows.opened.single);
      await tester.tap(find.byKey(const Key('composer-plus')));
      await _settle(tester);
      expect(find.byKey(const Key('voice-mode')), findsOneWidget);

      await tester.tap(find.byKey(const Key('voice-mode')));
      await _settle(tester);
      expect(find.byKey(const Key('voice-mode-on')), findsOneWidget);
      expect(scene.reader.voiceMode, isTrue);
      expect(scene.reader.pinned, scene.session.id);
      expect(scene.voiceModes, [true], reason: 'the choice is kept like the composer\'s');
      expect(scene.output.spoken, isEmpty, reason: 'turning it on reads no earlier reply');

      // A tool call ends the reply being written, so what follows is a new reply: it is read as it arrives, in the main window's
      // voice, a sentence at a time, and the sentence still being written waits for its end.
      scene.agent.update({'sessionUpdate': 'tool_call', 'toolCallId': 't', 'title': 'Read calc.js', 'kind': 'read', 'status': 'completed'});
      scene.agent.update(_said('A second reply. It has two'));
      await _settle(tester);
      await _settle(tester);
      expect(scene.output.spoken, ['A second reply.']);

      // The window's conversation going away hands voice mode back to the conversation the main window shows.
      await tester.runAsync(() => scene.hub.dockBack(scene.session));
      expect(scene.reader.pinned, isNull);
    });

    testWidgets('has neither where the app cannot read aloud', (tester) async {
      final scene = await _scene(tester);
      final bare = PopoutHub(scene.store, FakeWindows(), shareInterval: const Duration(milliseconds: 10));
      addTearDown(bare.dispose);
      await tester.runAsync(() => bare.popOut(scene.session));
      final window = (bare.windows as FakeWindows).opened.single;
      final view = await tester.runAsync(() => ChatSessionView.attach(window.far, clientId: 'window-1', name: 'the pop-out window')) as ChatSessionView;
      await tester.pumpWidget(PopoutApp(launch: PopoutLaunch(link: 'test', title: window.title), view: view, onClose: window.closeByUser));
      await _settle(tester);
      expect(find.byKey(const Key('reply-play')), findsNothing);
      expect(find.byKey(const Key('voice-mode')), findsNothing);
      expect(tester.widget<IconButton>(find.byKey(const Key('composer-plus'))).onPressed, isNull, reason: 'nothing to put in the menu');
      expect(find.text('It is done. Really.'), findsOneWidget);
    });
  });

  group('the link a window reads aloud over', () {
    test('keeps its own messages from the chat and the chat\'s from reading aloud', () async {
      final (near, far) = MemoryViewLink.pair();
      final link = ReadAloudLink(far);
      final chat = <Map<String, Object?>>[], read = <Map<String, Object?>>[];
      link.messages.listen(chat.add);
      link.asks.listen(read.add);
      near.send({'t': 'state', 'n': 1});
      near.send({'t': readAsks, 'op': 'hello'});
      near.send({'t': readTells, 'voice': true});
      await pumpEventQueue();
      expect(chat, [
        {'t': 'state', 'n': 1},
      ]);
      expect(read.map((each) => each['t']), [readAsks, readTells]);
      await link.close();
    });
  });

  group('a child\'s conversation', () {
    testWidgets('has Play on its replies, and voice mode reads them as they arrive while it is shown', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = store.createDraft()..agent = 'Desk';
      final output = FakeOutput('stand-in', 'Stand-in');
      final reader = ReplyReader(voice: () async => (output: output, choices: const SpeakChoices()), player: FakePlayer());
      addTearDown(reader.dispose);
      final task = DelegateTask('c1', 0)
        ..title = 'Share link API'
        ..source = 'codex';
      void hear(String text) {
        task.updates.add(_said(text));
        task.updatesSeen++;
        store.createDraft();
      }

      reader.setVoiceMode(true);
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(const CanoaConfig().look, Brightness.dark),
        home: Scaffold(body: ChildThreadView(store: store, session: session, task: task, reader: reader, renderers: replyRenderers(reader))),
      ));
      await tester.pump();
      expect(output.spoken, isEmpty);

      hear('The route is added.');
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      hear(' Tests next.');
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      // The turn ending completes the last sentence, which is read too.
      task.phase = TaskPhase.completed;
      store.createDraft();
      await tester.runAsync(() => pumpEventQueue());
      await tester.pump();
      await tester.runAsync(() => pumpEventQueue());
      expect(output.spoken, ['The route is added.', 'Tests next.'], reason: 'the child\'s reply is read a sentence at a time as it grows');
      // The app redraws the thread as the store changes; the test does it by drawing it again.
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(const CanoaConfig().look, Brightness.dark),
        home: Scaffold(body: ChildThreadView(store: store, session: session, task: task, reader: reader, renderers: replyRenderers(reader))),
      ));
      await tester.pump();
      expect(find.byKey(const Key('reply-play')), findsOneWidget, reason: 'and it has Play like any reply');
    });
  });
}
