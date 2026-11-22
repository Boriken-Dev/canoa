// The running counts the Terminal gives its tabs: how many shells of a tab are running, shown beside its title, and when the workspace
// tells its listeners (a count moving, not a shell's title changing).
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Shell implements RemoteTerminal {
  @override
  String get folder => '/work';
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();

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
    if (!_exit.isCompleted) _exit.complete(null);
    await _out.close();
  }
}

final class _Host implements TerminalHost {
  final opened = <_Shell>[];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final made = _Shell();
    opened.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

void main() {
  testWidgets('a workspace counts its running shells and tells its listeners when the count moves, not when a title does', (tester) async {
    final host = _Host();
    final workspace = TerminalWorkspace(host: host, cwd: '/work');
    addTearDown(() => tester.runAsync(workspace.release));
    var told = 0;
    workspace.addListener(() => told++);

    workspace.newTab();
    await _settle(tester);
    expect(workspace.runningShells, 1);
    expect(workspace.active!.runningCount, 1);

    told = 0;
    host.opened.single.print('\x1b]0;a new title\x07');
    await _settle(tester);
    expect(workspace.focused!.shellTitle, 'a new title');
    expect(told, 0, reason: 'a title is not a change of what is running');

    workspace.newTab();
    await _settle(tester);
    expect(workspace.runningShells, 2);
    expect(told, greaterThan(0));

    told = 0;
    host.opened.first.end(0);
    await _settle(tester);
    expect(workspace.runningShells, 1, reason: 'an ended shell is not running');
    expect(told, greaterThan(0), reason: 'a shell ending moves the count');
  });

  testWidgets('a tab beside its title and the surface\'s single tab show the number of shells running', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: '/work');
    final terminal = TerminalSurface((_) => TerminalOnHost(host));
    final registry = WorkSurfaces([terminal]);
    addTearDown(registry.dispose);
    final open = registry.of(session.id)..openSurface(terminal, SurfaceSlot.bottom);
    addTearDown(() => tester.runAsync(() => Future.wait([for (final each in open.open) each.controller.release()])));
    await tester.binding.setSurfaceSize(const Size(900, 420));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SurfaceEnvironment(
          store: store,
          onComment: (_) {},
          onOpenChild: () {},
          child: WorkPanel(
            slot: SurfaceSlot.bottom,
            session: session,
            surfaces: open,
            offered: registry.availableFor(session, SurfaceSlot.bottom),
            layout: ShellLayout(),
          ),
        ),
      ),
    ));
    await tester.pump(const Duration(milliseconds: 50));
    await _settle(tester);

    final tab = find.byKey(const ValueKey('tab terminal:1'));
    expect(tab, findsOneWidget);
    expect(find.descendant(of: tab, matching: find.text('1')), findsOneWidget);
    final controller = open.open.single.controller;
    expect(terminal.badge(session, controller), '1');

    host.opened.single.end(0);
    await _settle(tester);
    expect(find.descendant(of: tab, matching: find.text('1')), findsNothing, reason: 'the shell ended, so nothing is running in the tab');
    expect(terminal.badge(session, controller), isNull);
  });
}
