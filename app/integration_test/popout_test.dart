// A conversation popped out into a second window of the same engine, on a real desktop. The app runs with the platform's own windows
// (flutter_test's test binding replaces them with stand-ins that have no native window, so this is an app and a driver, as Flutter's
// own windowing tests are), and the driver asks it to act and to say what it sees.
//
//   flutter drive --driver=test_driver/popout_test.dart --target=integration_test/popout_test.dart -d windows   (or -d linux)
//
// The window draws the conversation the main window held, which is then a tab of that window and of no other (the main window
// shows its next tab). A request answered in the window is answered once, a message written there and one the app sends go to the
// one queue, and a reply arrives in the window. Closing the pop-out closes only it, and its tab goes back to the main window; closing
// the main window puts it away in the tray and leaves the pop-out where it is. A button in the pop-out is pressed by calling what it
// calls: a driver cannot reach a second window.
//
// After that, several windows at once: two sessions, a file's viewer and a terminal, each in a window of its own and each live: a
// message sent in a session's window reaches that session's agent, the file's text is drawn in its window, and a command typed in
// the terminal window runs in a real shell that goes on running when the window is closed.
//
// Last, tabs: the two sessions as tabs of the main window (closing their windows sent them there), one moved to a window of its own
// through its tab's menu, a message sent in each, and the tab moved back, each step read from the strips and the chats on screen.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:canoa/main.dart';
import 'package:canoa/popout/hub_tabs.dart' show TabbedWindow;
import 'package:canoa/popout/platform.dart' as platform;
import 'package:canoa/popout/popout_view.dart';
import 'package:canoa/popout/popout_window.dart';
import 'package:canoa/chat/inspector_surfaces.dart' show builtInSurfaces;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart' show PanelFrame;
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:canoa/tabs/session_tab_strip.dart';
import 'package:canoa/workspace/open_in.dart' show workspaceOpener;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity;
import 'package:material_ui/material_ui.dart';
// ignore: depend_on_referenced_packages
import 'package:flutter_driver/driver_extension.dart';
import 'package:window_manager/window_manager.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import '../test/fixtures/app_fixture.dart' show CountedWorktrees, FakeClipboard, scripted;
import '../test/fixtures/fake_host.dart' show defaultLocalSources, linkedHost;
import '../test/fixtures/open_host.dart';
import '../test/fixtures/scratch_host.dart';
import '../test/fixtures/scripted_agent.dart';
import '../test/fixtures/source_choices.dart' show FakeHost;
import 'windows_close.dart';

/// Every element of the whole tree, which spans the windows of this engine, that [test] accepts.
List<Element> _elements(bool Function(Element element) test) {
  final found = <Element>[];
  void visit(Element element) {
    if (test(element)) found.add(element);
    element.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement!.visitChildren(visit);
  return found;
}

bool _inWindow(Element element) {
  var inside = false;
  element.visitAncestorElements((ancestor) {
    if (ancestor.widget is PopoutChat || ancestor.widget is TabbedWindow) inside = true;
    return !inside;
  });
  return inside;
}

PopoutChat? get _chat => _elements((e) => e.widget is PopoutChat).map((e) => e.widget as PopoutChat).firstOrNull;

/// A folder of this computer, as a host's files answer for it.
final class _Disk implements WorkspaceFiles {
  _Disk(this.home);

  final Directory home;

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    final dir = Directory(path ?? home.path);
    final entries = [
      for (final each in dir.listSync())
        if (each is Directory || files) HostFolderEntry(name: each.uri.pathSegments.where((part) => part.isNotEmpty).last, path: each.path, isFolder: each is Directory),
    ];
    final parent = dir.parent.path;
    return HostFolderListing(path: dir.path, parent: parent == dir.path ? null : parent, entries: entries, more: false);
  }

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    final bytes = File(path).readAsBytesSync();
    return HostFileRead(path: path, name: path.split(RegExp(r'[\\/]')).last, size: bytes.length, kind: 'text', truncated: false, bytes: bytes);
  }
}

/// A shell that is a real program of this computer, with pipes for a terminal: what is typed is a line it runs.
final class _RealShell implements RemoteTerminal {
  _RealShell(this.process, this.folder) {
    process.stdout.listen(_out.add, onDone: () {});
    process.stderr.listen(_out.add, onDone: () {});
  }

  final Process process;
  final _out = StreamController<List<int>>();

  @override
  final String folder;

  @override
  String get shell => Platform.isWindows ? 'cmd' : 'sh';

  @override
  Stream<List<int>> get output => _out.stream;

  @override
  Future<int?> get exited => process.exitCode;

  @override
  Future<void> write(String text) async {
    process.stdin.add(utf8.encode(text.replaceAll('\r', Platform.isWindows ? '\r\n' : '\n')));
    await process.stdin.flush();
  }

  @override
  Future<void> resize(int cols, int rows) async {}

  @override
  Future<void> close() async {
    process.kill();
  }
}

final class _RealShells implements TerminalHost {
  final opened = <_RealShell>[];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final process = Platform.isWindows ? await Process.start('cmd.exe', ['/Q'], workingDirectory: cwd) : await Process.start('sh', const [], workingDirectory: cwd);
    final made = _RealShell(process, cwd ?? '');
    opened.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

final _shells = _RealShells();

/// The line that makes the shell print the product of [a] and [b].
String _multiply(int a, int b) => Platform.isWindows ? 'set /a $a*$b\r' : 'echo \$(($a*$b))\r';

Element? _first(bool Function(Element element) test) => _elements(test).firstOrNull;

/// The window of the conversation [session] is shown in, as its chat.
PopoutChat? _chatOf(ChatSession session) => _elements((e) => e.widget is PopoutChat && identical((e.widget as PopoutChat).conversation.session, session.chat)).map((e) => e.widget as PopoutChat).firstOrNull;

/// Whether [element] has a drawn text that says [needle] somewhere below it.
bool _textBelow(Element element, String needle) {
  var found = false;
  void visit(Element each) {
    if (found) return;
    final widget = each.widget;
    if ((widget is RichText && widget.text.toPlainText().contains(needle)) || (widget is EditableText && widget.controller.text.contains(needle))) {
      found = true;
      return;
    }
    each.visitChildren(visit);
  }

  element.visitChildren(visit);
  return found;
}

bool _inChatOf(ChatSession session, Element element) {
  var inside = false;
  element.visitAncestorElements((ancestor) {
    final widget = ancestor.widget;
    if (widget is PopoutChat && identical(widget.conversation.session, session.chat)) inside = true;
    return !inside;
  });
  return inside;
}

/// How many drawn texts say [needle] in [session]'s window.
int _saidIn(ChatSession session, String needle) =>
    _elements((e) => e.widget is RichText && (e.widget as RichText).text.toPlainText().contains(needle) && _inChatOf(session, e)).length;

/// The tab strips on screen: the main window's and each window's own, as the titles of their tabs and the one shown.
List<Map<String, Object?>> _strips() => [
  for (final element in _elements((e) => e.widget is SessionTabStrip))
    {
      // The new-session page the app opens on is a tab too; the titles are of the conversations, in alphabetical order.
      'tabs': [for (final session in (element.widget as SessionTabStrip).sessions) if (!session.isDraft) session.title]..sort(),
      'current': (element.widget as SessionTabStrip).sessions.where((session) => session.id == (element.widget as SessionTabStrip).current).map((s) => s.title).firstOrNull,
      'inWindow': _inTabbedWindow(element),
    },
];

bool _inTabbedWindow(Element element) {
  var inside = false;
  element.visitAncestorElements((ancestor) {
    if (ancestor.widget is TabbedWindow) inside = true;
    return !inside;
  });
  return inside;
}

/// Presses the menu button of [session]'s tab and then the entry [entry] of the menu, as a person would.
Future<void> _tabMenu(ChatSession session, Key entry) async {
  final button = _first((e) => e.widget is IconButton && e.widget.key == Key('tab-menu-${session.id}'));
  if (button == null) throw StateError('no menu button on the tab of ${session.title}');
  (button.widget as IconButton).onPressed!();
  Element? item() => _first((e) => e.widget is PopupMenuItem && e.widget.key == entry);
  final end = DateTime.now().add(const Duration(seconds: 10));
  while (item() == null) {
    if (DateTime.now().isAfter(end)) throw StateError('the menu did not offer $entry');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  InkWell? tap;
  void find(Element each) {
    if (each.widget is InkWell && tap == null) tap = each.widget as InkWell;
    each.visitChildren(find);
  }

  item()!.visitChildren(find);
  tap!.onTap!();
}

bool _isPrompt(Map<String, dynamic> message) => message['method'] == 'session/prompt';

Future<void> main() async {
  final scene = Completer<_Scene>();
  enableFlutterDriverExtension(
    handler: (message) async {
      final ready = await scene.future;
      try {
        return jsonEncode(await ready.handle(message ?? 'state'));
      } on Object catch (error, stack) {
        return jsonEncode({'error': '$error', 'stack': '$stack'});
      }
    },
  );
  scene.complete(await _Scene.start());
}

final class _Scene {
  _Scene(this.store, this.agent, this.session, this.windows, this.shell, this.layout, this.folder, this.files, this.terminals);

  final SessionStore store;
  final ScriptedAgent agent;
  final ChatSession session;
  final PopoutWindows windows;
  final TrayShell shell;
  final ShellLayout layout;
  final Directory folder;
  final FilesSurface files;
  final TerminalSurface terminals;

  /// What the steps after the pop-out report beyond [state].
  final Map<String, Object?> extra = {};

  /// The second conversation and its agent.
  late ChatSession second;
  late ScriptedAgent secondAgent;

  static Future<_Scene> start() async {
    final store = SessionStore();
    final windows = platform.createPopoutWindows();
    final shell = TrayShell();
    const config = CanoaConfig(agents: [scripted], density: ChatDensity.verbose);
    await shell.initialize(config, onSettings: () {}, onHosting: () {});
    // Nothing of the person's: a folder of its own for the config and for sessions, and a host that starts nothing.
    final dir = Directory.systemTemp.createTempSync('canoa-popout');
    File('${dir.path}/notes.txt').writeAsStringSync('the text of the notes file');
    workspaceOpener = await OpenHost().opener();
    final layout = ShellLayout();
    final files = FilesSurface((_) => _Disk(dir));
    final terminals = TerminalSurface((_) => TerminalOnHost(_shells));
    runApp(
      windows.host(
        CanoaApp(
          hostFolders: ScratchHost(dir.path, template: config.sessions.workspace),
          store: store,
          config: config,
          configPath: '${dir.path}/canoa.cfg',
          writeConfig: (_) async {},
          isGitRepository: (_) => false,
          folderExists: (_) => true,
          pickFolder: () async => '/work/repo',
          pickFiles: () async => const [],
          statFile: (_) async => null,
          clipboard: FakeClipboard(),
          worktrees: () => CountedWorktrees(),
          shell: shell,
          host: linkedHost((name) => throw StateError('nothing starts in this test: $name'), sources: () => [...defaultLocalSources, ('Scripted', null)]),
          askChoices: FakeHost().call,
          popoutWindows: windows,
          layout: layout,
          surfaces: [...builtInSurfaces(), files, terminals],
        ),
      ),
    );
    await Future<void>.delayed(const Duration(seconds: 1));
    final agent = ScriptedAgent();
    final session = await agent.start(store);
    // Under v1 a prompt answers when its turn ends, which this one never does.
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    agent.ask(5, 'Edit a.txt');
    agent.ask(6, 'Edit b.txt');
    final scene = _Scene(store, agent, session, windows, shell, layout, dir, files, terminals);
    await scene.until(() => session.pendingPermissions.length == 2, 'the agent\'s requests to be waiting');
    return scene;
  }

  Future<void> until(FutureOr<bool> Function() done, String what, {Duration within = const Duration(seconds: 30)}) async {
    final end = DateTime.now().add(within);
    while (!await done()) {
      if (DateTime.now().isAfter(end)) throw StateError('timed out waiting for $what');
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
  }

  /// How many drawn texts say [needle], in the pop-out window and elsewhere.
  Map<String, int> _said(String needle) {
    final texts = _elements((e) => e.widget is RichText && (e.widget as RichText).text.toPlainText().contains(needle));
    return {'window': texts.where(_inWindow).length, 'main': texts.where((e) => !_inWindow(e)).length};
  }

  Future<Map<String, Object?>> state() async => {
    'windows': [
      for (final window in windows.windows) {'id': window.id, 'title': window.title, 'link': window.link != null, 'width': window.size.width, 'height': window.size.height},
    ],
    'chats': _elements((e) => e.widget is PopoutChat).length,
    'pending': [for (final q in session.pendingPermissions) q.title],
    'settled': [for (final q in session.chat.settledPermissions) '${q.outcome?.by}'],
    'queue': [for (final m in session.queue) m.text],
    'clients': [for (final c in session.chat.clients) c.id],
    'status': session.status.name,
    'answers': agent.answers().map((m) => m['id']).toList(),
    'prompts': agent.sent.where((m) => m['method'] == 'session/prompt').length,
    'visible': await windowManager.isVisible(),
    'tray': shell.hasTray,
    'extra': extra,
    'strips': _strips(),
    'said': {
      'Reading calc.js.': _said('Reading calc.js.'),
      'And it is fixed.': _said('And it is fixed.'),
      'Edit b.txt': _said('Edit b.txt'),
      'Edit c.txt': _said('Edit c.txt'),
      'Answered in Canoa.': _said('Answered in Canoa.'),
      '2 messages wait for this turn to end.': _said('2 messages wait for this turn to end.'),
    },
  };

  Future<Map<String, Object?>> handle(String message) async {
    switch (message) {
      case 'state':
        break;
      case 'open':
        final button = _elements((e) => e.widget is IconButton && (e.widget as IconButton).tooltip == 'Open in new window').single.widget as IconButton;
        button.onPressed!();
        await until(() => windows.windows.length == 1 && _chat != null, 'the window to draw the conversation');
      case 'answer_in_window':
        final question = session.pendingPermissions.first;
        _chat!.conversation.choosePermission(question, question.options.first);
        await until(() => agent.answerTo(5) != null, 'the window\'s answer to reach the agent');
      case 'answer_in_main':
        store.choosePermission(session, session.pendingPermissions.first.options.first);
        await until(() => agent.answerTo(6) != null, 'the main window answer to reach the agent');
        await until(() => _said('Answered in Canoa.')['window']! > 0, 'the window to say where it was answered');
      case 'both_send':
        await Future.wait([store.sendTextTo(session, 'from the main window', attached: false), _chat!.conversation.send('from the window')]);
        await until(() => session.queue.length == 2, 'both messages to be queued');
        await until(() => _said('2 messages wait for this turn to end.')['window']! > 0, 'the window to show the queue');
      case 'reply':
        agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': ' And it is fixed.'}});
        agent.ask(7, 'Edit c.txt');
        await until(() => _said('And it is fixed.')['window']! > 0 && _said('Edit c.txt')['window']! > 0, 'the reply and the new request in the window');
      case 'close_popout':
        if (Platform.isWindows) {
          if (!closeWindowTitled('Fix calc - Canoa')) throw StateError('no window to close among ${windowTitles()}');
        } else {
          await windows.windows.single.close();
        }
        await until(() => windows.windows.isEmpty && _chat == null, 'the window to close');
      case 'close_main':
        await windowManager.close();
        if (shell.hasTray) await until(() async => !await windowManager.isVisible(), 'the main window to go to the tray');
        await Future<void>.delayed(const Duration(seconds: 1));
      case 'restore':
        await shell.open();
        await until(() async => await windowManager.isVisible() && !await windowManager.isMinimized(), 'the main window');
      case 'dock':
        final dock = _elements((e) => e.widget is IconButton && _inWindow(e) && (e.widget as IconButton).tooltip == 'Bring back to the main window').single.widget as IconButton;
        dock.onPressed!();
        await until(() => windows.windows.isEmpty, 'the window to dock back');
      case 'open_sessions':
        _header().onPressed!();
        await until(() => windows.windows.length == 1 && _chatOf(session) != null, 'the first session\'s window');
        secondAgent = ScriptedAgent();
        second = await secondAgent.start(store);
        second.title = 'Second';
        second.cwd = folder.path;
        await until(() => _elements((e) => e.widget is IconButton && (e.widget as IconButton).tooltip == 'Open in new window').length == 1, 'the second session to be shown');
        _header().onPressed!();
        await until(() => windows.windows.length == 2 && _chatOf(second) != null, 'the second session\'s window');
      case 'send_second':
        final chat = _chatOf(second)!;
        await chat.conversation.send('typed in the second window');
        await until(() => secondAgent.sent.any((m) => _isPrompt(m) && jsonEncode(m).contains('typed in the second window')), 'the message to reach the second agent');
        await until(() => _saidIn(second, 'typed in the second window') > 0, 'the second window to show it');
        extra['second prompts'] = secondAgent.sent.where(_isPrompt).length;
        extra['first prompts'] = agent.sent.where(_isPrompt).length;
        extra['shown in the first window'] = _saidIn(session, 'typed in the second window');
        secondAgent.ask(9, 'Edit z.txt');
        await until(() => second.pendingPermissions.length == 1 && _saidIn(second, 'Edit z.txt') > 0, 'the second agent\'s request in its window');
        extra['first window waits on z'] = _saidIn(session, 'Edit z.txt');
        final question = second.pendingPermissions.first;
        chat.conversation.choosePermission(question, question.options.first);
        await until(() => secondAgent.answerTo(9) != null, 'the answer given in the second window to reach its agent');
        extra['first agent was answered 9'] = agent.answerTo(9) != null;
        extra['z settled by'] = [for (final q in second.chat.settledPermissions) '${q.outcome?.by}'];
      case 'second_home':
        // The files panel is the main window's, and the main window shows only what is its own tab: the second session comes back.
        final back = _elements((e) => e.widget is IconButton && _inChatOf(second, e) && (e.widget as IconButton).tooltip == 'Bring back to the main window').single.widget as IconButton;
        back.onPressed!();
        await until(() => windows.windows.length == 1 && store.selectedId == second.id, 'the second session to be back in the main window');
      case 'open_file':
        final frame = _first((e) => e.widget is PanelFrame)!.widget as PanelFrame;
        frame.surfaces.openSurface(files, SurfaceSlot.right);
        layout.showRight();
        await until(() => _first((e) => e.widget is ListTile && e.widget.key == const ValueKey('files entry notes.txt')) != null, 'the folder to be listed');
        (_first((e) => e.widget is ListTile && e.widget.key == const ValueKey('files entry notes.txt'))!.widget as ListTile).onTap!();
        await until(() => _first((e) => e.widget is IconButton && e.widget.key == const ValueKey('files window')) != null, 'the file to be previewed');
        (_first((e) => e.widget is IconButton && e.widget.key == const ValueKey('files window'))!.widget as IconButton).onPressed!();
        await until(() => windows.windows.any((w) => w.title == 'notes.txt'), 'the file\'s window');
        // The window's panel is the one with no button for a window of its own.
        Element? windowPanel() => _first((e) => e.widget is FilesPanel && (e.widget as FilesPanel).onOpenWindow == null);
        await until(() => windowPanel() != null && _textBelow(windowPanel()!, 'the text of the notes file'), 'the file\'s text in its window');
        extra['files panels'] = _elements((e) => e.widget is FilesPanel).length;
      case 'open_terminal':
        final frame = _first((e) => e.widget is PanelFrame)!.widget as PanelFrame;
        final open = frame.surfaces.openSurface(terminals, SurfaceSlot.bottom);
        layout.showBottom();
        await until(() => _first((e) => e.widget is TerminalPanel) != null, 'the terminal panel');
        await terminals.openInWindow(_first((e) => e.widget is TerminalPanel)!, second, open.controller);
        await until(() => windows.windows.any((w) => w.title == 'Shell 1: Second'), 'the terminal\'s window');
        final workspace = TerminalSurface.openWorkspace(open.controller)!;
        await until(() => workspace.focused?.state == PaneState.running, 'the shell to start');
        await until(() => _elements((e) => e.widget is TerminalView).length == 1, 'one screen, the window\'s');
        extra['screens'] = _elements((e) => e.widget is TerminalView).length;
        extra['panel says it is in a window'] = _first((e) => e.widget.key == const ValueKey('terminal in window')) != null;
        workspace.focused!.terminal.textInput(_multiply(6, 7));
        await until(() => workspace.focused!.terminal.buffer.getText().contains('42'), 'the shell to print what was run in the window');
        extra['shells opened'] = _shells.opened.length;
      case 'close_terminal_window':
        final workspace = TerminalSurface.openWorkspace(((_first((e) => e.widget is PanelFrame)!.widget) as PanelFrame).surfaces.find('terminal')!.controller)!;
        if (Platform.isWindows) {
          if (!closeWindowTitled('Shell 1: Second - Canoa')) throw StateError('no terminal window to close among ${windowTitles()}');
        } else {
          await windows.windows.firstWhere((w) => w.title == 'Shell 1: Second').close();
        }
        await until(() => !windows.windows.any((w) => w.title == 'Shell 1: Second') && !workspace.inWindow, 'the terminal\'s window to close');
        await until(() => _elements((e) => e.widget is TerminalView).length == 1 && _first((e) => e.widget.key == const ValueKey('terminal in window')) == null, 'the panel to draw the shell again');
        extra['shell state'] = workspace.focused!.state.name;
        extra['shells after'] = _shells.opened.length;
        workspace.focused!.terminal.textInput(_multiply(7, 8));
        await until(() => workspace.focused!.terminal.buffer.getText().contains('56'), 'the same shell to answer once more');
        extra['screen keeps the first answer'] = workspace.focused!.terminal.buffer.getText().contains('42');
      case 'tabs_start':
        // Closing the windows of the two sessions sent their tabs to the main window.
        await until(() => _strips().any((strip) => strip['inWindow'] == false && (strip['tabs'] as List).length == 2), 'the two sessions to be tabs of the main window');
      case 'tabs_move':
        await _tabMenu(second, const Key('tab-move-new'));
        await until(() => windows.windows.length == 1 && _chatOf(second) != null, 'the window of the second session');
        await until(() => _strips().every((strip) => (strip['tabs'] as List).length != 2), 'the main window to lose the tab');
        extra['chat of the first in a window'] = _chatOf(session) != null;
      case 'tabs_send':
        await _chatOf(second)!.conversation.send('typed in the tab window');
        await store.sendText('typed in the main tab');
        await until(() => _saidIn(second, 'typed in the tab window') > 0 || second.queue.any((m) => m.text == 'typed in the tab window'), 'the second session to take its message');
        extra['second queue'] = [for (final m in second.queue) '${m.text} from ${m.from}'];
        extra['first queue'] = [for (final m in session.queue) '${m.text} from ${m.from}'];
        extra['second prompts'] = secondAgent.sent.where(_isPrompt).length;
        extra['first prompts'] = agent.sent.where(_isPrompt).length;
        await until(() => _saidIn(second, '1 message waits for this turn to end.') > 0, 'the window to show its queue');
        extra['window says one waits'] = _saidIn(second, '1 message waits for this turn to end.');
      case 'tabs_back':
        final dock = _elements((e) => e.widget is IconButton && _inWindow(e) && (e.widget as IconButton).tooltip == 'Bring back to the main window').single.widget as IconButton;
        dock.onPressed!();
        await until(() => windows.windows.isEmpty && _strips().any((strip) => (strip['tabs'] as List).length == 2), 'the tab to be back in the main window');
        extra['clients after'] = [for (final c in second.chat.clients) c.id];
      case 'finish':
        final frame = _first((e) => e.widget is PanelFrame)!.widget as PanelFrame;
        for (final window in [...windows.windows]) {
          await window.close();
        }
        frame.surfaces.close('terminal');
        extra['shell ended with its terminal'] = await Future.any([_shells.opened.single.exited.then((_) => true), Future.delayed(const Duration(seconds: 10), () => false)]);
        folder.deleteSync(recursive: true);
      default:
        throw ArgumentError('unknown message $message');
    }
    return state();
  }
}

/// The button that opens the conversation shown in a window of its own.
IconButton _header() => _elements((e) => e.widget is IconButton && (e.widget as IconButton).tooltip == 'Open in new window').single.widget as IconButton;
