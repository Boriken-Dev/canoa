// A file's viewer and a terminal in a window that is another page, as a browser opens one: the window is told what to show and the
// page that opened it serves the rest over their link. The platform's windows are the seam's stand-in (a window with a link in
// memory), so what runs here is the seam's use, both ends of the file and terminal links, and the setting, not the browser.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/popout/item_files.dart';
import 'package:canoa/popout/item_page.dart';
import 'package:canoa/popout/item_terminal.dart';
import 'package:canoa/popout/item_windows.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/popout/popout_launch.dart';
import 'package:canoa/popout/popout_window.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/popout_windows.dart';

/// A host's files: one folder with one file in it, which can be written.
final class _Files implements WorkspaceFiles, WorkspaceFileWriter {
  _Files(this.text);

  String text;
  final reads = <String>[];

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async => const HostFolderListing(
    path: '/ws',
    parent: null,
    entries: [HostFolderEntry(name: 'notes.txt', path: '/ws/notes.txt', isFolder: false)],
    more: false,
  );

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    reads.add(path);
    final bytes = Uint8List.fromList(utf8.encode(text));
    return HostFileRead(path: path, name: 'notes.txt', size: bytes.length, kind: 'text', truncated: false, bytes: bytes);
  }

  @override
  Future<HostFileRead> readWhole(String path) => readFile(path);

  @override
  Future<int> writeFile(String path, String contents) async {
    text = contents;
    return utf8.encode(contents).length;
  }
}

/// A shell the test drives.
final class _Shell implements RemoteTerminal {
  _Shell(this.number);

  final int number;
  final _out = StreamController<List<int>>();
  final _exit = Completer<int?>();
  final writes = <String>[];
  final sizes = <(int, int)>[];
  bool closed = false;

  @override
  String get folder => '/ws';
  @override
  String get shell => 'test-shell-$number';

  void print(String text) => _out.add(utf8.encode(text));

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
    if (closed) return;
    closed = true;
    if (!_exit.isCompleted) _exit.complete(null);
    await _out.close();
  }
}

final class _Host implements TerminalHost {
  final opened = <_Shell>[];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final made = _Shell(opened.length + 1);
    opened.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

/// Lets the stand-ins' futures and streams run, which the test clock does not.
Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 60)));
  await tester.pump(const Duration(milliseconds: 50));
}

ChatSession _session(SessionStore store) => store.addProviderSession(title: 'Fix calc', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-1', cwd: '/ws');

({SessionStore store, ChatSession session, FakeWindows windows, ItemWindows items}) _scene({bool links = true}) {
  final store = SessionStore();
  addTearDown(store.dispose);
  final windows = FakeWindows(linksItems: links);
  final items = ItemWindows(windows, store: store);
  addTearDown(items.dispose);
  return (store: store, session: _session(store), windows: windows, items: items);
}

/// A host's files that do not let a file be written, whatever the page that serves them says.
final class _ReadOnly implements WorkspaceFiles {
  _ReadOnly(this.files);

  final WorkspaceFiles files;

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) => this.files.listFolders(path: path, hidden: hidden, files: files);

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) => files.readFile(path, maxBytes: maxBytes);
}

/// The files [files] as a window of its own gets them: a window for a file is made, and its end of the link is returned.
Future<LinkedFiles> _linkedFiles(({SessionStore store, ChatSession session, FakeWindows windows, ItemWindows items}) s, WorkspaceFiles files, {required bool writable}) async {
  await s.items.show(
    key: 'files:${s.windows.opened.length}',
    session: s.session.id,
    title: 'f',
    builder: (_) => const Text('x'),
    item: const PopoutItem(PopoutItem.file),
    serve: (link) => serveFiles(files, link),
  );
  return LinkedFiles.of(s.windows.opened.last.far!, writable: writable);
}

PopoutLaunch _launch(FakeWindow window) => PopoutLaunch(link: 'test', title: window.title, item: window.item);

void main() {
  group('the seam of a platform whose windows are other pages', () {
    test('a file or a terminal is shown only where a window can link one', () {
      expect(_scene().items.available, isTrue);
      expect(_scene(links: false).items.available, isFalse, reason: 'a window that is another page and links nothing shows only a conversation');
      final engine = ItemWindows(FakeWindows(sameEngine: true), store: SessionStore());
      addTearDown(engine.dispose);
      expect(engine.available, isTrue, reason: 'a window of this engine draws what it is given');
    });

    testWidgets('a window is given the item to show and a link the item is served over, and it ends serving when it closes', (tester) async {
      final s = _scene();
      var served = 0, stopped = 0;
      await s.items.show(
        key: 'file:a',
        session: s.session.id,
        title: 'notes.txt',
        builder: (_) => const Text('drawn here'),
        item: const PopoutItem(PopoutItem.file, {'root': '/ws', 'path': '/ws/notes.txt'}),
        serve: (link) {
          served++;
          expect(link.isClosed, isFalse);
          return () => stopped++;
        },
      );
      final window = s.windows.opened.single;
      expect(window.item?.kind, PopoutItem.file);
      expect(window.item?.args, {'root': '/ws', 'path': '/ws/notes.txt'});
      expect(window.far, isNotNull, reason: 'the page at the other end is linked');
      expect(served, 1);

      await s.items.show(key: 'file:a', session: s.session.id, title: 'notes.txt', builder: (_) => const Text('x'), item: window.item, serve: (_) => () {});
      expect(s.windows.opened, hasLength(1), reason: 'asking again brings the same window forward');
      expect(window.focused, 1);

      window.closeByUser();
      await tester.pump();
      expect(stopped, 1);
      expect(s.items.isOpen('file:a'), isFalse);
    });

    testWidgets('an item that cannot be served to a page has no window, and says so', (tester) async {
      final s = _scene();
      await expectLater(s.items.show(key: 'a', session: s.session.id, title: 'x', builder: (_) => const Text('x')), throwsStateError);
      expect(s.windows.opened, isEmpty);
    });

    test('the launch a page starts with keeps the item and its arguments', () {
      const launch = PopoutLaunch(link: 'abc', title: 'notes.txt', item: PopoutItem(PopoutItem.file, {'root': '/ws', 'path': r'/ws/a #1.txt', 'lang': 'es'}));
      final back = PopoutLaunch.decode(launch.encode())!;
      expect(back.item?.kind, PopoutItem.file);
      expect(back.item?.args, {'root': '/ws', 'path': r'/ws/a #1.txt', 'lang': 'es'});
      expect(PopoutLaunch.decode(const PopoutLaunch(link: 'abc', title: 'T').encode())?.item, isNull);
    });
  });

  group('a file in a window', () {
    testWidgets('shows the file the page that opened it holds, asked over the link, and a save goes through that page', (tester) async {
      final s = _scene();
      final files = _Files('hello from the host');
      await s.items.show(
        key: 'file:a',
        session: s.session.id,
        title: 'notes.txt',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.file, {'root': '/ws', 'path': '/ws/notes.txt', 'write': '1'}),
        serve: (link) => serveFiles(files, link),
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      await _settle(tester);
      await _settle(tester);
      expect(files.reads, ['/ws/notes.txt'], reason: 'the file was read by the page that holds the host');
      expect(find.textContaining('hello from the host'), findsWidgets);
      expect(find.byKey(const ValueKey('files edit')), findsOneWidget, reason: 'the page allowed writing');

      final linked = await _linkedFiles(s, files, writable: true);
      addTearDown(linked.close);
      final saved = (linked as LinkedWritableFiles).writeFile('/ws/notes.txt', 'changed');
      await _settle(tester);
      expect(await saved, 7);
      expect(files.text, 'changed');
      final read = linked.readFile('/ws/notes.txt', maxBytes: 100);
      await _settle(tester);
      expect(utf8.decode((await read).bytes!), 'changed');
    });

    testWidgets('can be written only when the page that opened it said so', (tester) async {
      final s = _scene();
      await s.items.show(
        key: 'file:a',
        session: s.session.id,
        title: 'notes.txt',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.file, {'root': '/ws', 'path': '/ws/notes.txt'}),
        serve: (link) => serveFiles(_Files('text'), link),
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      await _settle(tester);
      await _settle(tester);
      expect(find.byKey(const ValueKey('files edit')), findsNothing);
    });

    testWidgets('a host that refuses reaches the window in its own words, and a page that is gone fails what is asked', (tester) async {
      final s = _scene();
      final readOnly = await _linkedFiles(s, _ReadOnly(_Files('text')), writable: true);
      addTearDown(readOnly.close);
      final refused = (readOnly as LinkedWritableFiles).writeFile('/x', 'y').then<Object?>((_) => null, onError: (Object error) => error);
      await _settle(tester);
      final said = await refused;
      expect(said, isA<ControlFailed>());
      expect((said! as ControlFailed).message, contains('does not let files be written'));
      expect((said as ControlFailed).code, -32001, reason: 'with the code the host gave');

      final silent = _scene();
      await silent.items.show(key: 'f', session: silent.session.id, title: 'f', builder: (_) => const Text('x'), item: const PopoutItem(PopoutItem.file), serve: (_) => () {});
      final asking = LinkedFiles.of(silent.windows.opened.single.far!, writable: false);
      addTearDown(asking.close);
      final pending = asking.listFolders().then<Object?>((_) => null, onError: (Object error) => error);
      silent.windows.opened.single.link!.send({'t': 'ended', 'reason': 'The page that held these files was closed'});
      await _settle(tester);
      final failed = await pending;
      expect(failed, isA<ControlFailed>(), reason: 'a call the page never answered fails when the page goes');
      expect((failed! as ControlFailed).message, contains('was closed'));
    });
  });

  group('a terminal in a window', () {
    testWidgets('draws the shells of the page that opened it, types into them, and leaves them running when it is closed', (tester) async {
      final s = _scene();
      final host = _Host();
      final workspace = TerminalWorkspace(host: host, cwd: '/ws')..newTab();
      addTearDown(workspace.release);
      await _settle(tester);
      host.opened.single.print('prompt> ');
      await _settle(tester);

      await s.items.show(
        key: 'terminal:a',
        session: s.session.id,
        title: 'Terminal',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.terminal),
        serve: (link) => serveTerminal(workspace, link),
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      await _settle(tester);
      await _settle(tester);
      final shown = tester.widget<TerminalPanel>(find.byType(TerminalPanel)).workspace;
      expect(shown.tabs, hasLength(1));
      expect(shown.active!.focused.terminal.buffer.getText(), contains('prompt> '), reason: 'what the shell printed before the window opened is drawn');

      host.opened.single.print('more');
      await _settle(tester);
      expect(shown.active!.focused.terminal.buffer.getText(), contains('prompt> more'), reason: 'and what it prints after');

      shown.active!.focused.terminal.textInput('ls\r');
      await _settle(tester);
      expect(host.opened.single.writes.join(), contains('ls'), reason: 'what is typed in the window reaches the host\'s shell');
      expect(host.opened, hasLength(1), reason: 'and the window opened no shell of its own for it');

      await tester.pumpWidget(const SizedBox());
      window.closeByUser();
      await tester.pump();
      expect(host.opened.single.closed, isFalse, reason: 'closing the window leaves the shell the host holds');
      expect(workspace.tabs, hasLength(1));
      expect(workspace.active!.focused.state, PaneState.running);
    });

    testWidgets('a shell made in the window is one of the workspace, and closing it there ends it on the host', (tester) async {
      final s = _scene();
      final host = _Host();
      final workspace = TerminalWorkspace(host: host, cwd: '/ws')..newTab();
      addTearDown(workspace.release);
      await _settle(tester);
      await s.items.show(
        key: 'terminal:a',
        session: s.session.id,
        title: 'Terminal',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.terminal),
        serve: (link) => serveTerminal(workspace, link),
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      await _settle(tester);
      await _settle(tester);
      final shown = tester.widget<TerminalPanel>(find.byType(TerminalPanel)).workspace;

      shown.newTab();
      await _settle(tester);
      await _settle(tester);
      expect(host.opened, hasLength(2), reason: 'the host was asked once, by the page that holds the shells');
      expect(workspace.tabs, hasLength(2), reason: 'the shell is a tab there too, which stays when the window goes');
      expect(shown.tabs, hasLength(2));
      shown.active!.focused.terminal.textInput('pwd\r');
      await _settle(tester);
      expect(host.opened.last.writes.join(), contains('pwd'));
      expect(host.opened.first.writes, isEmpty);

      host.opened.last.print('done');
      await _settle(tester);
      expect(shown.active!.focused.terminal.buffer.getText(), contains('done'));

      shown.closeTab(shown.active!);
      await _settle(tester);
      await _settle(tester);
      expect(host.opened.last.closed, isTrue, reason: 'a shell closed in the window is ended');
      expect(host.opened.first.closed, isFalse);
      expect(workspace.tabs, hasLength(1));
    });

    testWidgets('a shell the page makes while the window is open appears in it, and one that ends says so', (tester) async {
      final s = _scene();
      final host = _Host();
      final workspace = TerminalWorkspace(host: host, cwd: '/ws')..newTab();
      addTearDown(workspace.release);
      await _settle(tester);
      await s.items.show(
        key: 'terminal:a',
        session: s.session.id,
        title: 'Terminal',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.terminal),
        serve: (link) => serveTerminal(workspace, link),
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      await _settle(tester);
      await _settle(tester);
      final shown = tester.widget<TerminalPanel>(find.byType(TerminalPanel)).workspace;

      workspace.newTab();
      await _settle(tester);
      await _settle(tester);
      expect(shown.tabs, hasLength(2));
      expect(host.opened, hasLength(2));

      await host.opened.first.close();
      await _settle(tester);
      expect(shown.tabs.first.focused.state, PaneState.failed, reason: 'the link or the shell ended, which the pane tells');
    });

    testWidgets('a page that does not answer leaves the window saying so', (tester) async {
      final s = _scene();
      await s.items.show(
        key: 'terminal:a',
        session: s.session.id,
        title: 'Terminal',
        builder: (_) => const Text('x'),
        item: const PopoutItem(PopoutItem.terminal),
        serve: (link) => () {},
      );
      final window = s.windows.opened.single;
      await tester.pumpWidget(ItemPopoutApp(launch: _launch(window), link: window.far!));
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      await tester.pump(const Duration(seconds: 12));
      expect(find.byKey(const Key('popout-problem')), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });
  });
}
