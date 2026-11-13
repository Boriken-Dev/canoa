import 'dart:async';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart' show VerifiedAgent;
import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_acp/page.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/auth/peer_proof.dart';
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
    expect(find.text('Welcome to Canoa'), findsOneWidget);
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
    testWidgets('a permission waits in the conversation until the person answers', (tester) async {
      final store = SessionStore();
      final session = store.addProviderSession(title: 'T', agent: 'Agent', peerId: 'local:a', remoteSessionId: 'r');
      final silent = StreamController<String>();
      addTearDown(silent.close);
      final client = VerifiedAgent.fromChild(label: 'a', messages: silent.stream, send: (_) async {}, close: () async {});
      store.attachVerifiedClient(session.id, client, record: false);
      store.select(session.id);
      final answer = store.askPermission(const AcpPermissionRequest(sessionId: 'r', toolCallId: 't', title: 'Delete a file', options: [
        AcpPermissionOption(optionId: 'yes', name: 'Allow', kind: 'allow_once'),
        AcpPermissionOption(optionId: 'no', name: 'Refuse', kind: 'reject_once'),
      ]), client);
      await tester.pumpWidget(CanoaApp(store: store));
      await tester.pump();
      expect(find.textContaining('to delete a file?'), findsOneWidget);
      await tester.tap(find.byKey(const Key('permission-deny')));
      await tester.pump();
      expect(find.textContaining('to delete a file?'), findsNothing);
      expect(await answer, isA<AcpSelect>().having((select) => select.optionId, 'optionId', 'no'));
    });
  });

  group('a command a person gives', () {
    test('is split the way they meant', () {
      final parsed = parseCommand('  my-agent --acp "a path/with space" \'x y\' ""');
      expect(parsed?.program, 'my-agent');
      expect(parsed?.args, ['--acp', 'a path/with space', 'x y', '']);
      expect(parsed?.label, 'my-agent');
    });

    test('reads its usage from the preset it is, whatever the directory or extension', () {
      expect(usageFor(parseCommand('agy acp')!), 'antigravity');
      expect(usageFor(parseCommand(r'"C:\Users\me\AppData\Local\agy\bin\agy.exe" acp')!), 'antigravity');
      expect(usageFor(parseCommand('agy chat')!), isNull);
      expect(usageFor(parseCommand('my-agent --acp')!), isNull);
    });

    test('is refused when it is not one', () {
      expect(parseCommand(''), isNull);
      expect(parseCommand('   '), isNull);
      expect(parseCommand('"open quote'), isNull);
      expect(parseCommand('""'), isNull);
    });
  });

  testWidgets('an agent added by hand connects only once the person names the method its host proves by', (tester) async {
    await tester.binding.setSurfaceSize(const Size(900, 1200));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final targets = <ConnectTarget>[];
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () => showCanoaConnectDialog(context, (target) async => targets.add(target),
        methodUnavailable: (_) async => null, localPeerId: (method) async => 'sha256:own-${method.id}'),
      child: const Text('open'),
    ))));
    await tester.tap(find.text('open'));
    await tester.pump();
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextField, 'WebSocket WSS endpoint'), 'wss://agent.example/acp');
    await tester.enterText(find.widgetWithText(TextField, 'Pinned server peer ID'), 'sha256:${'a' * 64}');
    await tester.tap(find.text('Connect'));
    await tester.pump();
    expect(find.text('Choose the method the host proves by: canoa serve prints it.'), findsOneWidget);
    expect(targets, isEmpty, reason: 'no method is assumed');

    await tester.tap(find.text('WebCrypto'));
    await tester.pump();
    await tester.tap(find.text('Connect'));
    await tester.pump();
    await tester.pump();
    expect(targets.single.method, PeerMethod.webcrypto);
    expect(targets.single.peerId, 'sha256:${'a' * 64}');
  });

  group('agents in canoa.cfg', () {
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
    await tester.pumpWidget(CanoaApp(store: SessionStore(), shell: shell));
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
    await tester.pumpWidget(CanoaApp(store: SessionStore(), shell: shell, host: host));
    await tester.tap(find.byTooltip('Canoa settings'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
    expect(find.text('Quit Canoa'), findsNothing, reason: 'the tray and the menu quit, not Settings');
    expect(find.text('Save'), findsNothing);
    await tester.pageBack();
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Hosting'));
    await tester.pumpAndSettle();
    expect(find.text('Agents this computer hosts'), findsOneWidget, reason: 'Hosting is a section of Settings');
    await tester.pageBack();
    await tester.pumpAndSettle();

    await host.apply(configPath: '/c/canoa.cfg');
    await shell.beforeQuit!();
    expect(tool.children.single.inputClosed, isTrue, reason: 'the host and its agents end with the app');
  });

  testWidgets('settings open the profiles, each agent offered with its effective profile', (tester) async {
    final dir = Directory.systemTemp.createTempSync('canoa-profiles');
    addTearDown(() => dir.deleteSync(recursive: true));
    await tester.binding.setSurfaceSize(const Size(1200, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final config = CanoaConfig.parse('[profile]\neffort = "high"\n[agents.Mine.profile]\nmodel = "m"\n');
    await tester.pumpWidget(CanoaApp(
      store: SessionStore(), config: config, configPath: '${dir.path}/canoa.cfg',
      host: linkedHost((name) => throw StateError(name)),
    ));
    await tester.pump();
    await tester.tap(find.byTooltip('Canoa settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-nav-profiles')));
    await tester.pumpAndSettle();
    expect(find.text('Effective profile'), findsOneWidget);
    expect(find.text('user'), findsOneWidget, reason: 'effort comes from the person\'s own layer');
    await tester.tap(find.byKey(const Key('profile-agent')));
    await tester.pumpAndSettle();
    expect(find.text('Claude').last, findsOneWidget);
    expect(find.text('Mine').last, findsOneWidget, reason: 'a profile alone names an agent to show');
  });
}

void _staleDialogs() {
  testWidgets('a dialog left open never saves back what another one changed meanwhile', (tester) async {
    final dir = Directory.systemTemp.createTempSync('canoa-stale');
    addTearDown(() async {
      // Something of the dialogs' background work may still hold a file in it (errno 32 on Windows): the delete is repeated.
      final closed = DateTime.now();
      while (true) {
        try {
          return dir.deleteSync(recursive: true);
        } on FileSystemException {
          if (DateTime.now().difference(closed) > const Duration(seconds: 60)) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
    });
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    final settings = ValueNotifier<int>(0);
    final hosting = ValueNotifier<int>(0);
    final written = <CanoaConfig>[];
    await tester.binding.setSurfaceSize(const Size(1200, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final config = const CanoaConfig().withHostAgent(name: 'old', program: 'agent', listen: '127.0.0.1:7801');
    await tester.pumpWidget(CanoaApp(
      store: SessionStore(),
      config: config,
      shell: _FakeShell(),
      host: host,
      settingsSignal: settings,
      hostingSignal: hosting,
      configPath: '${dir.path}/canoa.cfg',
      writeConfig: (changed) async => written.add(changed),
    ));

    // Settings from the tray, then Hosting in it: the agent removed there stays removed when another setting changes.
    settings.value++;
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-nav-hosting')));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Stop hosting old'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Stop hosting'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(written.last.hostEntries, isEmpty);
    await tester.tap(find.byKey(const Key('settings-nav-appearance')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('settings-brightness')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Light').last);
    await tester.pumpAndSettle();
    expect(written.last.brightness, 'light');
    expect(written.last.hostEntries, isEmpty, reason: 'Settings wrote its own copy of [host] back');

    // A look chosen there stays when Hosting adds an agent.
    await tester.tap(find.byKey(const Key('settings-nav-hosting')));
    await tester.pumpAndSettle();
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

    // One Settings at a time.
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
  Future<void> showWaiting(int count) async {}
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
