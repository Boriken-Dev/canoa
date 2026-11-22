import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/delegate.dart' show delegateStopMethod;
import 'package:canoa/chat/activity_strip.dart';
import 'package:canoa/chat/inspector_surfaces.dart';
import 'package:canoa/main.dart' show CanoaApp;
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';
import 'fixtures/delegation_scene.dart' show childTitles, delegationCall, sceneAt;

final class _FakeShell implements RemoteTerminal {
  _FakeShell(this.folder);

  @override
  final String folder;
  @override
  String get shell => 'pwsh';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();

  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => _exit.future;
  @override
  Future<void> write(String text) async {}
  @override
  Future<void> resize(int cols, int rows) async {}
  @override
  Future<void> close() async {
    if (!_exit.isCompleted) _exit.complete(0);
    await _out.close();
  }
}

final class _FakeTerminalHost implements TerminalHost {
  final opened = <_FakeShell>[];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final made = _FakeShell(cwd ?? '/work/proj');
    opened.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

Future<({DelegateHost host, ChatSession session, SessionStore store, _FakeTerminalHost termHost})> _openApp(
  WidgetTester tester,
) async {
  await tester.binding.setSurfaceSize(const Size(1280, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final host = DelegateHost();
  final store = SessionStore();
  addTearDown(store.dispose);
  final session = await tester.runAsync(() => host.start(store)) as ChatSession;
  final termHost = _FakeTerminalHost();
  final layout = ShellLayout();
  await tester.pumpWidget(
    CanoaApp(
      store: store,
      layout: layout,
      surfaces: [
        ...builtInSurfaces(),
        TerminalSurface((_) => TerminalOnHost(termHost)),
      ],
    ),
  );
  await tester.pump();
  return (host: host, session: session, store: store, termHost: termHost);
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump();
  await tester.pump(Duration.zero);
}

void main() {
  testWidgets('header state chip says the word of the row state', (tester) async {
    await _openApp(tester);
    final chip = tester.widget<Chip>(find.byKey(const Key('session-state-chip')));
    expect((chip.label as Text).data, 'Idle');
    expect(find.byKey(const Key('session-activity-strip')), findsNothing);
  });

  testWidgets('between turns, live subagents appear in activity strip with open and Stop all controls', (tester) async {
    final env = await _openApp(tester);
    env.host.say(taskNotice('c0', taskEvent('c0', 'WORKING', title: childTitles[0], source: 'claude', at: sceneAt(0))));
    env.host.say(delegationCall('call0', 'c0'));
    await _settle(tester);

    expect(find.byKey(const Key('session-activity-strip')), findsOneWidget);
    expect(find.text('1 live subagent'), findsOneWidget);

    // Tapping the subagents chip opens SubagentsSurface and shows its running count badge.
    await tester.tap(find.byKey(const Key('activity-subagents')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('tab subagents')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const ValueKey('tab subagents')), matching: find.text('1')), findsOneWidget);

    // Tapping Stop all cancels the running subagent and clears the strip.
    await tester.tap(find.byKey(const Key('activity-stop-subagents')));
    await _settle(tester);
    expect(env.host.sentOf(delegateStopMethod), hasLength(1));
    expect(find.byKey(const Key('session-activity-strip')), findsNothing);
  });

  testWidgets('running ACP terminal command appears in strip and links to Details above Outputs', (tester) async {
    final env = await _openApp(tester);
    env.host.say({
      'jsonrpc': '2.0',
      'method': 'session/update',
      'params': {
        'sessionId': 's1',
        'update': {
          'sessionUpdate': 'terminal_update',
          'terminalId': 'term-1',
          'command': 'npm run dev',
          'output': {'data': base64Encode(utf8.encode('listening\n'))},
        },
      },
    });
    env.host.say({
      'jsonrpc': '2.0',
      'method': 'session/update',
      'params': {
        'sessionId': 's1',
        'update': {
          'sessionUpdate': 'tool_call',
          'toolCallId': 'w1',
          'title': 'Write file',
          'kind': 'edit',
          'status': 'completed',
          'content': [
            {'type': 'diff', 'path': '/work/proj/out.txt', 'newText': 'text\n'},
          ],
        },
      },
    });
    await _settle(tester);

    expect(find.byKey(const Key('activity-commands')), findsOneWidget);
    expect(find.text('1 command'), findsOneWidget);

    await tester.tap(find.byKey(const Key('activity-commands')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('tab details')), findsOneWidget);
    expect(find.text('Background tasks · 1 running'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('Background tasks · 1 running')).dy,
      lessThan(tester.getTopLeft(find.text('Outputs · 1')).dy),
    );
  });

  testWidgets('interactive terminal shell appears in strip and its tab shows the running count', (tester) async {
    final env = await _openApp(tester);

    // Open interactive terminal via bottom panel launcher
    await tester.tap(find.byKey(const ValueKey('layout-bottom')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('launch terminal')));
    await _settle(tester);

    expect(env.termHost.opened, hasLength(1));
    expect(find.byKey(const Key('activity-shells')), findsOneWidget);
    expect(find.text('1 shell running'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const ValueKey('tab terminal:1')), matching: find.text('1')), findsOneWidget);

    // Closing the shell closes the tab cleanly.
    await tester.tap(find.byKey(const ValueKey('close terminal:1')));
    await _settle(tester);
    expect(find.byKey(const Key('activity-shells')), findsNothing);
    expect(SessionActivityStrip.runningCommandsOf(env.session), 0);
  });

  testWidgets('subagent asking for input highlights warning in activity strip alongside shells', (tester) async {
    final env = await _openApp(tester);
    env.host.say(taskNotice('c0', taskEvent('c0', 'WORKING', title: 'Child task', source: 'claude', at: sceneAt(0))));
    env.host.ask(10, 'Run dangerous command', task: 'c0');
    await _settle(tester);

    expect(find.byKey(const Key('session-activity-strip')), findsOneWidget);
    expect(find.text('Subagent needs input · 1'), findsOneWidget);
    expect(find.byKey(const Key('activity-subagents')), findsOneWidget);
  });
}
