// The Terminal surface over a stand-in host: a shell opened in the session's workspace, what is typed reaching it, hiding and moving
// the surface leaving the shell and its screen as they were, tabs and a split, what the person attaches to their message, and what
// the surface says where there is no shell to open.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/sample_render.dart' show loadTerminalFont;
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A shell the test drives: it prints what [print] gives it and records what was typed.
final class _Shell implements RemoteTerminal {
  _Shell(this.folder);

  @override
  final String folder;
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  final writes = <String>[];
  final sizes = <(int, int)>[];
  bool closed = false;

  void print(String text) => _out.add(utf8.encode(text));
  void end(int code) => _exit.complete(code);

  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => _exit.future;
  @override
  Future<void> write(String text) async => writes.add(text);
  @override
  Future<void> resize(int cols, int rows) async => sizes.add((cols, rows));
  @override
  Future<void> close() async {
    closed = true;
    if (!_exit.isCompleted) _exit.complete(null);
    await _out.close();
  }
}

final class _Host implements TerminalHost {
  final opened = <_Shell>[];
  final cwds = <String?>[];

  /// A refusal to give instead of a shell.
  String? refuse;

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    cwds.add(cwd);
    if (refuse case final why?) throw ControlFailed(-32002, why);
    final shell = _Shell(cwd ?? '/home');
    opened.add(shell);
    return shell;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

Finder _key(String name) => find.byKey(ValueKey(name));

ChatSession _session(SessionStore store, String? cwd) =>
    store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: cwd);

Future<void> _show(WidgetTester tester, Widget child, {Size size = const Size(900, 420)}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(debugShowCheckedModeBanner: false, theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: child)));
  await tester.pump(const Duration(milliseconds: 50));
}

/// Lets the stand-in's futures and streams run, which the test clock does not.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

void main() {
  setUpAll(() async {
    goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/terminal_surface_test.dart'));
    await loadTerminalFont();
  });

  testWidgets('a shell opens in the session\'s workspace, shows what it prints and gets what is typed', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));

    expect(surface.available(session), isTrue, reason: 'it opens before any agent has started');
    expect(surface.placements, [SurfaceSlot.bottom, SurfaceSlot.right]);
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
    await _settle(tester);

    expect(host.cwds, ['/work/app']);
    final pane = surface.workspaceFor(controller, session)!.focused!;
    expect(pane.state, PaneState.running);
    host.opened.single.print('PS /work/app> ');
    await _settle(tester);
    expect(pane.terminal.buffer.getText(), contains('PS /work/app> '));

    pane.terminal.textInput('ls\r');
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ls\r');
  });

  testWidgets('the terminal draws in a bundled monospace font, so a cell is as wide as any letter in it', (tester) async {
    final style = terminalTextStyle.toTextStyle();
    expect(style.fontFamily, 'Source Code Pro', reason: 'a page or a minimal system has no monospace face of its own');
    double width(String text) => (TextPainter(text: TextSpan(text: text, style: style), textDirection: TextDirection.ltr)..layout()).width;
    expect(width('i'), width('m'));
    expect(width('illimit'), width('WWWWWWW'));
  });

  testWidgets('text sits in even cells: no gap after a narrow letter', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)), size: const Size(520, 160));
    await _settle(tester);
    host.opened.single.print('PS /work/app> git status\r\nillimit WWW mmm lllll iiiii\r\n0123456789 {}[]()\r\n');
    await _settle(tester);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/terminal_even_cells.png'));
  });

  testWidgets('a second shell from the panel opens in the session\'s workspace too', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
    await _settle(tester);
    await tester.tap(_key('terminal new'));
    await _settle(tester);
    final workspace = surface.workspaceFor(controller, session)!;
    workspace.split(workspace.active!);
    await _settle(tester);
    expect(host.cwds, ['/work/app', '/work/app', '/work/app']);
  });

  testWidgets('hiding the panel, and moving the surface, keep the shell and what it printed', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final mine = SessionSurfaces();
    final open = mine.openSurface(surface);
    addTearDown(() => tester.runAsync(open.controller.release));
    Widget panel() => Builder(builder: (context) => KeyedSubtree(key: open.key, child: surface.build(context, session, open.controller)));

    await _show(tester, panel());
    await _settle(tester);
    host.opened.single.print('first line\r\nsecond line\r\n');
    await _settle(tester);

    // Hidden: the widgets go, the shell and its screen stay.
    await tester.pumpWidget(const SizedBox());
    host.opened.single.print('printed while hidden\r\n');
    await _settle(tester);
    await _show(tester, panel());
    await _settle(tester);
    final text = surface.workspaceFor(open.controller, session)!.focused!.terminal.buffer.getText();
    expect(text, allOf(contains('first line'), contains('printed while hidden')));
    expect(host.opened, hasLength(1), reason: 'showing it again starts no second shell');
    expect(host.opened.single.closed, isFalse);

    // Moved to the right panel and back: the same controller, the same shell.
    expect(mine.move(TerminalSurface.surfaceId, SurfaceSlot.right), isTrue);
    expect(mine.move(TerminalSurface.surfaceId, SurfaceSlot.bottom), isTrue);
    expect(identical(mine.find(TerminalSurface.surfaceId)!.controller, open.controller), isTrue);
    await _show(tester, panel());
    await _settle(tester);
    expect(host.opened, hasLength(1));
    expect(host.opened.single.closed, isFalse);

    // Closing the surface ends the shell.
    mine.close(TerminalSurface.surfaceId);
    await _settle(tester);
    expect(host.opened.single.closed, isTrue);
  });

  testWidgets('tabs: a new shell, a split beside one, a rename, and closing ends only that shell', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
    await _settle(tester);
    final workspace = surface.workspaceFor(controller, session)!;

    await tester.tap(_key('terminal new'));
    await _settle(tester);
    expect(workspace.tabs.map((tab) => tab.name), ['Shell 1', 'Shell 2']);
    expect(host.opened, hasLength(2));

    await tester.tap(_key('terminal split'));
    await _settle(tester);
    expect(workspace.active!.panes, hasLength(2));
    expect(host.opened, hasLength(3));
    expect(tester.widget<IconButton>(_key('terminal split')).onPressed, isNull, reason: 'a tab splits in two, no more');

    // The second tab is the one shown, so the menu renames it.
    await tester.tap(_key('terminal menu'));
    await tester.pumpAndSettle();
    await tester.tap(_key('terminal rename'));
    await tester.pumpAndSettle();
    await tester.enterText(_key('terminal rename field'), 'Build');
    await tester.tap(_key('terminal rename ok'));
    await tester.pumpAndSettle();
    expect(workspace.tabs.map((tab) => tab.name), ['Shell 1', 'Build']);

    await tester.tap(_key('terminal close tab 2'));
    await _settle(tester);
    expect(workspace.tabs.map((tab) => tab.name), ['Shell 1']);
    expect(host.opened.map((shell) => shell.closed), [false, true, true], reason: 'both shells of the closed tab ended, the first stayed');
  });

  testWidgets('the selection and the last output go to the message as text files; nothing else does', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    final attached = <(String, String)>[];
    await _show(
      tester,
      SurfaceEnvironment(
        store: store,
        onComment: (_) {},
        onOpenChild: () {},
        onAttachText: (name, text) => attached.add((name, text)),
        child: Builder(builder: (context) => surface.build(context, session, controller)),
      ),
    );
    await _settle(tester);
    host.opened.single.print('build ok\r\n3 tests passed\r\n');
    await _settle(tester);

    expect(tester.widget<IconButton>(_key('terminal attach selection')).onPressed, isNull, reason: 'nothing is selected');
    await tester.tap(_key('terminal attach output'));
    await tester.pump();
    expect(attached, [('terminal output.txt', 'build ok\n3 tests passed')]);
    expect(host.opened.single.writes, isEmpty, reason: 'attaching types nothing into the shell');

    final pane = surface.workspaceFor(controller, session)!.focused!;
    final buffer = pane.terminal.buffer;
    pane.view.setSelection(buffer.createAnchor(0, 0), buffer.createAnchor(8, 0));
    await tester.pump();
    expect(pane.selectedText, 'build ok');
    await tester.tap(_key('terminal attach selection'));
    await tester.pump();
    expect(attached.last, ('terminal selection.txt', 'build ok'));
  });

  testWidgets('a command is typed without Enter, even while the shell is still starting', (tester) async {
    final gate = Completer<void>();
    final host = _GatedHost(gate.future);
    final pane = TerminalPane(host: host, cwd: '/work');
    addTearDown(() => tester.runAsync(pane.close));
    final starting = pane.start();
    pane.type('cargo test');
    expect(pane.state, PaneState.starting);
    gate.complete();
    await tester.runAsync(() => starting);
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    expect(host.remote.writes.join(), 'cargo test', reason: 'no carriage return: the person decides');
  });

  testWidgets('a host that refuses says so in its words, and the shell is started again on request', (tester) async {
    final host = _Host()..refuse = 'at most 8 terminals are open on this link';
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
    await _settle(tester);
    expect(find.text('at most 8 terminals are open on this link'), findsOneWidget);

    host.refuse = null;
    await tester.tap(_key('terminal restart'));
    await _settle(tester);
    expect(_key('terminal note'), findsNothing);
    expect(host.opened, hasLength(1));

    // A shell that ends says its status and can be started again.
    host.opened.single.end(3);
    await _settle(tester);
    expect(find.text('The shell ended (exit status 3).'), findsOneWidget);
    await tester.tap(_key('terminal restart'));
    await _settle(tester);
    expect(host.opened, hasLength(2));
    expect(_key('terminal note'), findsNothing);
  });

  testWidgets('a direct agent endpoint says it offers no shell and opens none; a host without the grant lists no surface', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final refusing = TerminalSurface((_) => const TerminalRefused('This is a direct agent endpoint: it offers a session and no shell.'));
    expect(refusing.available(session), isTrue);
    final controller = SurfaceController('terminal');
    await _show(tester, Builder(builder: (context) => refusing.build(context, session, controller)));
    expect(find.text('This is a direct agent endpoint: it offers a session and no shell.'), findsOneWidget);
    expect(refusing.workspaceFor(controller, session), isNull);
    expect(const TerminalSurface(_none).available(session), isFalse);
  });

  test('the bottom-panel key opens the Terminal there when the host offers one, and hides the panel the next time', () {
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final layout = ShellLayout();
    final surfaces = WorkSurfaces([TerminalSurface((_) => TerminalOnHost(_Host()))]);
    addTearDown(surfaces.dispose);
    expect(layout.bottomOpen, isFalse);

    toggleBottomPanel(surfaces: surfaces, layout: layout, session: session);
    expect(layout.bottomOpen, isTrue);
    expect(surfaces.of(session.id).activeIn(SurfaceSlot.bottom)?.surface.id, TerminalSurface.surfaceId);

    toggleBottomPanel(surfaces: surfaces, layout: layout, session: session);
    expect(layout.bottomOpen, isFalse);
    expect(surfaces.of(session.id).find(TerminalSurface.surfaceId), isNotNull, reason: 'hiding closes nothing');

    // Without a host's terminal the surface is not listed, and the key has nothing to show.
    final none = WorkSurfaces([const TerminalSurface(_none)]);
    addTearDown(none.dispose);
    final other = ShellLayout();
    toggleBottomPanel(surfaces: none, layout: other, session: session);
    expect(other.bottomOpen, isFalse);
  });

  testWidgets('the panel as it is drawn: two tabs, the second split, a prompt in each', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = _session(store, '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(
      tester,
      SurfaceEnvironment(
        store: store,
        onComment: (_) {},
        onOpenChild: () {},
        onAttachText: (_, _) {},
        child: Builder(builder: (context) => surface.build(context, session, controller)),
      ),
      size: const Size(900, 300),
    );
    await _settle(tester);
    final workspace = surface.workspaceFor(controller, session)!;
    workspace.newTab();
    await _settle(tester);
    workspace.split(workspace.active!);
    await _settle(tester);
    host.opened[0].print('PS /work/app> cargo test\r\n');
    host.opened[1].print('PS /work/app> ');
    host.opened[2].print('PS /work/app> git status\r\n');
    await _settle(tester);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/terminal_panel.png'));
  });
}

TerminalAccess? _none(ChatSession _) => null;

final class _GatedHost implements TerminalHost {
  _GatedHost(this.gate);

  final Future<void> gate;
  final remote = _Shell('/work');

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    await gate;
    return remote;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}
