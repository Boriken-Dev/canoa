// Keys and text typed into the terminal as the app builds it, in the bottom panel and in the Terminal section, reaching the shell.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xterm2/xterm.dart';

final class _Shell implements RemoteTerminal {
  _Shell(this.folder);

  @override
  final String folder;
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  final writes = <String>[];

  void print(String text) => _out.add(utf8.encode(text));

  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => _exit.future;
  @override
  Future<void> write(String text) async => writes.add(text);
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
    final remote = _Shell(cwd ?? '/home');
    opened.add(remote);
    return remote;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

bool _asApp = false;

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _show(WidgetTester tester, Widget child) async {
  await tester.binding.setSurfaceSize(const Size(900, 420));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  // As main.dart draws the window: the whole shell is one selection area, and the composer (here a field) has the keyboard before
  // the panel is opened.
  Widget app(Widget panel) => MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: !_asApp
          ? panel
          : SelectionArea(
              child: Column(children: [
                const TextField(key: ValueKey('composer'), autofocus: true),
                Expanded(child: panel),
              ]),
            ),
    ),
  );
  if (_asApp) {
    await tester.pumpWidget(app(const SizedBox()));
    await tester.pump(const Duration(milliseconds: 50));
  }
  await tester.pumpWidget(app(child));
  await tester.pump(const Duration(milliseconds: 50));
}

/// The panel as a conversation draws it, with the Terminal open in it.
Future<({SessionSurfaces open, TerminalSurface surface, ChatSession session})> _panel(WidgetTester tester, _Host host, SessionStore store) async {
  final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: '/work/app');
  final terminal = TerminalSurface((_) => TerminalOnHost(host));
  final registry = WorkSurfaces([terminal]);
  addTearDown(registry.dispose);
  final open = registry.of(session.id)..openSurface(terminal, SurfaceSlot.bottom);
  addTearDown(() => tester.runAsync(() => Future.wait([for (final each in open.open) each.controller.release()])));
  await _show(
    tester,
    SurfaceEnvironment(
      store: store,
      onComment: (_) {},
      onOpenChild: () {},
      child: WorkPanel(slot: SurfaceSlot.bottom, session: session, surfaces: open, offered: registry.availableFor(session, SurfaceSlot.bottom), layout: ShellLayout()),
    ),
  );
  await _settle(tester);
  return (open: open, surface: terminal, session: session);
}

/// Types as a person does: through the platform's text input, which the terminal has once it has the keyboard.
Future<void> _typeText(WidgetTester tester, String text) async {
  expect(tester.testTextInput.isRegistered, isTrue, reason: 'no text input is connected: the terminal does not have the keyboard');
  tester.testTextInput.enterText(text);
  await tester.pump();
}

void main() {
  _frameTests();
  _sectionTests();
  for (final asApp in [false, true]) {
    group(asApp ? 'in the window' : 'alone', () {
      setUp(() => _asApp = asApp);
      _tests();
    });
  }
}

void _tests() {
  testWidgets('the bottom panel: the first shell has the keyboard, and text and Enter reach the host', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    expect(host.opened, hasLength(1));

    await _typeText(tester, 'ls');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ls\r');
  });

  testWidgets('Windows: letters arrive through the platform text input, and Enter as a key', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    expect(tester.testTextInput.isRegistered, isTrue);
    expect(tester.testTextInput.setClientArgs?['viewId'], isNotNull, reason: 'the text input carries the view id, which Windows needs');

    tester.testTextInput.enterText('abc');
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'abc\r');
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));

  testWidgets("an input method's composition reaches the host once it is committed, and Enter during it is the input method's", (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    expect(tester.testTextInput.isRegistered, isTrue);

    tester.testTextInput.updateEditingValue(const TextEditingValue(text: 'ni', selection: TextSelection.collapsed(offset: 2), composing: TextRange(start: 0, end: 2)));
    await tester.pump();
    expect(host.opened.single.writes, isEmpty, reason: 'a composition in progress is not sent');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(host.opened.single.writes, isEmpty, reason: 'Enter while composing belongs to the input method');

    tester.testTextInput.updateEditingValue(const TextEditingValue(text: '\u4f60', selection: TextSelection.collapsed(offset: 1)));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), '\u4f60\r');
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));

  testWidgets('Linux: letters arrive through the platform text input, and Enter as a key', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    expect(tester.testTextInput.isRegistered, isTrue);

    tester.testTextInput.enterText('abc');
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'abc\r');
  }, variant: TargetPlatformVariant.only(TargetPlatform.linux));

  testWidgets('the bottom panel: after a click in the screen, typing reaches the host', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    await _click(tester);
    await tester.pump();
    await _typeText(tester, 'pwd');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'pwd\r');
  });

  testWidgets('the bottom panel: a second shell from the + menu, and going back to the first, each take typing', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    await _panel(tester, host, store);
    await tester.tap(_key('panel-add'));
    await tester.pumpAndSettle();
    await tester.tap(_key('open terminal'));
    await _settle(tester);
    expect(host.opened, hasLength(2));

    await _typeText(tester, 'two');
    await _settle(tester);
    expect(host.opened.map((shell) => shell.writes.join()), ['', 'two'], reason: 'the shown shell is the second');
    expect(host.opened[0].writes, isEmpty, reason: 'nothing typed went to the shell that is hidden');

    await tester.tap(_key('tab terminal:1'));
    await tester.pumpAndSettle();
    await _click(tester);
    await tester.pump();
    await _typeText(tester, 'one');
    await _settle(tester);
    expect(host.opened[0].writes.join(), 'one');
  });

  testWidgets('the Terminal section: a shell takes typing', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: '/work/app');
    final surface = TerminalSurface((_) => TerminalOnHost(host));
    final controller = SurfaceController('terminal');
    addTearDown(() => tester.runAsync(controller.release));
    await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
    await _settle(tester);
    expect(find.byType(TerminalView), findsOneWidget);
    await _typeText(tester, 'ok');
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ok');
  });
}

/// A click in the middle of the screen; the double-tap timer runs out before the test ends.
Future<void> _click(WidgetTester tester) async {
  await tester.tapAt(const Offset(450, 200));
  await tester.pump(const Duration(seconds: 1));
}

Widget _frame(SessionStore store, ChatSession session, WorkSurfaces registry, ShellLayout layout, TextEditingController composer) => MaterialApp(
  theme: themeFor(LookId.canoa, Brightness.dark),
  home: Scaffold(
    body: PanelFrame(
      layout: layout,
      session: session,
      surfaces: registry.of(session.id),
      registry: registry,
      wrap: (panels) => SurfaceEnvironment(store: store, onComment: (_) {}, onOpenChild: () {}, child: panels),
      child: Material(child: TextField(key: const ValueKey('composer'), controller: composer, autofocus: true)),
    ),
  ),
);

void _frameTests() {
  testWidgets('the panel hidden and shown again: the composer gets the typing while it is hidden, the shell once it is back', (tester) async {
    final host = _Host();
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(title: 'T', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-t', cwd: '/work/app');
    final registry = WorkSurfaces([TerminalSurface((_) => TerminalOnHost(host))]);
    addTearDown(registry.dispose);
    final layout = ShellLayout();
    final composer = TextEditingController();
    addTearDown(composer.dispose);
    await tester.binding.setSurfaceSize(const Size(900, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(_frame(store, session, registry, layout, composer));
    await tester.pump();

    // Opened as the key does: the shell takes the keyboard from the composer.
    toggleBottomPanel(surfaces: registry, layout: layout, session: session);
    await tester.pump();
    await _settle(tester);
    addTearDown(() => tester.runAsync(() => Future.wait([for (final each in registry.of(session.id).open) each.controller.release()])));
    expect(host.opened, hasLength(1));
    await _typeText(tester, 'first');
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'first');
    expect(composer.text, isEmpty);

    // Hidden: the shell is kept, and it no longer takes what is typed.
    toggleBottomPanel(surfaces: registry, layout: layout, session: session);
    await tester.pump();
    await _settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'first', reason: 'a hidden shell does not hear the keyboard');

    // Shown again: it takes the keyboard back.
    toggleBottomPanel(surfaces: registry, layout: layout, session: session);
    await tester.pump();
    await _settle(tester);
    await _typeText(tester, 'second');
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'firstsecond');
  });
}

void _sectionTests() {
  testWidgets('the Terminal section: choosing it gives the shell the keyboard, and leaving it takes the keyboard from the shell', (tester) async {
    final host = _Host();
    final workspace = TerminalWorkspace(host: host, cwd: '/work')..newTab();
    addTearDown(() => tester.runAsync(workspace.release));
    final composer = TextEditingController();
    addTearDown(composer.dispose);
    await tester.binding.setSurfaceSize(const Size(1100, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: AppShell(
        layout: ShellLayout(),
        narrow: false,
        sections: [
          AppSection(id: 'chats', label: 'Chats', icon: Icons.chat_bubble_outline, main: Material(child: TextField(key: const ValueKey('composer'), controller: composer, autofocus: true))),
          AppSection(id: 'terminal', label: 'Terminal', icon: Icons.terminal, main: TerminalPanel(workspace: workspace, focusOnMount: true)),
        ],
      ),
    ));
    await tester.pump();
    await tester.tap(_key('composer'));
    await tester.pump();
    await _typeText(tester, 'hello');
    expect(composer.text, 'hello');

    await tester.tap(_key('section terminal'));
    await tester.pump();
    await _settle(tester);
    await _typeText(tester, 'ls');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ls\r');
    expect(composer.text, 'hello', reason: 'the composer is out of sight and hears nothing');

    await tester.tap(_key('section chats'));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ls\r', reason: 'the shell is out of sight and hears nothing');

    await tester.tap(_key('section terminal'));
    await tester.pump();
    await _settle(tester);
    await _typeText(tester, 'pwd');
    await _settle(tester);
    expect(host.opened.single.writes.join(), 'ls\rpwd');
  });
}
