// The windows of the web app as a bare Flutter page for a browser test: the session list, the Files panel and a terminal's shells of
// the real app, over two conversations answered by an agent in the page ("echo: TEXT") and a shell that runs what is typed ("ran:
// LINE"). A conversation, a file and a terminal each open in a browser window of their own, which is this same page started as that
// window. Built beside the app with
//   flutter build web --release --no-web-resources-cdn -t tool/windows_page.dart -o <dir>
// and driven by `clients/browser/chromium/e2e/web_windows.mjs`, which reads what the page holds through `canoaWindows*`.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/acp/client.dart';
import 'package:canoa/popout/item_windows.dart' show ItemWindows;
import 'package:canoa/popout/platform.dart' as platform;
import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/rail/session_rail.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/shell/surface.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

/// An agent in the page: it answers a prompt with `echo: TEXT` and ends the turn.
final class _Echo {
  _Echo(this.id);

  final String id;
  final incoming = StreamController<String>();
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store, String title) async {
    client = VerifiedAgent.fromChild(
      label: 'Echo',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final method = message['method'];
        if (method == 'initialize') {
          _reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (method == 'session/new') {
          _reply(message['id'], {'sessionId': id});
        } else if (method == 'session/prompt') {
          final text = ((message['params'] as Map)['prompt'] as List).map((part) => (part as Map)['text'] ?? '').join();
          incoming.add(jsonEncode({
            'jsonrpc': '2.0',
            'method': 'session/update',
            'params': {'sessionId': id, 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'echo: $text'}}},
          }));
          _reply(message['id'], {'stopReason': 'end_turn'});
        } else if (message['id'] != null && method != null) {
          _reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/ws');
    final session = store.addProviderSession(title: title, agent: 'Echo', peerId: client.peerId, remoteSessionId: opened.id, cwd: '/ws');
    store.attachVerifiedClient(session.id, client);
    return session;
  }

  void _reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
}

/// A folder of two files on a host that holds nothing else.
final class _Files implements WorkspaceFiles {
  final reads = <String>[];

  static const _texts = {'/ws/notes.txt': 'notes of the host', '/ws/todo.txt': 'todo of the host'};

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async => HostFolderListing(
    path: '/ws',
    parent: null,
    entries: [for (final entry in _texts.keys) HostFolderEntry(name: entry.substring(4), path: entry, isFolder: false)],
    more: false,
  );

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    reads.add(path);
    final bytes = Uint8List.fromList(utf8.encode(_texts[path] ?? ''));
    return HostFileRead(path: path, name: path.substring(4), size: bytes.length, kind: 'text', truncated: false, bytes: bytes);
  }
}

/// A shell that echoes what is typed and runs a line when Enter comes: `ran: LINE`.
final class _Shell implements RemoteTerminal {
  final _out = StreamController<List<int>>();
  final writes = <String>[];
  var _line = '';

  @override
  String get folder => '/ws';
  @override
  String get shell => 'page-shell';
  @override
  Stream<List<int>> get output => _out.stream;
  @override
  Future<int?> get exited => Completer<int?>().future;
  @override
  Future<void> write(String text) async {
    writes.add(text);
    for (final rune in text.runes) {
      final char = String.fromCharCode(rune);
      if (char == '\r') {
        _out.add(utf8.encode('\r\nran: $_line\r\n'));
        _line = '';
      } else {
        _line += char;
        _out.add(utf8.encode(char));
      }
    }
  }

  @override
  Future<void> resize(int cols, int rows) async {}
  @override
  Future<void> close() async {}
}

final class _Host implements TerminalHost {
  _Host(this.shells$);

  final List<_Shell> shells$;

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final made = _Shell();
    shells$.add(made);
    return made;
  }

  @override
  Future<List<HostShell>> shells() async => const [];
}

/// What the framework holds of the focus and of the text field that has it, in whichever window: the view it is drawn in, and the
/// field's text, selection and composing range, which only the framework knows (the browser's own input may have moved on).
String _focusState() {
  final primary = FocusManager.instance.primaryFocus;
  final context = primary?.context;
  final field = context?.findAncestorStateOfType<EditableTextState>();
  final value = field?.widget.controller.value;
  return jsonEncode({
    'view': context == null ? null : View.maybeOf(context)?.viewId,
    'field': value == null ? null : {'text': value.text, 'selection': [value.selection.start, value.selection.end], 'composing': [value.composing.start, value.composing.end]},
  });
}

int _frames = 0;
final _keys = <String>[];

Future<void> main(List<String> args) async {
  final popped = platform.popoutLaunch(args);
  if (popped != null) return runPopout(popped);
  WidgetsFlutterBinding.ensureInitialized();
  // What the checks of the keyboard and of the frames read: the keys the framework heard, and how many frames it drew.
  HardwareKeyboard.instance.addHandler((event) {
    _keys.add('${event is KeyDownEvent ? 'down' : event is KeyUpEvent ? 'up' : 'repeat'}:${event.logicalKey.keyLabel}${HardwareKeyboard.instance.isControlPressed ? '+ctrl' : ''}');
    if (_keys.length > 60) _keys.removeAt(0);
    return false;
  });
  SchedulerBinding.instance.addPersistentFrameCallback((_) => _frames++);

  final store = SessionStore();
  final first = await _Echo('s1').start(store, 'Fix calc');
  await _Echo('s2').start(store, 'Plan docs');
  store.select(first.id);
  final windows = platform.createPopoutWindows();
  final hub = PopoutHub(store, windows);
  final items = ItemWindows(windows, store: store);
  final shells = <_Shell>[];
  final files = _Files();
  final controller = SurfaceController(TerminalSurface.surfaceId);
  final terminal = TerminalSurface((session) => TerminalOnHost(_Host(shells)));
  final home = GlobalKey();
  final setting = ValueNotifier('this_window');

  // Settings, Appearance, Open in: what main.dart does with `[appearance] open_in`.
  void openIn(String value) {
    hub.openInNewWindows = value == 'new_windows';
    items.openInNewWindows = value == 'new_windows';
    setting.value = value;
  }

  String describe() => jsonEncode({
    'selected': store.sessions.where((session) => session.id == store.selectedId).firstOrNull?.title,
    'sessions': [
      for (final session in store.sessions) {'title': session.title, 'poppedOut': hub.isPoppedOut(session), 'messages': [for (final message in session.messages) message.text]},
    ],
    'windows': [for (final window in windows.windows) window.title],
    'items': items.keys,
    'filesRead': files.reads,
    'openInNewWindows': items.openInNewWindows,
    'shellsOpened': shells.length,
    'shellWrites': [for (final shell in shells) shell.writes.join()],
    'shellScreen': terminal.workspaceFor(controller, store.sessions.first)?.focused?.terminal.buffer.getText() ?? '',
  });

  globalContext['canoaWindowsFocus'] = (() => _focusState().toJS).toJS;
  globalContext['canoaWindowsFrames'] = (() => _frames.toJS).toJS;
  globalContext['canoaWindowsKeys'] = (() => jsonEncode(_keys).toJS).toJS;
  globalContext['canoaWindowsKeysReset'] = (_keys.clear).toJS;
  globalContext['canoaWindowsOpenIn'] = ((JSString value) => openIn(value.toDart)).toJS;
  globalContext['canoaWindowsState'] = (() => describe().toJS).toJS;
  globalContext['canoaWindowsOpenSession'] = ((JSString title) {
    final session = store.sessions.firstWhere((each) => each.title == title.toDart);
    unawaited(hub.popOut(session));
  }).toJS;
  globalContext['canoaWindowsOpenTerminal'] = (() {
    unawaited(terminal.openInWindow(home.currentContext!, first, controller));
  }).toJS;

  final filesSurface = FilesSurface((session) => files);
  final filesController = SurfaceController(FilesSurface.surfaceId);
  platform.startApp(windows, MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: ThemeData.dark(),
    home: ListenableBuilder(
      listenable: Listenable.merge([store, hub, items, setting]),
      builder: (context, _) => Scaffold(
        body: SurfaceEnvironment(
          store: store,
          onComment: (_) {},
          onOpenChild: () {},
          windows: items,
          child: Builder(
            key: home,
            builder: (context) => Row(children: [
              SizedBox(
                width: 320,
                child: SessionRail(
                  store: store,
                  onNewSession: () {},
                  onSearch: () {},
                  onClose: store.close,
                  onOpenWindow: hub.popOut,
                  opensInWindow: hub.openInNewWindows,
                ),
              ),
              Expanded(
                child: Column(children: [
                  // A text field of the page's own view, for the checks of the focus between the page and its windows.
                  const Padding(padding: EdgeInsets.all(8), child: TextField(decoration: InputDecoration(labelText: 'opener field'))),
                  Expanded(child: filesSurface.build(context, first, filesController)),
                ]),
              ),
            ]),
          ),
        ),
      ),
    ),
  ));
}
