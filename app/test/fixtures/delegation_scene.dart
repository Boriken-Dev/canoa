// A conversation with children handed out, as the delegation tests and scenes share it: an agent in memory, the app drawn over it,
// and a clock the test moves.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/delegate.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'delegate_host.dart';

/// Every step of a turn as a card of its own, so a call that is not folded into the row shows as one.
const verboseConfig = CanoaConfig(density: ChatDensity.verbose);

final sceneStart = DateTime.utc(2026, 11, 13, 10);

String sceneAt(int seconds) => sceneStart.add(Duration(seconds: seconds)).toIso8601String();

/// A call of a delegation tool, as an agent reports it: the title names the server, the result names the task.
Map<String, Object?> delegationCall(String id, String task, {String server = 'canoa-delegate', String tool = 'delegate_task', bool result = true}) => {
  'jsonrpc': '2.0',
  'method': 'session/update',
  'params': {
    'sessionId': 's1',
    'update': {
      'sessionUpdate': 'tool_call',
      'toolCallId': id,
      'title': '$server: $tool',
      'kind': 'other',
      'status': 'completed',
      if (result)
        'content': [
          {'type': 'content', 'content': {'type': 'text', 'text': jsonEncode({'taskId': task, 'state': 'working'})}},
        ],
    },
  },
};

Map<String, Object?> childUpdate(String task, Map<String, Object?> update) => {
  'jsonrpc': '2.0',
  'method': delegateEventMethod,
  'params': {'sessionId': 's1', 'taskId': task, 'update': update},
};

const childTitles = ['Dashboard share button', 'Share link API', 'Share link tests', 'Share link docs'];

final class Scene {
  Scene(this.host, this.session, this.store, this.tester);
  final DelegateHost host;
  final ChatSession session;
  final SessionStore store;
  final WidgetTester tester;
  DateTime now = sceneStart.add(const Duration(seconds: 25));

  Future<void> settle() async {
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
  }

  Future<void> say(Map<String, Object?> message) async {
    host.say(message);
    await settle();
  }

  /// Four children of different sources, all working since the turn began, and the calls that handed them out.
  Future<void> fourChildren() async {
    for (final (i, title) in childTitles.indexed) {
      final id = 'c$i';
      host.say(taskNotice(id, taskEvent(id, 'WORKING', title: title, source: const ['claude', 'codex', 'codex', 'grok'][i], at: sceneAt(0))));
      host.say(delegationCall('call$i', id));
    }
    await settle();
  }

  Future<void> end(String id, String state, {int at = 20, String? note}) => say(taskNotice(id, statusEvent(state, at: sceneAt(at), note: note)));

  /// A turn that is still running, which a message of the person starts.
  Future<void> turn() async {
    unawaited(store.sendText('Build the share link'));
    await settle();
  }
}

Future<Scene> openScene(WidgetTester tester, {Size size = const Size(1100, 900)}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore();
  final host = DelegateHost();
  final session = await tester.runAsync(() => host.start(store)) as ChatSession;
  final scene = Scene(host, session, store, tester);
  store.clock = () => scene.now;
  await tester.pumpWidget(CanoaApp(store: store, config: verboseConfig));
  await tester.pump();
  return scene;
}

