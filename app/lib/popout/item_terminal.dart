import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/foundation.dart' show VoidCallback;

import '../desktop/control_client.dart' show ControlFailed;
import '../settings/shell_settings.dart' show HostShell;
import '../surfaces/terminal_controller.dart';

// A terminal in a window that is another page. The shells are the page-that-opened-it's: the host ties a shell to the connection it
// was opened on, so the window cannot hold one of its own. It draws the shells over a link instead: what a shell prints goes to it,
// what is typed in it goes to the shell, and closing the window leaves the shells running, as a window of the engine does.

/// Serves [workspace]'s shells to the window at the far end of [link]. Returns what stops it; the shells go on.
VoidCallback serveTerminal(TerminalWorkspace workspace, ChatViewLink link) => _Server(workspace, link).stop;

final class _Server {
  _Server(this.workspace, this.link) {
    _heard = link.messages.listen(_ask);
    workspace.addListener(_changed);
  }

  final TerminalWorkspace workspace;
  final ChatViewLink link;
  late final StreamSubscription<Map<String, Object?>> _heard;
  final _ids = <TerminalPane, int>{};
  final _panes = <int, TerminalPane>{};
  final _watching = <int, (StreamSubscription<List<int>>, VoidCallback)>{};
  final _told = <TerminalTab>{};
  var _next = 0;
  var _opening = 0;
  var _stopped = false;

  void stop() {
    if (_stopped) return;
    _stopped = true;
    workspace.removeListener(_changed);
    unawaited(_heard.cancel());
    for (final id in _watching.keys.toList()) {
      _forget(id);
    }
  }

  void _send(Map<String, Object?> message) {
    if (!_stopped) link.send(message);
  }

  Future<void> _ask(Map<String, Object?> message) async {
    final pane = _panes[(message['pane'] as num?)?.toInt()];
    switch (message['t']) {
      case 'hello':
        await _hello();
      case 'open':
        await _open(message);
      case 'in':
        if (pane != null && message['data'] is String) await _typed(pane, message['data'] as String);
      case 'size':
        final cols = (message['cols'] as num?)?.toInt(), rows = (message['rows'] as num?)?.toInt();
        if (pane != null && cols != null && rows != null) {
          try {
            await pane.remote?.resize(cols, rows);
          } on Object {
            // A size the host refused leaves the shell at the last one.
          }
        }
      case 'close':
        if (pane != null) _close(pane);
    }
  }

  Future<void> _typed(TerminalPane pane, String text) async {
    try {
      await pane.remote?.write(text);
    } on Object catch (error) {
      // The host holds only so many keystrokes for a shell that is not reading; say so where the person looks.
      final why = switch (error) { ControlFailed(:final message) => message, _ => 'the input was not sent' };
      _send({'t': 'out', 'pane': _ids[pane], 'data': base64Encode(utf8.encode('\r\n\x1b[31m[$why]\x1b[0m\r\n'))});
    }
  }

  void _close(TerminalPane pane) {
    final id = _ids[pane];
    if (id != null) _forget(id);
    for (final tab in workspace.tabs.toList()) {
      if (tab.panes.contains(pane)) workspace.closePane(tab, pane);
    }
  }

  Future<void> _hello() async {
    final tabs = workspace.tabs.toList();
    _told.addAll(tabs);
    await Future.wait([for (final tab in tabs) _settled(tab)]);
    if (_stopped) return;
    _send({
      't': 'tabs',
      'tabs': [for (final tab in tabs) if (workspace.tabs.contains(tab)) _tabOut(tab)],
      'active': tabs.contains(workspace.active) ? tabs.indexOf(workspace.active!) : 0,
    });
  }

  /// A shell the window asked for: a tab of the workspace, so that closing the window leaves it where the person finds it.
  Future<void> _open(Map<String, Object?> message) async {
    final request = message['req'];
    _opening++;
    final TerminalTab? tab;
    try {
      tab = workspace.newTab(shell: message['shell'] as String?);
    } finally {
      _opening--;
    }
    if (tab == null) {
      _send({'t': 'refused', 'req': request, 'message': 'The host holds ${TerminalWorkspace.limit} shells at most'});
      return;
    }
    _told.add(tab);
    await _settled(tab);
    final pane = tab.panes.first;
    if (pane.state == PaneState.failed) {
      _send({'t': 'refused', 'req': request, 'message': pane.problem ?? 'The shell could not be started.'});
      workspace.closeTab(tab);
      return;
    }
    _send({'t': 'opened', 'req': request, 'pane': _paneOut(pane)});
  }

  void _changed() {
    if (_opening > 0 || _stopped) return;
    for (final tab in workspace.tabs.toList()) {
      if (!_told.add(tab)) continue;
      unawaited(_announce(tab));
    }
  }

  /// A tab made after the window opened (a new shell from the panel's menu): the window gets it as one of its own.
  Future<void> _announce(TerminalTab tab) async {
    await _settled(tab);
    if (_stopped || !workspace.tabs.contains(tab)) return;
    _send({'t': 'tab', 'tab': _tabOut(tab)});
  }

  Future<void> _settled(TerminalTab tab) => Future.wait([for (final pane in tab.panes) _settledPane(pane)]);

  Future<void> _settledPane(TerminalPane pane) {
    if (pane.state != PaneState.starting) return Future.value();
    final done = Completer<void>();
    void check() {
      if (pane.state == PaneState.starting) return;
      pane.removeListener(check);
      done.complete();
    }

    pane.addListener(check);
    return done.future;
  }

  Map<String, Object?> _tabOut(TerminalTab tab) => {
    'name': tab.name,
    'panes': [for (final pane in tab.panes) _paneOut(pane)],
  };

  Map<String, Object?> _paneOut(TerminalPane pane) {
    final id = _ids.putIfAbsent(pane, () => ++_next);
    _panes[id] = pane;
    if (!_watching.containsKey(id)) {
      final printing = pane.rawOutput.listen((bytes) => _send({'t': 'out', 'pane': id, 'data': base64Encode(bytes)}));
      var told = false;
      void ended() {
        if (told || pane.state == PaneState.running || pane.state == PaneState.starting) return;
        told = true;
        _send({'t': 'exit', 'pane': id, 'code': pane.exitCode});
      }

      pane.addListener(ended);
      _watching[id] = (printing, () => pane.removeListener(ended));
    }
    final remote = pane.remote;
    return {
      'id': id,
      'shell': remote?.shell ?? '',
      'folder': remote?.folder ?? '',
      'history': base64Encode(pane.recentOutput),
      'state': pane.state.name,
      'code': pane.exitCode,
    };
  }

  void _forget(int id) {
    final watch = _watching.remove(id);
    if (watch != null) {
      unawaited(watch.$1.cancel());
      watch.$2();
    }
    final pane = _panes.remove(id);
    if (pane != null) _ids.remove(pane);
  }
}

/// A tab the page that holds the shells has, as the window sees it.
final class LinkedTab {
  const LinkedTab(this.name, this.shells);

  final String name;

  /// The shells in it: one, or two side by side.
  final List<RemoteTerminal> shells;
}

/// The shells of the page that opened this window, as a [TerminalHost]: opening one asks that page for a shell of the workspace it
/// holds. [adopt] gives a tab the window's workspace the shells that already run there.
final class LinkedTerminals implements TerminalHost {
  LinkedTerminals(this.link, {this.onTabs, this.onTab}) {
    _heard = link.messages.listen(_hear, onDone: () => _endAll('The page that held these shells was closed'));
  }

  final ChatViewLink link;

  /// Told the tabs the page holds (the answer to [hello]) and the index of the one it shows, and each tab it makes later.
  final void Function(List<LinkedTab> tabs, int active)? onTabs;
  final void Function(LinkedTab tab)? onTab;
  late final StreamSubscription<Map<String, Object?>> _heard;
  final _shells = <int, _Shell>{};
  final _adopting = Queue<RemoteTerminal>();
  final _requests = <int, Completer<RemoteTerminal>>{};
  var _asked = 0;

  /// Asks the page for its tabs.
  void hello() => link.send({'t': 'hello'});

  /// Makes a tab of [workspace] that draws [tab]'s shells.
  void adopt(TerminalWorkspace workspace, LinkedTab tab) {
    _adopting.addAll(tab.shells);
    final made = workspace.newTab();
    if (made == null) {
      _adopting.clear();
      return;
    }
    if (tab.shells.length > 1) workspace.split(made);
    workspace.rename(made, tab.name);
  }

  @override
  Future<List<HostShell>> shells() async => const [];

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) {
    if (_adopting.isNotEmpty) return Future.value(_adopting.removeFirst());
    if (link.isClosed) return Future.error(const ControlFailed(-32603, 'The page that held these shells was closed'));
    final request = ++_asked;
    final answer = _requests[request] = Completer<RemoteTerminal>();
    link.send({'t': 'open', 'req': request, 'shell': shell, 'cols': cols, 'rows': rows});
    return answer.future;
  }

  _Shell _shellOf(Object? raw) {
    final map = raw as Map;
    final id = (map['id'] as num).toInt();
    final held = _shells[id];
    if (held != null) return held;
    final made = _Shell(link, id, '${map['shell']}', '${map['folder']}');
    _shells[id] = made;
    final history = map['history'];
    if (history is String && history.isNotEmpty) made._print(base64Decode(history));
    if (map['state'] == 'exited' || map['state'] == 'failed') made._ended((map['code'] as num?)?.toInt());
    return made;
  }

  LinkedTab _tabOf(Object? raw) {
    final map = raw as Map;
    return LinkedTab('${map['name']}', [for (final pane in map['panes'] as List) _shellOf(pane)]);
  }

  void _hear(Map<String, Object?> message) {
    switch (message['t']) {
      case 'tabs':
        onTabs?.call([for (final tab in message['tabs'] as List) _tabOf(tab)], (message['active'] as num?)?.toInt() ?? 0);
      case 'tab':
        onTab?.call(_tabOf(message['tab']));
      case 'opened':
        _requests.remove(message['req'])?.complete(_shellOf(message['pane']));
      case 'refused':
        _requests.remove(message['req'])?.completeError(ControlFailed(-32002, '${message['message']}'));
      case 'out':
        final data = message['data'];
        if (data is String) _shells[(message['pane'] as num?)?.toInt()]?._print(base64Decode(data));
      case 'exit':
        _shells[(message['pane'] as num?)?.toInt()]?._ended((message['code'] as num?)?.toInt());
      case 'ended':
        _endAll('${message['reason'] ?? 'The page that held these shells was closed'}');
    }
  }

  void _endAll(String why) {
    for (final shell in _shells.values.toList()) {
      shell._ended(null);
    }
    for (final waiting in _requests.values.toList()) {
      waiting.completeError(ControlFailed(-32603, why));
    }
    _requests.clear();
  }

  /// Stops listening. The shells are not ended: they are the other page's.
  Future<void> close() => _heard.cancel();
}

final class _Shell implements RemoteTerminal {
  _Shell(this._link, this.id, this.shell, this.folder);

  final ChatViewLink _link;
  final int id;
  final _bytes = StreamController<List<int>>();
  final _exit = Completer<int?>();
  var _over = false;

  @override
  final String shell;
  @override
  final String folder;

  @override
  Stream<List<int>> get output => _bytes.stream;

  @override
  Future<int?> get exited => _exit.future;

  void _print(List<int> bytes) {
    if (!_bytes.isClosed) _bytes.add(bytes);
  }

  void _ended(int? code) {
    if (_over) return;
    _over = true;
    if (!_exit.isCompleted) _exit.complete(code);
    unawaited(_bytes.close());
  }

  @override
  Future<void> write(String text) async {
    if (!_over) _link.send({'t': 'in', 'pane': id, 'data': text});
  }

  @override
  Future<void> resize(int cols, int rows) async {
    if (!_over) _link.send({'t': 'size', 'pane': id, 'cols': cols, 'rows': rows});
  }

  @override
  Future<void> close() async {
    final was = _over;
    _ended(null);
    if (!was && !_link.isClosed) _link.send({'t': 'close', 'pane': id});
  }
}
