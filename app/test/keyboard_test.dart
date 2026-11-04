// Each shortcut, the palette with the agent's own commands, the help sheet, and Up recalling the last prompt.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/changes.dart';
import 'package:canoa/chat/palette.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];

  Future<ChatSession> start(SessionStore store, String title) async {
    final client = VerifiedAgent.fromChild(
      label: title,
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        switch (message['method']) {
          case 'initialize':
            reply(id, {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}});
          case 'session/new':
            reply(id, {'sessionId': 's1'});
          case 'session/prompt':
            update({'sessionUpdate': 'user_message', 'messageId': 'u${sent.length}', 'content': message['params']['prompt']});
            reply(id, {'messageId': 'u${sent.length}'});
          default:
            reply(id, <String, Object?>{});
        }
      },
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: title, agent: title, peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 's1', 'update': body},
  }));

  void ask(int id) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1', 'title': 'Edit a.txt',
      'subject': {'type': 'tool_call', 'toolCall': {'toolCallId': 't$id'}},
      'options': [
        {'optionId': 'always', 'name': 'Always', 'kind': 'allow_always'},
        {'optionId': 'yes', 'name': 'Yes', 'kind': 'allow_once'},
        {'optionId': 'no', 'name': 'No', 'kind': 'reject_once'},
      ],
    },
  }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump();
}

/// Three conversations, the first one chosen, each with its own agent.
Future<(List<_Agent>, List<ChatSession>, SessionStore)> _three(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(1280, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final store = SessionStore(includePreview: false);
  final agents = [_Agent(), _Agent(), _Agent()];
  final sessions = <ChatSession>[];
  for (final (index, agent) in agents.indexed) {
    sessions.add(await tester.runAsync(() => agent.start(store, 'Agent $index')) as ChatSession);
  }
  // The list holds the newest first.
  store.select(store.sessions.first.id);
  await tester.pumpWidget(CanoaApp(store: store));
  await tester.pump();
  return (agents, sessions, store);
}

Future<void> _press(WidgetTester tester, LogicalKeyboardKey key, {bool shift = false}) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(key);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  // A working session's mark turns for ever, so this waits long enough for a dialog to open rather than for stillness.
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

Finder get _composer => find.byWidgetPredicate((widget) => widget is TextField && widget.decoration?.hintText == 'Message agent');

void main() {
  testWidgets('Ctrl+N starts a new session', (tester) async {
    final (_, _, store) = await _three(tester);
    await _press(tester, LogicalKeyboardKey.keyN);
    expect(store.sessions, hasLength(4));
    expect(store.selected?.status, SessionStatus.preview);
  });

  testWidgets('Ctrl+2 goes to the second session, Ctrl+Tab to the next and Ctrl+Shift+Tab back', (tester) async {
    final (_, _, store) = await _three(tester);
    await _press(tester, LogicalKeyboardKey.digit2);
    expect(store.selectedId, store.sessions[1].id);
    await _press(tester, LogicalKeyboardKey.tab);
    expect(store.selectedId, store.sessions[2].id);
    await _press(tester, LogicalKeyboardKey.tab);
    expect(store.selectedId, store.sessions[0].id, reason: 'it goes round');
    await _press(tester, LogicalKeyboardKey.tab, shift: true);
    expect(store.selectedId, store.sessions[2].id);
  });

  testWidgets('Ctrl+L puts the cursor in the composer', (tester) async {
    await _three(tester);
    expect(tester.widget<TextField>(_composer).focusNode?.hasFocus, isFalse);
    await _press(tester, LogicalKeyboardKey.keyL);
    expect(tester.widget<TextField>(_composer).focusNode?.hasFocus, isTrue);
  });

  testWidgets('Ctrl+. stops the turn', (tester) async {
    final (agents, _, store) = await _three(tester);
    agents.last.update({'sessionUpdate': 'state_update', 'state': 'running'});
    await _settle(tester);
    expect(store.selected?.status, SessionStatus.working);
    await _press(tester, LogicalKeyboardKey.period);
    await _settle(tester);
    expect(agents.last.sent.where((message) => message['method'] == 'session/cancel'), hasLength(1));
  });

  testWidgets('Ctrl+Shift+A allows once and Ctrl+Shift+R refuses once, never an always', (tester) async {
    final (agents, _, _) = await _three(tester);
    agents.last.ask(7);
    agents.last.ask(8);
    await _settle(tester);
    await _press(tester, LogicalKeyboardKey.keyA, shift: true);
    await _settle(tester);
    expect(agents.last.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'yes'}});
    await _press(tester, LogicalKeyboardKey.keyR, shift: true);
    await _settle(tester);
    expect(agents.last.answerTo(8)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'no'}});
  });

  testWidgets('Ctrl+Shift+D opens the changes pane', (tester) async {
    final (agents, _, _) = await _three(tester);
    agents.last.update({'sessionUpdate': 'tool_call_update', 'toolCallId': 'e', 'title': 'Edit', 'status': 'completed',
      'content': [{'type': 'diff', 'path': 'a.txt', 'oldText': 'a\n', 'newText': 'b\n'}]});
    await _settle(tester);
    await _press(tester, LogicalKeyboardKey.keyD, shift: true);
    expect(find.byType(ChangesPane), findsOneWidget);
  });

  testWidgets('Ctrl+K lists actions, sessions and the agent\'s own commands, and one chosen runs', (tester) async {
    final (agents, _, _) = await _three(tester);
    agents.last.update({'sessionUpdate': 'available_commands_update', 'availableCommands': [
      {'name': 'review', 'description': 'Review the change'},
      {'name': 'compact', 'description': 'Make room'},
    ]});
    await _settle(tester);
    await _press(tester, LogicalKeyboardKey.keyK);
    expect(find.byType(CommandPalette), findsOneWidget);
    expect(find.text('/review'), findsOneWidget);
    expect(find.text('New session'), findsOneWidget);
    expect(find.text('Go to Agent 0'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('palette-search')), 'rev');
    await tester.pumpAndSettle();
    expect(find.text('/compact'), findsNothing);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.byType(CommandPalette), findsNothing);
    expect(tester.widget<TextField>(_composer).controller?.text, '/review ');
  });

  testWidgets('Ctrl+/ shows every shortcut', (tester) async {
    await _three(tester);
    await _press(tester, LogicalKeyboardKey.slash);
    expect(find.text('Keyboard shortcuts'), findsOneWidget);
    for (final shortcut in shortcutHelp) {
      expect(find.text(shortcut.what), findsOneWidget);
    }
  });

  testWidgets('Up in an empty composer brings back the last message sent here, and moves as usual otherwise', (tester) async {
    final (_, _, store) = await _three(tester);
    await tester.runAsync(() => store.sendText('run the tests'));
    await _settle(tester);
    await tester.tap(_composer);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
    await tester.pump();
    expect(tester.widget<TextField>(_composer).controller?.text, 'run the tests');
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
    await tester.pump();
    expect(tester.widget<TextField>(_composer).controller?.text, 'run the tests', reason: 'with text there, Up moves the cursor');
  });
}
