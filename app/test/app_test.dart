import 'package:canoa/acp/agent_commands.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_acp/page.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_host.dart';

final class _FakeProvider implements AcpPage {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  _staleDialogs();
  testWidgets('shows Canoa multi-session chat shell', (tester) async {
    final store = SessionStore();
    await tester.pumpWidget(CanoaApp(store: store));
    expect(find.text('Canoa'), findsWidgets);
    expect(find.text('Getting started'), findsWidgets);
    expect(find.textContaining('Preview'), findsWidgets);
    expect(find.byIcon(Icons.add_comment_outlined), findsOneWidget);
    store.dispose();
  });

  group('agents from the browser', () {
    Future<void> open(WidgetTester tester, Future<AcpPage?> provider, UseBrowserAgents use) async {
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () => showCanoaConnectDialog(context, (_) async {}, browserProvider: provider, onUseBrowserAgents: use),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pump();
      await tester.pump();
    }

    const label = 'Use an agent from this browser';

    testWidgets('no provider, no button', (tester) async {
      await open(tester, Future.value(null), (_, {required cwd, required title}) async {});
      expect(find.text(label), findsNothing);
    });

    testWidgets('a provider offers it, and the dialog closes once the agents are in', (tester) async {
      AcpPage? used;
      final provider = _FakeProvider();
      await open(tester, Future.value(provider), (acp, {required cwd, required title}) async => used = acp);
      await tester.tap(find.text(label));
      await tester.pump();
      await tester.pump();
      expect(used, same(provider));
      expect(find.text(label), findsNothing);
    });

    testWidgets('a refusal stays in the dialog and says so', (tester) async {
      await open(tester, Future.value(_FakeProvider()), (_, {required cwd, required title}) async {
        throw const AcpPageFailure('NotAllowedError', 'this page may not do that');
      });
      await tester.tap(find.text(label));
      await tester.pump();
      await tester.pump();
      expect(find.textContaining('NotAllowedError'), findsOneWidget);
    });
  });

  group('agents on this computer', () {
    testWidgets('only what is installed is offered, and running one closes the dialog', (tester) async {
      AgentCommand? ran;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () => showCanoaConnectDialog(
          context,
          (_) async {},
          onRunLocalAgent: (command, {required cwd, required title}) async => ran = command,
          localAgents: const [
            AgentCommand(label: 'Here', program: 'here'),
            AgentCommand(label: 'Missing', program: 'missing'),
          ],
          checkInstalled: (command) async => command.label == 'Here',
        ),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pump();
      await tester.pump();
      expect(find.text('Run Here'), findsOneWidget);
      expect(find.text('Run Missing'), findsNothing);
      await tester.tap(find.text('Run Here'));
      await tester.pump();
      await tester.pump();
      expect(ran?.label, 'Here');
      expect(find.text('Run Here'), findsNothing);
    });

    testWidgets('a permission waits in the conversation until the person answers', (tester) async {
      final store = SessionStore(includePreview: false);
      final session = store.addProviderSession(title: 'T', agent: 'Agent', peerId: 'local:a', remoteSessionId: 'r');
      store.select(session.id);
      final answer = store.askPermission(const AcpPermissionRequest(sessionId: 'r', toolCallId: 't', title: 'Delete a file'));
      await tester.pumpWidget(CanoaApp(store: store));
      await tester.pump();
      expect(find.text('Delete a file'), findsOneWidget);
      await tester.tap(find.text('Refuse'));
      await tester.pump();
      expect(find.text('Delete a file'), findsNothing);
      expect(await answer, isA<AcpDeny>());
    });
  });

  group('a command a person gives', () {
    test('is split the way they meant', () {
      final parsed = parseCommand('  my-agent --acp "a path/with space" \'x y\' ""');
      expect(parsed?.program, 'my-agent');
      expect(parsed?.args, ['--acp', 'a path/with space', 'x y', '']);
      expect(parsed?.label, 'my-agent');
    });

    test('is refused when it is not one', () {
      expect(parseCommand(''), isNull);
      expect(parseCommand('   '), isNull);
      expect(parseCommand('"open quote'), isNull);
      expect(parseCommand('""'), isNull);
    });

    testWidgets('typed in the dialog runs, and a bad one says so', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1200));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      AgentCommand? ran;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () => showCanoaConnectDialog(
          context,
          (_) async {},
          onRunLocalAgent: (command, {required cwd, required title}) async => ran = command,
          localAgents: const [],
          checkInstalled: (_) async => true,
        ),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pump();
      await tester.pump();
      final field = find.widgetWithText(TextField, 'Or run any ACP agent, by its command');
      await tester.enterText(field, '"oops');
      await tester.tap(find.byTooltip('Run this command'));
      await tester.pump();
      expect(find.textContaining('check its quotes'), findsOneWidget);
      expect(ran, isNull);
      await tester.enterText(field, 'my-agent --acp');
      await tester.tap(find.byTooltip('Run this command'));
      await tester.pump();
      await tester.pump();
      expect(ran?.args, ['--acp']);
    });
  });

  testWidgets('an agent added by hand connects only once the person names the method its host proves by', (tester) async {
    await tester.binding.setSurfaceSize(const Size(900, 1200));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final targets = <ConnectTarget>[];
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () => showCanoaConnectDialog(context, (target) async => targets.add(target)),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pump();
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextField, 'WebSocket WSS endpoint'), 'wss://agent.example/acp');
    await tester.enterText(find.widgetWithText(TextField, 'Pinned server peer ID'), 'sha256:${'a' * 64}');
    await tester.tap(find.text('Connect'));
    await tester.pump();
    expect(find.text('Choose the method the host proves by: canoa-host serve prints it.'), findsOneWidget);
    expect(targets, isEmpty, reason: 'no method is assumed');

    await tester.tap(find.text('webcrypto'));
    await tester.pump();
    await tester.tap(find.text('Connect'));
    await tester.pump();
    await tester.pump();
    expect(targets.single.method, 'webcrypto');
    expect(targets.single.peerId, 'sha256:${'a' * 64}');
  });

  group('agents in Canoa.cfg', () {
    test('survive a round trip, with names that need quoting', () {
      const config = CanoaConfig(agents: [AgentCommand(label: 'My "agent"', program: r'C:\tools\agent.exe', args: ['--acp', 'a b'])]);
      final read = CanoaConfig.parse(config.encode());
      expect(read.agents.single.label, 'My "agent"');
      expect(read.agents.single.program, r'C:\tools\agent.exe');
      expect(read.agents.single.args, ['--acp', 'a b']);
    });

    test('are refused when they are not right', () {
      expect(() => CanoaConfig.parse('[agents.a]\nargs = []\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[agents.a]\nprogram = 7\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[agents.a]\nprogram = "x"\nargs = [1]\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[agents.a]\nprogram = "x"\nextra = 1\n'), throwsFormatException);
    });
  });

  testWidgets('quitting has something to run, and leaving takes it back', (tester) async {
    final shell = _FakeShell();
    await tester.pumpWidget(CanoaApp(store: SessionStore(includePreview: false), shell: shell));
    expect(shell.beforeQuit, isNotNull);
    await shell.beforeQuit!();
    await tester.pumpWidget(const SizedBox());
    expect(shell.beforeQuit, isNull);
  });

  testWidgets('the rail opens settings and hosting, and quitting stops the host', (tester) async {
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    final shell = _FakeShell();
    await tester.binding.setSurfaceSize(const Size(1200, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(CanoaApp(store: SessionStore(includePreview: false), shell: shell, host: host));
    await tester.tap(find.byTooltip('Canoa settings'));
    await tester.pumpAndSettle();
    expect(find.text('Look'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Hosting'));
    await tester.pumpAndSettle();
    expect(find.text('Agents this computer hosts'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();

    await host.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    await shell.beforeQuit!();
    expect(tool.children.single.inputClosed, isTrue, reason: 'the host and its agents end with the app');
  });
}

void _staleDialogs() {
  testWidgets('a dialog left open never saves back what another one changed meanwhile', (tester) async {
    final dir = Directory.systemTemp.createTempSync('canoa-stale');
    addTearDown(() => dir.deleteSync(recursive: true));
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    final settings = ValueNotifier<int>(0);
    final hosting = ValueNotifier<int>(0);
    final written = <CanoaConfig>[];
    await tester.binding.setSurfaceSize(const Size(1200, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final config = const CanoaConfig().withHostAgent(name: 'old', program: 'agent', listen: '127.0.0.1:7801');
    await tester.pumpWidget(CanoaApp(
      store: SessionStore(includePreview: false),
      config: config,
      shell: _FakeShell(),
      host: host,
      settingsSignal: settings,
      hostingSignal: hosting,
      configPath: '${dir.path}/Canoa.cfg',
      writeConfig: (changed) async => written.add(changed),
    ));

    // Settings from the tray, then Hosting over it: the agent removed there stays removed when Settings is saved.
    settings.value++;
    await tester.pumpAndSettle();
    hosting.value++;
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Stop hosting old'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Stop hosting'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(written.last.hostEntries, isEmpty);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(written.last.hostEntries, isEmpty, reason: 'Settings saved its own copy of [host] back');

    // Hosting open, then Settings over it: a look chosen there stays when Hosting adds an agent.
    hosting.value++;
    await tester.pumpAndSettle();
    settings.value++;
    await tester.pumpAndSettle();
    await tester.tap(find.text('Dark'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Light').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(written.last.brightness, 'light');
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'new');
    await tester.enterText(fields.at(1), 'agent');
    final before = written.length;
    await tester.tap(find.text('Add'));
    // The port and the check are real input and output, which run only outside the test's clock.
    for (var turn = 0; turn < 40 && written.length == before; turn++) {
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
      await tester.pump();
    }
    await tester.pumpAndSettle();
    expect(written.last.hostEntries.map((entry) => entry.name), ['new']);
    expect(written.last.brightness, 'light', reason: 'Hosting saved its own copy of the look back');

    // One Hosting at a time.
    hosting.value++;
    await tester.pumpAndSettle();
    expect(find.text('Agents this computer hosts'), findsOneWidget);
  });
}

final class _FakeShell implements DesktopShell {
  @override
  Future<void> Function()? beforeQuit;
  @override
  bool get isDesktop => true;
  @override
  Future<void> quit() async {}
  @override
  Future<void> update(CanoaConfig config) async {}
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
