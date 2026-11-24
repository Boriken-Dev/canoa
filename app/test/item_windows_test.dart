// A file's viewer and a terminal in windows of their own, over the seam's stand-in: one window per item, the same window when it is
// asked for again, the app's own words and look in it, a title that follows the item, and a shell that is the host's and outlives
// the window that shows it.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/popout/item_windows.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/popout_windows.dart';
import 'fixtures/sample_render.dart' show loadTerminalFont;

/// A folder with one file in it, on a host that holds nothing else.
final class _Files implements WorkspaceFiles {
  _Files(this.text);

  final String text;

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async => const HostFolderListing(
    path: '/ws',
    parent: null,
    entries: [HostFolderEntry(name: 'notes.txt', path: '/ws/notes.txt', isFolder: false)],
    more: false,
  );

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    final bytes = Uint8List.fromList(utf8.encode(text));
    return HostFileRead(path: path, name: 'notes.txt', size: bytes.length, kind: 'text', truncated: false, bytes: bytes);
  }
}

/// A shell the test drives.
final class _Shell implements RemoteTerminal {
  @override
  String get folder => '/ws';
  @override
  String get shell => 'test-shell';
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  final writes = <String>[];
  bool closed = false;

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
    closed = true;
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

Finder _key(String name) => find.byKey(ValueKey(name));

/// Lets the stand-ins' futures and streams run, which the test clock does not.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump(const Duration(milliseconds: 50));
}

ChatSession _session(SessionStore store, {String title = 'Fix calc'}) =>
    store.addProviderSession(title: title, agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-$title', cwd: '/ws');

typedef _Scene = ({SessionStore store, ChatSession session, FakeWindows windows, ItemWindows items});

_Scene _scene(WidgetTester tester, {bool sameEngine = true, WindowFrame? frame}) {
  final store = SessionStore();
  addTearDown(store.dispose);
  final windows = FakeWindows(sameEngine: sameEngine);
  final items = ItemWindows(windows, store: store, frame: frame);
  addTearDown(items.dispose);
  return (store: store, session: _session(store), windows: windows, items: items);
}

/// What the window [index] of [windows] draws, beside [main], the way the app's root draws both.
Future<void> _showBoth(WidgetTester tester, Widget main, FakeWindows windows) async {
  await tester.pumpWidget(
    ListenableBuilder(
      listenable: windows,
      builder: (context, _) => Row(
        textDirection: TextDirection.ltr,
        children: [
          Expanded(child: main),
          for (final window in windows.opened)
            if (!window.isClosed) Expanded(child: Builder(builder: window.builder!)),
        ],
      ),
    ),
  );
  await _settle(tester);
}

Widget _app(Widget child, {String? language}) => MaterialApp(
  debugShowCheckedModeBanner: false,
  theme: themeFor(LookId.canoa, Brightness.dark),
  builder: (context, home) => AppTextHost(language: language, child: home!),
  home: Scaffold(body: child),
);

void main() {
  setUpAll(loadTerminalFont);

  group('an item\'s window', () {
    testWidgets('opens once for its key, and asking again brings that window forward', (tester) async {
      final s = _scene(tester);
      await s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one'));
      await s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('two'));
      expect(s.windows.opened, hasLength(1));
      expect(s.windows.opened.single.title, 'Notes');
      expect(s.windows.opened.single.focused, 1);
      expect(s.items.isOpen('a'), isTrue);

      await s.items.show(key: 'b', session: s.session.id, title: 'Other', builder: (_) => const Text('three'));
      expect(s.windows.opened, hasLength(2), reason: 'another item has a window of its own');
      expect(s.items.keys, ['a', 'b']);
    });

    testWidgets('two requests for one item made together make one window', (tester) async {
      final s = _scene(tester);
      await Future.wait([
        s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one')),
        s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one')),
      ]);
      expect(s.windows.opened, hasLength(1));
    });

    testWidgets('closing the window ends it and says so once, and the item can be opened again', (tester) async {
      final s = _scene(tester);
      var closed = 0;
      await s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one'), onClosed: () => closed++);
      s.windows.opened.single.closeByUser();
      await tester.pump();
      expect(s.items.isOpen('a'), isFalse);
      expect(closed, 1);

      await s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one'), onClosed: () => closed++);
      expect(s.windows.opened, hasLength(2));
      await s.items.close('a');
      await tester.pump();
      expect(closed, 2);
      expect(s.windows.opened.last.isClosed, isTrue);
    });

    testWidgets('a window goes with the session it belongs to', (tester) async {
      final s = _scene(tester);
      final other = _session(s.store, title: 'Other');
      await s.items.show(key: 'a', session: s.session.id, title: 'Notes', builder: (_) => const Text('one'));
      await s.items.show(key: 'b', session: other.id, title: 'Other notes', builder: (_) => const Text('two'));
      await tester.runAsync(() => s.store.close(other));
      await tester.pump();
      expect(s.items.keys, ['a']);
      expect(s.windows.opened.last.isClosed, isTrue);
    });

    testWidgets('its title follows what it asks for as the item changes', (tester) async {
      final s = _scene(tester);
      final changes = ValueNotifier('one');
      addTearDown(changes.dispose);
      await s.items.show(
        key: 'a',
        session: s.session.id,
        title: 'one',
        titleOf: () => changes.value,
        changes: changes,
        builder: (_) => const Text('x'),
      );
      changes.value = 'two';
      expect(s.windows.opened.single.title, 'two');
      s.windows.opened.single.closeByUser();
      await tester.pump();
      changes.value = 'three';
      expect(s.windows.opened.single.title, 'two', reason: 'a window that is gone is not retitled');
    });

    testWidgets('the frame gives the content the app\'s words and look, and a window without one still has a theme', (tester) async {
      final framed = _scene(
        tester,
        frame: (context, home) => MaterialApp(
          theme: themeFor(LookId.terminal, Brightness.dark),
          builder: (c, h) => AppTextHost(language: 'es', child: h!),
          home: home,
        ),
      );
      await framed.items.show(
        key: 'a',
        session: framed.session.id,
        title: 'x',
        builder: (context) => Text('${context.t('window.open')}|${Theme.of(context).extension<CanoaLook>() != null}'),
      );
      await _showBoth(tester, const SizedBox(), framed.windows);
      expect(find.text('Abrir en una ventana nueva|true'), findsOneWidget);

      final plain = _scene(tester);
      await plain.items.show(
        key: 'a',
        session: plain.session.id,
        title: 'x',
        builder: (context) => Text('${context.t('window.open')}|${Theme.of(context).extension<CanoaLook>() != null}'),
      );
      await _showBoth(tester, const SizedBox(), plain.windows);
      expect(find.text('Open in new window|true'), findsOneWidget);
    });

    testWidgets('a platform whose windows are other pages cannot show one, and the person is told why', (tester) async {
      final s = _scene(tester, sameEngine: false);
      expect(s.items.available, isFalse);
      await expectLater(s.items.show(key: 'a', session: s.session.id, title: 'x', builder: (_) => const Text('x')), throwsStateError);
      expect(s.windows.opened, isEmpty);

      await tester.pumpWidget(
        _app(
          Builder(
            builder: (context) => TextButton(
              onPressed: () =>
                  unawaited(showItemWindow(context, s.items, key: 'a', session: s.session.id, title: 'x', builder: (_) => const Text('x'))),
              child: const Text('go'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('go'));
      await tester.pump();
      await tester.pump();
      expect(find.textContaining('Could not open a window: This platform has no windows'), findsOneWidget);
    });
  });

  group('a file\'s viewer', () {
    Future<_Scene> pane(WidgetTester tester, {String text = 'hello from notes', String? language, WindowFrame? frame}) async {
      final s = _scene(tester, frame: frame);
      final surface = FilesSurface((_) => _Files(text));
      final controller = SurfaceController('files');
      await tester.binding.setSurfaceSize(const Size(1000, 600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await _showBoth(
        tester,
        _app(
          SurfaceEnvironment(
            store: s.store,
            onComment: (_) {},
            onOpenChild: () {},
            windows: s.items,
            child: Builder(builder: (context) => surface.build(context, s.session, controller)),
          ),
          language: language,
        ),
        s.windows,
      );
      await tester.tap(_key('files entry notes.txt'));
      await _settle(tester);
      expect(find.text('hello from notes'), findsOneWidget);
      return s;
    }

    testWidgets('the preview offers a window, which opens on that file and shows its text', (tester) async {
      final s = await pane(tester);
      await tester.tap(_key('files window'));
      await _settle(tester);
      expect(s.windows.opened, hasLength(1));
      expect(s.windows.opened.single.title, 'notes.txt');
      expect(s.items.isOpen(FilesSurface.windowKey(s.session, '/ws/notes.txt')), isTrue);
      await _settle(tester);
      await _settle(tester);

      expect(
        find.text('hello from notes'),
        findsNWidgets(2),
        reason: 'the pane and the window both show the file, the window having opened on it and not on its folder',
      );
      expect(_key('files name'), findsNWidgets(2));
    });

    testWidgets('asking for the same file again brings its window forward', (tester) async {
      final s = await pane(tester);
      await tester.tap(_key('files window'));
      await _settle(tester);
      await tester.tap(_key('files window'));
      await _settle(tester);
      expect(s.windows.opened, hasLength(1));
      expect(s.windows.opened.single.focused, 1);
    });

    testWidgets('closing the window leaves the pane on the file as it was', (tester) async {
      final s = await pane(tester);
      await tester.tap(_key('files window'));
      await _settle(tester);
      s.windows.opened.single.closeByUser();
      await _settle(tester);
      expect(s.items.keys, isEmpty);
      expect(find.text('hello from notes'), findsOneWidget);
      expect(_key('files name'), findsOneWidget);
    });

    testWidgets('with no windows there is no button', (tester) async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = _session(store);
      final surface = FilesSurface((_) => _Files('x'));
      await tester.pumpWidget(_app(Builder(builder: (context) => surface.build(context, session, SurfaceController('files')))));
      await _settle(tester);
      await tester.tap(_key('files entry notes.txt'));
      await _settle(tester);
      expect(_key('files window'), findsNothing);
    });
  });

  group('a terminal', () {
    Future<({_Scene scene, _Host host, SessionSurfaces open, TerminalSurface surface})> panel(
      WidgetTester tester, {
      WindowFrame? frame,
    }) async {
      final s = _scene(tester, frame: frame);
      final host = _Host();
      final surface = TerminalSurface((_) => TerminalOnHost(host));
      final registry = WorkSurfaces([surface]);
      addTearDown(registry.dispose);
      final open = registry.of(s.session.id)..openSurface(surface, SurfaceSlot.bottom);
      addTearDown(() => tester.runAsync(() => Future.wait([for (final each in open.open) each.controller.release()])));
      await tester.binding.setSurfaceSize(const Size(1200, 500));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await _showBoth(
        tester,
        _app(
          SurfaceEnvironment(
            store: s.store,
            onComment: (_) {},
            onOpenChild: () {},
            windows: s.items,
            child: WorkPanel(
              slot: SurfaceSlot.bottom,
              session: s.session,
              surfaces: open,
              offered: registry.availableFor(s.session, SurfaceSlot.bottom),
              layout: ShellLayout(),
            ),
          ),
        ),
        s.windows,
      );
      return (scene: s, host: host, open: open, surface: surface);
    }

    Future<void> openWindow(WidgetTester tester) async {
      await tester.tap(_key('panel-more'));
      await tester.pumpAndSettle();
      await tester.tap(_key('action terminal window'));
      await _settle(tester);
    }

    testWidgets('the actions menu opens it in a window, and the panel says where the shell went', (tester) async {
      final p = await panel(tester);
      final workspace = p.surface.workspaceFor(p.open.find('terminal')!.controller, p.scene.session)!;
      expect(_key('terminal in window'), findsNothing);
      await openWindow(tester);

      expect(p.scene.windows.opened, hasLength(1));
      expect(p.scene.windows.opened.single.title, 'Shell 1: Fix calc');
      expect(workspace.inWindow, isTrue);
      expect(_key('terminal in window'), findsOneWidget);
      expect(p.host.opened, hasLength(1), reason: 'the window shows the shell the panel had: it opened no second one');
    });

    testWidgets('what is typed in the window reaches the host shell, and what the shell prints shows there', (tester) async {
      final p = await panel(tester);
      final workspace = p.surface.workspaceFor(p.open.find('terminal')!.controller, p.scene.session)!;
      await openWindow(tester);

      p.host.opened.single.print('PS /ws> ');
      await _settle(tester);
      final pane = workspace.focused!;
      expect(pane.terminal.buffer.getText(), contains('PS /ws> '));
      expect(find.byKey(ValueKey('terminal view ${pane.hashCode}')), findsOneWidget, reason: 'one screen is drawn: the windows one');
      pane.terminal.textInput('ls\r');
      await _settle(tester);
      expect(p.host.opened.single.writes.join(), 'ls\r');
    });

    testWidgets('closing the window leaves the shell running, and the panel draws it again', (tester) async {
      final p = await panel(tester);
      final workspace = p.surface.workspaceFor(p.open.find('terminal')!.controller, p.scene.session)!;
      await openWindow(tester);
      p.host.opened.single.print('kept\r\n');
      await _settle(tester);

      p.scene.windows.opened.single.closeByUser();
      await _settle(tester);
      expect(workspace.inWindow, isFalse);
      expect(_key('terminal in window'), findsNothing);
      expect(
        find.byKey(ValueKey('terminal view ${workspace.focused!.hashCode}')),
        findsOneWidget,
        reason: 'the panel draws the screen again',
      );
      expect(p.host.opened.single.closed, isFalse, reason: 'the shell is the host own: the window was only a view');
      expect(workspace.shells, 1);
      expect(workspace.focused!.terminal.buffer.getText(), contains('kept'), reason: 'its screen is as it was');
    });

    testWidgets('the panel action brings it back, closing the window, and a second window can be opened after', (tester) async {
      final p = await panel(tester);
      final workspace = p.surface.workspaceFor(p.open.find('terminal')!.controller, p.scene.session)!;
      await openWindow(tester);
      expect(workspace.inWindow, isTrue);

      await openWindow(tester);
      expect(p.scene.windows.opened.single.isClosed, isTrue, reason: 'while it is in a window the same entry brings it back');
      expect(workspace.inWindow, isFalse);

      await openWindow(tester);
      expect(p.scene.windows.opened, hasLength(2));
      expect(workspace.inWindow, isTrue);
    });

    testWidgets('closing the terminal itself ends its shells and closes its window', (tester) async {
      final p = await panel(tester);
      await openWindow(tester);

      await tester.runAsync(() async {
        p.open.close('terminal');
        await Future<void>.delayed(const Duration(milliseconds: 30));
      });
      await tester.pump();
      expect(p.host.opened.single.closed, isTrue);
      expect(p.scene.windows.opened.single.isClosed, isTrue);
      expect(p.scene.items.keys, isEmpty);
    });

    testWidgets('without windows the menu has no such action', (tester) async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = _session(store);
      final host = _Host();
      final surface = TerminalSurface((_) => TerminalOnHost(host));
      final registry = WorkSurfaces([surface]);
      addTearDown(registry.dispose);
      final open = registry.of(session.id)..openSurface(surface, SurfaceSlot.bottom);
      addTearDown(() => tester.runAsync(() => Future.wait([for (final each in open.open) each.controller.release()])));
      await tester.pumpWidget(
        _app(
          SurfaceEnvironment(
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
      );
      await _settle(tester);
      await tester.tap(_key('panel-more'));
      await tester.pumpAndSettle();
      expect(_key('action terminal window'), findsNothing);
      expect(_key('action terminal split'), findsOneWidget);
    });
  });
}
