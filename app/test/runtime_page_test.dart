// A runtime's icon on its tile and its page, over a scripted host (test/fixtures/store_link.dart): the page lays out what the store
// says, the version installed and the newest, where it is, where it downloads from, what uses it, and Install, Update and Remove
// through the host's plan; and the pages and tiles are drawn at phone and desktop width.
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/profile/profile.dart';
import 'package:canoa/store/programs.dart';
import 'package:canoa/store/runtime_page.dart';
import 'package:canoa/store/store_extension.dart';
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_runtimes.dart';
import 'package:canoa/store/store_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;
import 'fixtures/store_link.dart';

final _digest = 'c0' * 32;

FakeStoreHost _host() => FakeStoreHost([
  FakeStore(
    'node_store',
    title: 'Node.js',
    runtime: true,
    summary: 'A JavaScript runtime: what MCP servers and agents written for it start with node, npm or npx.',
    icon: 'nodes',
    homepage: 'https://nodejs.org',
    commands: ['node', 'npm', 'npx', 'corepack'],
    endpoints: ['https://nodejs.org/dist/'],
    copies: [copyOf('24.1.0', path: '/programs/node/24.1.0/bin/node')],
    latest: '24.2.0',
    plan: {
      'update': planOf(
        'update',
        '24.2.0',
        from: '24.1.0',
        bytes: 31 * 1024 * 1024,
        downloads: [
          {'url': 'https://nodejs.org/dist/v24.2.0/node-v24.2.0-win-arm64.zip', 'bytes': 31 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': _digest}},
        ],
      ),
      'uninstall': planOf('uninstall', '24.1.0'),
    },
  ),
  FakeStore(
    'python_store',
    title: 'Python',
    runtime: true,
    summary: 'A Python interpreter: what MCP servers and agents written in Python start with python or pip.',
    icon: 'indent',
    homepage: 'https://www.python.org',
    commands: ['python', 'python3', 'pip', 'pip3'],
    endpoints: ['https://github.com/astral-sh/python-build-standalone/releases/'],
    latest: '3.13.1',
    plan: {
      'install': planOf(
        'install',
        '3.13.1',
        bytes: 40 * 1024 * 1024,
        downloads: [
          {'url': 'https://github.com/astral-sh/python-build-standalone/releases/download/20251101/cpython-3.13.1.tar.gz', 'bytes': 40 * 1024 * 1024, 'check': {'kind': 'unverified', 'why': 'the release lists no checksum for this file'}},
        ],
      ),
    },
  ),
  FakeStore(
    'foundry_store',
    title: 'Foundry Local',
    runtime: true,
    summary: 'Runs language models on this computer, for the agents that use a Foundry Local account.',
    icon: 'crucible',
    homepage: 'https://github.com/microsoft/Foundry-Local',
    endpoints: ['https://api.github.com/repos/microsoft/Foundry-Local/releases'],
    copies: [copyOf('0.8.2', managed: false, path: r'C:\Program Files\Foundry Local\foundry.exe')],
    latest: '0.8.2',
  ),
]);

/// What canoa.cfg starts by name: an agent of the host and one of the person's, a Node MCP server, one by a full path (a path names
/// one copy, which no runtime is found by), one over HTTP and a Python one.
final _config = CanoaConfig(
  host: {
    'agents': {
      'review': {'program': 'npx', 'args': ['-y', 'some-agent'], 'listen': '127.0.0.1:7001'},
    },
  },
  agents: const [AgentCommand(label: 'Mine', program: 'python3', args: ['-m', 'agent'])],
  profile: const Profile(mcp: {
    'files': McpStdio(command: 'node.exe', args: ['server.js']),
    'pinned': McpStdio(command: r'C:\tools\node.exe'),
    'remote': McpHttp(url: 'https://mcp.example.test/'),
    'notes': McpStdio(command: 'Python'),
  }),
);

Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 20; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}

Future<ProgramStores> _load(WidgetTester tester, FakeStoreHost host) async {
  final programs = host.programs(kind: runtimesTypeId);
  await tester.runAsync(() => programs.refresh(latest: true));
  return programs;
}

Future<void> _page(WidgetTester tester, ProgramStores programs, String provider, {Size size = const Size(1000, 1600), CanoaConfig? config}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(debugShowCheckedModeBanner: false, home: RuntimePage(programs: programs, provider: provider, config: config ?? _config)));
  await _settle(tester);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/runtime_page_test.dart')));

  group('what uses a runtime', () {
    test('a command found by name on the PATH, whatever its case or extension, and no path', () {
      expect(bareCommand('node.exe'), 'node');
      expect(bareCommand('NPX.CMD'), 'npx');
      expect(bareCommand(r'C:\tools\node.exe'), isNull);
      expect(bareCommand('/usr/bin/node'), isNull);
      expect(bareCommand(' '), isNull);
    });

    test('agents of the host and of the person and the MCP servers that start a command it provides', () async {
      final host = _host();
      final programs = host.programs(kind: runtimesTypeId);
      await programs.refresh();
      final node = RuntimeItem.find(programs, 'node_store')!;
      expect([for (final user in node.users(_config)) '${user.kind} ${user.name} ${user.command}'], [
        'Agent review npx',
        'MCP server files node.exe',
      ]);
      final python = RuntimeItem.find(programs, 'python_store')!;
      expect([for (final user in python.users(_config)) '${user.kind} ${user.name} ${user.command}'], [
        'Agent Mine python3',
        'MCP server notes Python',
      ]);
      expect(RuntimeItem.find(programs, 'foundry_store')!.users(_config), isEmpty, reason: 'it provides no command');
      expect(RuntimeItem.find(programs, 'nothing_store'), isNull);
    });
  });

  testWidgets('each runtime\'s tile carries the glyph its store names, and opens its page', (tester) async {
    final host = _host();
    final extensions = StoreExtensions(control: () async => host.client(), open: () async => host.client());
    addTearDown(extensions.dispose);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      home: Scaffold(body: ExtensionBody(type: extensions.types.first, view: StoreView(runtimesTypeId), config: _config)),
    ));
    await tester.runAsync(extensions.ensure);
    await _settle(tester);
    for (final glyph in ['nodes', 'indent', 'crucible']) {
      expect(find.byKey(Key('runtime-glyph-$glyph')), findsOneWidget, reason: glyph);
    }
    await tester.tap(find.byKey(const Key('store-entry-This host-Python')));
    await _settle(tester);
    expect(find.byKey(const Key('runtime-details-python_store')), findsOneWidget);
    expect(find.byKey(const Key('runtime-glyph-indent')), findsOneWidget);
    await tester.pageBack();
    await _settle(tester);
    expect(find.byKey(const Key('runtime-details-python_store')), findsNothing);
  });

  testWidgets('an installed runtime says its versions, where it is, where it downloads from and what uses it', (tester) async {
    final host = _host();
    final programs = await _load(tester, host);
    await _page(tester, programs, 'node_store');
    expect(find.byKey(const Key('runtime-glyph-nodes')), findsOneWidget);
    expect(find.text('Node.js'), findsWidgets);
    expect(find.byKey(const Key('details-state')), findsOneWidget);
    expect(find.text('Update available'), findsOneWidget);
    expect(find.textContaining('A JavaScript runtime'), findsOneWidget);
    expect(find.textContaining('Publisher: A Publisher'), findsOneWidget);
    expect(find.byKey(const Key('details-link-homepage')), findsOneWidget);
    expect(find.textContaining('Installed version: 24.1.0'), findsOneWidget);
    expect(find.textContaining('Newest version: 24.2.0 (an update)'), findsOneWidget);
    expect(find.textContaining('Installed at: /programs/node/24.1.0/bin/node'), findsOneWidget);
    expect(find.textContaining('Downloads from: https://nodejs.org/dist/'), findsOneWidget);
    expect(find.textContaining('Commands it provides: node, npm, npx, corepack'), findsOneWidget);
    expect(find.byKey(const Key('runtime-user-Agent-review')), findsOneWidget);
    expect(find.byKey(const Key('runtime-user-MCP server-files')), findsOneWidget);
    expect(find.byKey(const Key('runtime-user-MCP server-pinned')), findsNothing, reason: 'a full path names a copy, not what the PATH finds');
    expect(find.byKey(const Key('runtime-user-MCP server-remote')), findsNothing);
    expect(find.byKey(const Key('details-contents-unread')), findsNothing, reason: 'a runtime provides no skills or servers');
    expect(find.byKey(const Key('runtime-install')), findsNothing);
    expect(find.byKey(const Key('runtime-update')), findsOneWidget);
    expect(find.byKey(const Key('runtime-rollback')), findsOneWidget);
    expect(find.byKey(const Key('runtime-remove')), findsOneWidget);
    expect(host.calls.where((call) => call.startsWith('store/plan')), isEmpty, reason: 'nothing is asked of the publisher by opening it');
    expect(tester.takeException(), isNull);
  });

  testWidgets('a runtime that is not installed, and one found elsewhere, offer a copy of the host\'s own', (tester) async {
    final host = _host();
    final programs = await _load(tester, host);
    await _page(tester, programs, 'python_store');
    expect(find.byKey(const Key('details-state')), findsNothing);
    expect(find.textContaining('Installed version: none'), findsOneWidget);
    expect(find.textContaining('Newest version: 3.13.1'), findsOneWidget);
    expect(find.text('Install'), findsOneWidget);
    expect(find.byKey(const Key('runtime-remove')), findsNothing);
    expect(find.byKey(const Key('runtime-update')), findsNothing);

    await _page(tester, programs, 'foundry_store');
    expect(find.byKey(const Key('runtime-glyph-crucible')), findsOneWidget);
    expect(find.textContaining('found on this computer, not installed by Canoa'), findsOneWidget);
    expect(find.textContaining(r'Installed at: C:\Program Files\Foundry Local\foundry.exe'), findsOneWidget);
    expect(find.text('Install a copy for Canoa'), findsOneWidget);
    expect(find.byKey(const Key('runtime-remove')), findsNothing, reason: 'a program found on this computer is not Canoa\'s to remove');
    expect(find.byKey(const Key('runtime-uses-none')), findsOneWidget);
    expect(find.textContaining('the host starts it itself'), findsOneWidget);
  });

  testWidgets('the download and its checksum are asked of the publisher only when the person asks', (tester) async {
    final host = _host();
    final programs = await _load(tester, host);
    await _page(tester, programs, 'node_store');
    expect(find.byKey(const Key('runtime-file-0')), findsNothing);
    await tester.ensureVisible(find.byKey(const Key('runtime-lookup')));
    await tester.tap(find.byKey(const Key('runtime-lookup')));
    await _settle(tester);
    expect(host.params['store/plan']!.single, containsPair('action', 'update'));
    expect(find.textContaining('File: https://nodejs.org/dist/v24.2.0/node-v24.2.0-win-arm64.zip'), findsOneWidget);
    expect(find.textContaining('Checked against: sha256 $_digest'), findsOneWidget);
    expect(find.textContaining('Size: 31 MB'), findsOneWidget);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'looking it up installs nothing');

    await _page(tester, programs, 'python_store');
    await tester.ensureVisible(find.byKey(const Key('runtime-lookup')));
    await tester.tap(find.byKey(const Key('runtime-lookup')));
    await _settle(tester);
    expect(find.byKey(const Key('runtime-unverified-0')), findsOneWidget, reason: 'a file the publisher gives nothing for is said so');
    expect(find.textContaining('the release lists no checksum for this file'), findsOneWidget);
  });

  testWidgets('what the host refuses a lookup with is shown in its words', (tester) async {
    final host = _host()..failures['store/plan'] = ControlFailed(-32000, 'the publisher\'s release list could not be read');
    final programs = await _load(tester, host);
    await _page(tester, programs, 'node_store');
    await tester.ensureVisible(find.byKey(const Key('runtime-lookup')));
    await tester.tap(find.byKey(const Key('runtime-lookup')));
    await _settle(tester);
    expect(find.byKey(const Key('runtime-lookup-problem')), findsOneWidget);
    expect(find.textContaining('release list could not be read'), findsOneWidget);
    expect(find.byKey(const Key('runtime-lookup')), findsOneWidget, reason: 'it can be asked again');
  });

  testWidgets('Update shows the host\'s plan and installs only once it is confirmed; the page then says what it is now', (tester) async {
    final host = _host();
    host.afterApply = (store, plan) => store.copies = [copyOf('24.2.0', path: '/programs/node/24.2.0/bin/node')];
    final programs = await _load(tester, host);
    await _page(tester, programs, 'node_store');
    await tester.ensureVisible(find.byKey(const Key('runtime-update')));
    await tester.tap(find.byKey(const Key('runtime-update')));
    await _settle(tester);
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'nothing runs before the person confirms');
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(find.byKey(const Key('program-done')), findsOneWidget);
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    expect(host.calls.where((call) => call == 'store/apply node_store'), hasLength(1));
    expect(find.textContaining('Installed version: 24.2.0'), findsOneWidget);
    expect(find.byKey(const Key('runtime-update')), findsNothing, reason: 'it is at the newest version');
    expect(find.text('Installed'), findsOneWidget);
  });

  testWidgets('Install of a runtime that is not here, and Remove of the host\'s copy, each show the plan first', (tester) async {
    final host = _host();
    host.afterApply = (store, plan) {
      store.copies = plan['action'] == 'uninstall' ? [] : [copyOf('3.13.1')];
    };
    final programs = await _load(tester, host);
    await _page(tester, programs, 'python_store');
    await tester.tap(find.byKey(const Key('runtime-install')));
    await _settle(tester);
    expect(host.params['store/plan']!.last, containsPair('action', 'install'));
    expect(find.byKey(const Key('plan-unverified-0')), findsOneWidget);
    await tester.tap(find.byKey(const Key('plan-cancel')));
    await _settle(tester);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'a refused plan installs nothing');
    await tester.tap(find.byKey(const Key('runtime-install')));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    expect(find.textContaining('Installed version: 3.13.1'), findsOneWidget);

    await tester.ensureVisible(find.byKey(const Key('runtime-remove')));
    await tester.tap(find.byKey(const Key('runtime-remove')));
    await _settle(tester);
    expect(host.params['store/plan']!.last, containsPair('action', 'uninstall'));
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    expect(find.textContaining('Installed version: none'), findsOneWidget);
    expect(host.calls.where((call) => call == 'store/apply python_store'), hasLength(2));
  });

  testWidgets('a runtime the host stops listing leaves the page saying so', (tester) async {
    final host = _host();
    final programs = await _load(tester, host);
    await _page(tester, programs, 'nothing_store');
    expect(find.byKey(const Key('runtime-gone')), findsOneWidget);
  });

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    testWidgets('the Runtimes list draws at $label width', (tester) async {
      final host = _host();
      final extensions = StoreExtensions(control: () async => host.client(), open: () async => host.client());
      addTearDown(extensions.dispose);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        home: Scaffold(body: ExtensionBody(type: extensions.types.first, view: StoreView(runtimesTypeId), config: _config)),
      ));
      await tester.runAsync(extensions.ensure);
      await _settle(tester);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/runtimes_list_$label.png'));
    });

    testWidgets('a runtime page draws at $label width', (tester) async {
      final host = _host();
      final programs = await _load(tester, host);
      await _page(tester, programs, 'node_store', size: size);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/runtime_page_$label.png'));
    });

    testWidgets('a runtime download and its checksum, once asked for, draw at $label width', (tester) async {
      final host = _host();
      final programs = await _load(tester, host);
      await _page(tester, programs, 'node_store', size: size);
      await tester.scrollUntilVisible(find.byKey(const Key('runtime-lookup')), 300, scrollable: find.byType(Scrollable).first);
      await tester.tap(find.byKey(const Key('runtime-lookup')));
      await _settle(tester);
      await tester.drag(find.byType(ListView).first, const Offset(0, -4000));
      await _settle(tester);
      expect(find.byKey(const Key('runtime-checksum-0')), findsOneWidget);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/runtime_page_checksum_$label.png'));
    });
  }

  testWidgets('a runtime that is not installed draws its page at desktop width', (tester) async {
    final host = _host();
    final programs = await _load(tester, host);
    await _page(tester, programs, 'python_store', size: const Size(1280, 800));
    expect(tester.takeException(), isNull);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/runtime_page_not_installed_desktop.png'));
  });
}
