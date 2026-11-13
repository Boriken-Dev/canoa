// A conversation popped out into a second window, which is a second Flutter engine of the same program: the window attaches to the
// conversation the main window holds, answers what its agent waits on, writes to it, and docks back, with the turn and the
// request that waits unchanged.
//
//   flutter drive --driver=test_driver/integration_test.dart --target=integration_test/popout_test.dart -d windows   (or -d linux)
//
// Not `flutter test`: its generated entry waits for the test server before it calls `main`, and a second engine inherits that entry,
// so the window's `main` never runs. `flutter drive` runs this file's `main` in every engine.
//
// The window's engine runs this file's `main` with the arguments the plugin gives it, where the window drives itself: its title
// says how. `answer` answers the oldest request waiting and sends a message; `watch` says in a message who settled a request it was
// showing. Everything the test checks is what reaches the main window across the two engines.
import 'dart:async';

import 'package:canoa/popout/platform.dart' as platform;
import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_launch.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show Settlement;
import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import '../test/fixtures/scripted_agent.dart';

Future<void> main(List<String> args) async {
  final launch = platform.popoutLaunch(args);
  if (launch != null) return _window(launch);
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('a popped-out window keeps one model with the main window: request, turn, answers and messages cross the engines', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = ScriptedAgent();
    final session = (await agent.start(store));
    // Under v1 a prompt answers when its turn ends, which this one never does.
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    agent.ask(5, 'Edit a.txt');
    agent.ask(6, 'Edit b.txt');
    await _until(() => session.pendingPermissions.length == 2, 'the agent\'s requests to be waiting');
    expect(session.status, SessionStatus.working);

    final hub = PopoutHub(store, platform.createPopoutWindows(), shareInterval: const Duration(milliseconds: 20));
    addTearDown(hub.dispose);
    expect(hub.available, isTrue);
    final windowsBefore = (await WindowController.getAll()).length;

    // First window: it answers the oldest request and writes.
    session.title = 'answer';
    await hub.popOut(session);
    expect(hub.isPoppedOut(session), isTrue);
    await _until(() => hub.viewsOf(session).isNotEmpty, 'the window to attach');
    await _until(() => agent.answerTo(5) != null, 'the window\'s answer to reach the agent');
    expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
    await _until(() => session.queue.any((m) => m.text == 'from the window'), 'the window\'s message to wait in the queue');
    expect(session.pendingPermissions, hasLength(1), reason: 'the second request still waits');
    expect(agent.answerTo(6), isNull);

    // Docked back: the window goes, the conversation is as it was.
    await hub.dockBack(session);
    await _until(() async => (await WindowController.getAll()).length == windowsBefore, 'the window to close');
    expect(hub.isPoppedOut(session), isFalse);
    expect(session.status, SessionStatus.working, reason: 'the turn was not touched');
    expect(session.pendingPermissions.single.title, 'Edit b.txt');

    // Second window: it only watches; the main window answers the request, and the window says it saw it settle.
    session.title = 'watch';
    await hub.popOut(session);
    await _until(() => hub.viewsOf(session).isNotEmpty, 'the second window to attach');
    // Let the window show the request before it is answered here.
    await Future<void>.delayed(const Duration(seconds: 2));
    store.choosePermission(session, session.pendingPermissions.first.options.last);
    await _until(() => session.queue.any((m) => m.text == 'settled by main'), 'the window to tell it saw the request settled here');
    expect(agent.answerTo(6)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});

    await hub.dockBack(session);
    await _until(() async => (await WindowController.getAll()).length == windowsBefore, 'the second window to close');
    expect(session.status, SessionStatus.working);
  }, timeout: const Timeout(Duration(minutes: 6)));
}

Future<void> _until(FutureOr<bool> Function() done, String what, {Duration within = const Duration(seconds: 90)}) async {
  final end = DateTime.now().add(within);
  while (!await done()) {
    if (DateTime.now().isAfter(end)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

/// A pop-out window that drives itself, in the engine the main window's plugin started.
Future<void> _window(PopoutLaunch launch) async {
  WidgetsFlutterBinding.ensureInitialized();
  await platform.preparePopoutWindow(launch);
  final view = await attachPopout(launch);
  runApp(PopoutApp(launch: launch, view: view));
  final session = view.session;
  var answered = false, told = false;
  void act() {
    if (launch.title == 'answer' && !answered && session.pendingPermissions.isNotEmpty) {
      answered = true;
      final question = session.pendingPermissions.first;
      view.choosePermission(question, question.options.first);
    }
    if (launch.title == 'answer' && answered && !told && session.settledPermissions.isNotEmpty) {
      told = true;
      unawaited(view.send('from the window'));
    }
    if (launch.title == 'watch' && !told) {
      final settled = session.settledPermissions.where((q) => q.outcome?.how == Settlement.answered).firstOrNull;
      if (settled != null) {
        told = true;
        unawaited(view.send('settled by ${settled.outcome!.by}'));
      }
    }
  }

  session.addListener(act);
  // What was waiting when it attached is already there: nothing more is sent until something changes.
  act();
}
