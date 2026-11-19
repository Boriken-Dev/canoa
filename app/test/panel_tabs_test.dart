// The kinds of tab the host's providers offer the panel (the `panel_tab` kind): what a new tab can be is what the host lists, a log shows
// the lines its provider keeps and follows them, and a kind an extension adds shows beside the built-in ones. The host is a stand-in.
@TestOn('vm')
library;

import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/panel_tabs.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/store/links.dart' as links;
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/sample_render.dart' show loadTerminalFont;
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _terminal = {'id': 'terminal', 'title': 'Terminal', 'icon': 'terminal', 'shows': 'terminal', 'shell': null, 'origin': 'built-in'};
const _logs = {'id': 'logs', 'title': 'Logs', 'icon': 'article', 'shows': 'log', 'shell': null, 'origin': 'built-in'};
const _buildLog = {'id': 'buildlog', 'title': 'Build log', 'icon': null, 'shows': 'log', 'shell': null, 'origin': 'builds'};
const _docs = {'id': 'docs', 'title': 'Docs', 'icon': 'book', 'shows': 'page', 'url': 'http://127.0.0.1:8123/docs', 'help': 'The extension\'s documentation.', 'origin': 'builds'};
const _devShell = {'id': 'devshell', 'title': 'Dev shell', 'icon': 'code', 'shows': 'terminal', 'shell': 'git-bash', 'origin': 'builds'};

/// The host's two methods over what each log holds.
final class _Host {
  List<Object?> listed = [_terminal, _logs, _buildLog, _devShell];
  final lines = <String, String>{'logs': 'INFO host started\nINFO listening\n', 'buildlog': 'step 1\nstep 2\n'};
  final asked = <String>[];
  String? refuse;

  Future<Object?> call(String method, Map<String, Object?> params) async {
    switch (method) {
      case 'panel_tabs/list':
        return {'tabs': listed};
      case 'panel_tabs/tail':
        final id = params['id']! as String;
        asked.add(id);
        if (refuse case final why?) throw ControlFailed(-32001, why);
        return {'id': id, 'text': lines[id] ?? ''};
    }
    throw ControlFailed(-32601, 'no $method');
  }
}

/// A host whose shells these tests only list: none is started.
final class _Shells implements TerminalHost {
  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) => throw UnimplementedError('no shell is started here');

  @override
  Future<List<HostShell>> shells() async => const [HostShell(id: 'git-bash', label: 'Git Bash', path: '/bin/bash', family: 'bash', origin: 'canoa')];
}

Finder _key(String name) => find.byKey(ValueKey(name));

class _Probe extends WorkSurface {
  const _Probe();
  @override
  String get id => 'probe';
  @override
  String get title => 'Details';
  @override
  IconData get icon => Icons.info_outline;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.bottom];
  @override
  bool available(ChatSession session) => true;
  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) => const Center(child: Text('probe'));
}

void main() {
  setUpAll(() async {
    goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/panel_tabs_test.dart'));
    await loadTerminalFont();
  });

  test('the kinds the host lists are read, one that shows nothing the app draws is left out, and an empty answer lists none', () async {
    final host = _Host();
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    host.listed = [..._host(), {'id': 'web', 'title': 'Page', 'shows': 'web'}, 'junk'];
    await kinds.load();
    expect([for (final each in kinds.kinds) each.id], ['terminal', 'logs', 'buildlog', 'devshell']);
    expect([for (final each in kinds.terminals) each.id], ['terminal', 'devshell']);
    expect([for (final each in kinds.logs) each.id], ['logs', 'buildlog']);
    expect(kinds.terminals.last.shell, 'git-bash');
    host.listed = const [];
    await kinds.load();
    expect(kinds.kinds, isEmpty);
    final silent = PanelTabKinds((_, _) async => throw const ControlFailed(-32601, 'no'));
    addTearDown(silent.dispose);
    await silent.load();
    expect(silent.kinds, isEmpty, reason: 'a host that cannot answer lists none');
  });

  testWidgets('the + menu offers what the host lists: Terminal with its shells, the logs and an extension\'s kinds; taking the extension away takes them away', (tester) async {
    final host = _Host();
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    await kinds.load();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: '/work');
    final registry = WorkSurfaces(
      [const _Probe(), TerminalSurface((_) => TerminalOnHost(_Shells()), kinds: () => kinds.terminals)],
      added: () => [for (final kind in kinds.logs) LogSurface(kind, kinds)],
      addedChanged: kinds,
    );
    addTearDown(registry.dispose);
    final open = registry.of(session.id)..openSurface(registry.registry.first, SurfaceSlot.bottom);
    final layout = ShellLayout();
    await tester.binding.setSurfaceSize(const Size(900, 320));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    Widget panel() => MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SurfaceEnvironment(
          store: store,
          onComment: (_) {},
          onOpenChild: () {},
          child: ListenableBuilder(
            listenable: registry,
            builder: (context, _) => WorkPanel(
              slot: SurfaceSlot.bottom,
              session: session,
              surfaces: open,
              offered: registry.availableFor(session, SurfaceSlot.bottom),
              layout: layout,
            ),
          ),
        ),
      ),
    );
    await tester.pumpWidget(panel());
    await tester.pump();

    await tester.tap(_key('panel-add'));
    await tester.pumpAndSettle();
    for (final key in ['open terminal', 'open terminal shell git-bash', 'open terminal kind devshell', 'open log:logs', 'open log:buildlog']) {
      expect(_key(key), findsOneWidget, reason: key);
    }
    expect(_key('open probe'), findsNothing, reason: 'Details is open already');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/panel_new_tab_menu.png'));
    await tester.tap(_key('open log:buildlog'));
    await tester.pumpAndSettle();
    expect(_key('tab log:buildlog'), findsOneWidget);
    expect(find.text('step 1\nstep 2\n'), findsOneWidget);

    // The extension is taken away: its kinds leave the menu, and a log already open stays until it is closed.
    host.listed = [_terminal, _logs];
    await kinds.load();
    await tester.pumpAndSettle();
    await tester.tap(_key('panel-add'));
    await tester.pumpAndSettle();
    expect(_key('open log:logs'), findsOneWidget);
    expect(_key('open log:buildlog'), findsNothing);
    expect(_key('open terminal kind devshell'), findsNothing);
    expect(_key('open terminal'), findsOneWidget);
    await tester.tapAt(const Offset(2, 2));
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox());
  });

  test('a page is read with the address its provider gives, and one without a plain http or https address is left out', () async {
    final host = _Host();
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    host.listed = [
      _terminal,
      _docs,
      {..._docs, 'id': 'nourl', 'url': null},
      {..._docs, 'id': 'file', 'url': 'file:///etc/passwd'},
      {..._docs, 'id': 'script', 'url': 'javascript:alert(1)'},
    ];
    await kinds.load();
    expect([for (final each in kinds.pages) each.id], ['docs']);
    expect(kinds.pages.single.url, Uri.parse('http://127.0.0.1:8123/docs'));
    expect([for (final each in kinds.terminals) each.id], ['terminal'], reason: 'a page is no terminal or log');
    expect(kinds.logs, isEmpty);
  });

  testWidgets('a page an extension serves is offered by the + menu and opens in the browser; a browser that takes none says so', (tester) async {
    final host = _Host()..listed = [_terminal, _logs, _docs];
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    await kinds.load();
    final opened = <Uri>[];
    var answer = true;
    final before = links.openLink;
    links.openLink = (uri) async {
      opened.add(uri);
      return answer;
    };
    addTearDown(() => links.openLink = before);
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t');
    final registry = WorkSurfaces(
      [const _Probe()],
      added: () => [for (final kind in kinds.pages) PageSurface(kind)],
      addedChanged: kinds,
    );
    addTearDown(registry.dispose);
    final open = registry.of(session.id)..openSurface(registry.registry.first, SurfaceSlot.bottom);
    await tester.binding.setSurfaceSize(const Size(900, 360));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SurfaceEnvironment(
          store: store,
          onComment: (_) {},
          onOpenChild: () {},
          child: ListenableBuilder(
            listenable: registry,
            builder: (context, _) => WorkPanel(slot: SurfaceSlot.bottom, session: session, surfaces: open, offered: registry.availableFor(session, SurfaceSlot.bottom), layout: ShellLayout()),
          ),
        ),
      ),
    ));
    await tester.pump();
    await tester.tap(_key('panel-add'));
    await tester.pumpAndSettle();
    expect(_key('open page:docs'), findsOneWidget);
    await tester.tap(_key('open page:docs'));
    await tester.pumpAndSettle();
    expect(_key('tab page:docs'), findsOneWidget);
    expect(find.text('http://127.0.0.1:8123/docs'), findsOneWidget);
    expect(find.text('The extension\'s documentation.'), findsOneWidget);
    expect(opened, isEmpty, reason: 'nothing opens until the person asks');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/panel_page_tab.png'));

    await tester.tap(_key('page open'));
    await tester.pumpAndSettle();
    expect(opened, [Uri.parse('http://127.0.0.1:8123/docs')]);
    expect(_key('page problem'), findsNothing);

    answer = false;
    await tester.tap(_key('page open'));
    await tester.pumpAndSettle();
    expect(_key('page problem'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a log shows the lines its provider keeps and follows them, and says so when there are none or the host refuses', (tester) async {
    final host = _Host();
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    await kinds.load();
    final surface = LogSurface(kinds.logs.first, kinds);
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t');
    final controller = SurfaceController(surface.id);
    await tester.binding.setSurfaceSize(const Size(700, 240));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: Builder(builder: (context) => surface.build(context, session, controller))),
    ));
    await tester.pump();
    expect(find.text('INFO host started\nINFO listening\n'), findsOneWidget);
    expect(surface.id, 'log:logs');

    host.lines['logs'] = 'INFO host started\nINFO listening\nWARN a request was slow\n';
    await tester.pump(LogSurface.refresh);
    await tester.pump();
    expect(find.textContaining('WARN a request was slow'), findsOneWidget, reason: 'it follows the log');

    host.lines['logs'] = '';
    await tester.pump(LogSurface.refresh);
    await tester.pump();
    expect(_key('log empty'), findsOneWidget);

    host.refuse = 'This key may not read the host\'s log.';
    await tester.pump(LogSurface.refresh);
    await tester.pump();
    expect(find.textContaining('may not read the host'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('the panel as it is drawn: a shell and the host\'s log as tabs of one bar', (tester) async {
    final host = _Host();
    final kinds = PanelTabKinds(host.call);
    addTearDown(kinds.dispose);
    await kinds.load();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t');
    final registry = WorkSurfaces(
      [const _Probe()],
      added: () => [for (final kind in kinds.logs) LogSurface(kind, kinds)],
      addedChanged: kinds,
    );
    addTearDown(registry.dispose);
    final open = registry.of(session.id)
      ..openSurface(registry.registry.last, SurfaceSlot.bottom)
      ..openSurface(registry.registry[1], SurfaceSlot.bottom);
    await tester.binding.setSurfaceSize(const Size(900, 240));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SurfaceEnvironment(
          store: store,
          onComment: (_) {},
          onOpenChild: () {},
          child: WorkPanel(slot: SurfaceSlot.bottom, session: session, surfaces: open, offered: registry.availableFor(session, SurfaceSlot.bottom), layout: ShellLayout()),
        ),
      ),
    ));
    await tester.pump();
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/panel_log_tab.png'));
    await tester.pumpWidget(const SizedBox());
  });
}

List<Object?> _host() => [_terminal, _logs, _buildLog, _devShell];
