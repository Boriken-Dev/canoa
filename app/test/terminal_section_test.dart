// The Terminal section over stand-in hosts: shells opened with no conversation on Local and on a second host listed apart, by host and
// folder, with their state; a host that gives no terminal refuses visibly and opens nothing; closing and ending a shell; and what a
// saved Canoa host offers of a shell, read from its own list and grants. With CANOA_SAMPLES_DIR set the scene is written with real fonts.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/connection.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_remote.dart';
import 'package:canoa/surfaces/terminal_section.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

final class _Shell implements RemoteTerminal {
  _Shell(this.folder);

  @override
  final String folder;
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  bool closed = false;

  void print(String text) => _out.add(utf8.encode(text));
  void end(int code) => _exit.complete(code);

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
    closed = true;
    if (!_exit.isCompleted) _exit.complete(null);
    await _out.close();
  }
}

final class _Host implements TerminalHost {
  _Host(this.name);

  final String name;
  final opened = <_Shell>[];
  final cwds = <String?>[];

  /// The shell each open named, and the shells this host lists.
  final named = <String?>[];
  List<HostShell> listed = const [];

  @override
  Future<List<HostShell>> shells() async => listed;

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    cwds.add(cwd);
    named.add(shell);
    final made = _Shell(cwd ?? '/home/$name');
    opened.add(made);
    return made;
  }
}

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _show(WidgetTester tester, TerminalSectionController controller) async {
  await tester.binding.setSurfaceSize(const Size(1100, 520));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: Row(children: [
        SizedBox(width: 300, child: TerminalIndex(controller: controller)),
        const VerticalDivider(width: 1),
        Expanded(child: TerminalMain(controller: controller)),
      ]),
    ),
  ));
  await _settle(tester);
}

/// Local and a second host's home, each with what it offers.
({TerminalSectionController controller, _Host local, _Host second}) _world({TerminalAccess? secondAccess, ({String target, String folder})? selected}) {
  final local = _Host('local'), second = _Host('second');
  final targets = [
    TerminalTarget(id: 'local', name: 'Local', kind: 'This computer', access: () async => TerminalOnHost(local)),
    TerminalTarget(id: 'connection:Build box', name: 'Build box', kind: 'Canoa host', access: () async => secondAccess ?? TerminalOnHost(second)),
  ];
  final controller = TerminalSectionController(
    targets: () => targets,
    workspaces: (id) => id == 'local' ? const ['/work/app'] : const ['/srv/build'],
    selected: () => selected,
  );
  return (controller: controller, local: local, second: second);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/terminal_section_test.dart')));

  testWidgets('a shell on Local and one on a second host are listed apart, each under its host and folder, and run on its own host', (tester) async {
    final world = _world();
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    await tester.runAsync(() => world.controller.open('local', '/work/app'));
    await tester.runAsync(() => world.controller.open('connection:Build box', '/srv/build'));
    await _settle(tester);

    expect(world.local.cwds, ['/work/app']);
    expect(world.second.cwds, ['/srv/build']);
    expect(find.text('Local'), findsOneWidget);
    expect(find.text('Build box'), findsWidgets);
    expect(find.text('/work/app'), findsOneWidget);
    expect(find.text('/srv/build'), findsOneWidget);
    expect(world.controller.groups.map((group) => group.target.id), ['local', 'connection:Build box']);
    // One row each, both running, and the main view shows the one opened last.
    expect(find.text('running'), findsNWidgets(2));
    expect(tester.widget<Text>(_key('terminal-section title')).data, contains('Build box'));

    world.controller.show(world.controller.groups.first);
    await _settle(tester);
    expect(tester.widget<Text>(_key('terminal-section title')).data, contains('Local'));
    world.local.opened.single.print('PS /work/app> ');
    await _settle(tester);
    expect(world.controller.groups.first.workspace.focused!.terminal.buffer.getText(), contains('PS /work/app> '));
    expect(world.second.opened.single.closed, isFalse);
  });

  testWidgets('a second shell in the same folder joins its group; an ended shell reads exited; closing one ends it', (tester) async {
    final world = _world();
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    await tester.runAsync(() => world.controller.open('local', '/work/app'));
    await tester.runAsync(() => world.controller.open('local', '/work/app'));
    await _settle(tester);
    expect(world.controller.groups, hasLength(1));
    expect(world.controller.groups.single.workspace.tabs, hasLength(2));
    expect(find.descendant(of: find.byType(TerminalIndex), matching: find.text('Shell 1')), findsOneWidget);
    expect(find.descendant(of: find.byType(TerminalIndex), matching: find.text('Shell 2')), findsOneWidget);

    world.local.opened.first.end(0);
    await _settle(tester);
    expect(find.text('exited'), findsOneWidget);
    expect(find.text('running'), findsOneWidget);

    final group = world.controller.groups.single;
    await tester.tap(find.byKey(ValueKey('terminal section close ${group.key} 1')));
    await _settle(tester);
    expect(world.local.opened.first.closed, isTrue);
    expect(find.text('Shell 1'), findsNothing);

    await tester.tap(find.byKey(ValueKey('terminal section close ${group.key} 2')));
    await _settle(tester);
    expect(world.controller.groups, isEmpty, reason: 'a group with no shell is gone');
    expect(find.text('/work/app'), findsNothing);
    expect(_key('terminal-section start'), findsOneWidget);
  });

  testWidgets('a host that is removed from the list takes its shells with it and leaves the other host alone', (tester) async {
    final world = _world();
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);
    await tester.runAsync(() => world.controller.open('local', '/work/app'));
    await tester.runAsync(() => world.controller.open('connection:Build box', '/srv/build'));
    await tester.runAsync(() => world.controller.open('connection:Build box', null));
    await _settle(tester);
    expect(world.controller.groups, hasLength(3));

    await tester.runAsync(() => world.controller.closeHost('connection:Build box'));
    await _settle(tester);
    expect(world.second.opened.map((shell) => shell.closed), [true, true], reason: 'every shell on it ended');
    expect(world.local.opened.single.closed, isFalse);
    expect(world.controller.groups.map((group) => group.target.id), ['local']);
    expect(world.controller.accessOf('connection:Build box'), isNull, reason: 'what it said is forgotten');
    expect(find.text('/srv/build'), findsNothing);
    expect(find.text('/work/app'), findsOneWidget);
  });

  testWidgets('a host that gives no terminal says so in the index and in the main view, and opens nothing', (tester) async {
    final world = _world(secondAccess: const TerminalRefused('Build box has not given this app a terminal.'));
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    expect(_key('terminal host note connection:Build box'), findsOneWidget);
    expect(find.text('Build box has not given this app a terminal.'), findsOneWidget);
    expect(_key('terminal host note local'), findsNothing);

    await tester.runAsync(() => world.controller.open('connection:Build box', null));
    await _settle(tester);
    expect(world.controller.groups, isEmpty);
    expect(world.second.opened, isEmpty);
    expect(world.controller.refusal, 'Build box has not given this app a terminal.');
    expect(tester.widget<Text>(_key('terminal-section refusal')).data, 'Build box has not given this app a terminal.');

    // Local still works beside it, and showing a shell clears the refusal.
    await tester.runAsync(() => world.controller.open('local', null));
    await _settle(tester);
    expect(world.local.cwds, [null]);
    expect(_key('terminal-section refusal'), findsNothing);
  });

  testWidgets('the New shell dialog asks the host, then a folder, and offers the folders its conversations use', (tester) async {
    final world = _world();
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    await tester.tap(_key('terminal-section start'));
    await tester.pumpAndSettle();
    expect(find.text('/work/app'), findsOneWidget, reason: 'Local is first, so its folders are offered');
    await tester.tap(_key('terminal-section host'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Build box').last);
    await tester.pumpAndSettle();
    expect(find.text('/srv/build'), findsOneWidget);
    await tester.tap(find.text('/srv/build'));
    await tester.pump();
    await tester.tap(_key('terminal-section open'));
    await tester.pumpAndSettle();
    await _settle(tester);
    expect(world.second.cwds, ['/srv/build']);
    expect(world.local.opened, isEmpty);
  });

  testWidgets('the New shell dialog starts on the selected conversation\'s host and workspace, and drops that folder for another host', (tester) async {
    final world = _world(selected: (target: 'connection:Build box', folder: '/srv/build'));
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    await tester.tap(_key('terminal-section start'));
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(_key('terminal-section folder')).controller!.text, '/srv/build');
    await tester.tap(_key('terminal-section open'));
    await tester.pumpAndSettle();
    await _settle(tester);
    expect(world.second.cwds, ['/srv/build'], reason: 'with no choice made, the shell opens where the conversation is');
    expect(world.local.opened, isEmpty);

    await tester.tap(_key('terminal-section new'));
    await tester.pumpAndSettle();
    await tester.tap(_key('terminal-section host'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Local').last);
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(_key('terminal-section folder')).controller!.text, isEmpty, reason: 'that folder is the other host\'s');
  });

  testWidgets('the New shell dialog offers the shells the host found, and a shell is opened with the one chosen', (tester) async {
    final world = _world();
    world.local.listed = const [
      HostShell(id: 'git-bash', label: 'Git Bash', path: r'C:\Git\bin\bash.exe', family: 'bash', origin: 'git-for-windows'),
      HostShell(id: 'pwsh', label: 'PowerShell 7', path: r'C:\PS\pwsh.exe', family: 'powershell', origin: 'system'),
    ];
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);

    await tester.tap(_key('terminal-section start'));
    await tester.pumpAndSettle();
    expect(_key('terminal-section shell'), findsOneWidget);
    await tester.tap(_key('terminal-section shell'));
    await tester.pumpAndSettle();
    expect(find.text('The host\'s own choice'), findsWidgets);
    await tester.tap(find.text('Git Bash').last);
    await tester.pumpAndSettle();
    await tester.tap(_key('terminal-section open'));
    await tester.pumpAndSettle();
    await _settle(tester);
    expect(world.local.named, ['git-bash']);

    // The same folder and another shell is another group, the same shell again joins the first.
    await tester.runAsync(() => world.controller.open('local', null, shell: 'pwsh'));
    await tester.runAsync(() => world.controller.open('local', null, shell: 'git-bash'));
    await _settle(tester);
    expect(world.local.named, ['git-bash', 'pwsh', 'git-bash']);
    expect(world.controller.groups.map((group) => group.shell), ['git-bash', 'pwsh']);
    expect(world.controller.groups.first.workspace.shells, 2);
  });

  testWidgets('a host that lists no shell shows no Shell field, and the host\'s own choice opens with none named', (tester) async {
    final world = _world();
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);
    await tester.tap(_key('terminal-section start'));
    await tester.pumpAndSettle();
    expect(_key('terminal-section shell'), findsNothing);
    await tester.tap(_key('terminal-section open'));
    await tester.pumpAndSettle();
    await _settle(tester);
    expect(world.local.named, [null]);
  });

  testWidgets('the section as the person sees it', (tester) async {
    final world = _world(secondAccess: null);
    addTearDown(() => tester.runAsync(world.controller.release));
    await _show(tester, world.controller);
    await tester.runAsync(() => world.controller.open('local', '/work/app'));
    await tester.runAsync(() => world.controller.open('connection:Build box', '/srv/build'));
    await tester.runAsync(() => world.controller.open('connection:Build box', '/srv/build'));
    await _settle(tester);
    world.local.opened.single.print('PS /work/app> cargo test\r\n');
    world.second.opened[0].print('build@box:/srv/build\$ make\r\n');
    world.second.opened[1].print('build@box:/srv/build\$ ');
    world.second.opened[0].end(0);
    await _settle(tester);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/terminal_section.png'));
  });

  group('what a saved Canoa host offers', () {
    final peer = 'sha256:${'a' * 64}';
    Connection connection({String? id}) => Connection(name: 'Build box', kind: ConnectionKind.canoa, address: 'http://build.example:8080/', peer: id ?? peer, method: 'noise');

    http.Client list(Object entries, {String? id}) => MockClient((request) async {
      expect(request.url.path, '/canoa.json');
      return http.Response(jsonEncode({'peer': id ?? peer, 'method': 'noise', 'entries': entries}), 200);
    });

    const control = [{'name': 'control', 'kind': 'control', 'url': 'ws://build.example:8081/control'}];

    test('a direct agent endpoint offers no shell, in words', () async {
      final access = await terminalOnConnection(Connection(name: 'agent', kind: ConnectionKind.acp, address: 'ws://x/acp', peer: peer, method: 'noise'));
      expect(access, isA<TerminalRefused>());
      expect((access as TerminalRefused).reason, contains('no shell'));
    });

    test('a list that names no control channel, or another key, or no list at all, refuses with the reason', () async {
      final none = await terminalOnConnection(connection(), client: list(const []), control: (_) => throw StateError('must not dial'));
      expect((none as TerminalRefused).reason, contains('no control channel'));
      final changed = await terminalOnConnection(connection(), client: list(control, id: 'sha256:${'b' * 64}'), control: (_) => throw StateError('must not dial'));
      expect((changed as TerminalRefused).reason, contains('not the one saved'));
      final down = await terminalOnConnection(connection(), client: MockClient((_) async => http.Response('', 503)), control: (_) => throw StateError('must not dial'));
      expect((down as TerminalRefused).reason, contains('did not answer'));
    });

    test('a host whose grants lack terminal refuses; one that holds it opens shells through its own channel', () async {
      final links = <_GrantLink>[];
      HostControl dial(Set<String> grants) {
        final link = _GrantLink(grants);
        links.add(link);
        return HostControl.dialing(() async => link);
      }

      final refused = await terminalOnConnection(connection(), client: list(control), control: (_) => dial({'ledger.read'}));
      expect((refused as TerminalRefused).reason, contains('has not given this app a terminal'));
      expect(links.single.closed, isTrue, reason: 'a channel that gives nothing is not kept open');

      final granted = await terminalOnConnection(connection(), client: list(control), control: (_) => dial({'terminal'}));
      expect(granted, isA<TerminalOnHost>());
      final shell = await (granted as TerminalOnHost).host.open(cwd: '/srv/build', cols: 80, rows: 24);
      expect(shell.folder, '/srv/build');
      expect(links.last.asked, ['host/grants', 'terminal/open']);
      expect(links.first.asked, ['host/grants'], reason: 'nothing was asked of the host that refused beyond its grants');
    });

    test('a host the key cannot be proven to leaves its grants unknown and says to pair first', () async {
      final access = await terminalOnConnection(connection(), client: list(control), control: (_) => HostControl.dialing(() async => throw const ControlFailed(-1, 'refused')));
      expect((access as TerminalRefused).reason, contains('Pair'));
    });
  });
}

/// A host's control link that says its grants and opens a shell.
final class _GrantLink implements HostLink {
  _GrantLink(this.grants);

  final Set<String> grants;
  final _in = StreamController<String>();
  final asked = <String>[];
  bool closed = false;

  @override
  Stream<String> get messages => _in.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method'] as String;
    asked.add(method);
    final result = switch (method) {
      'host/grants' => {'grants': grants.toList()},
      'terminal/open' => {'id': 1, 'shell': 'bash', 'cwd': '/srv/build'},
      _ => <String, Object?>{},
    };
    if (request['id'] case final int id) _in.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
  }

  @override
  Future<void> close() async => closed = true;
}
