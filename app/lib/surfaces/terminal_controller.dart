import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show FocusNode;
import 'package:xterm/xterm.dart';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../settings/shell_settings.dart' show HostShell;
import 'terminal_clipboard.dart';

/// A shell on a host, as the Terminal surface drives it. What it prints arrives on [output] as bytes; what the person types goes
/// through [write].
abstract interface class RemoteTerminal {
  /// The program the host started and the folder it started in.
  String get shell;
  String get folder;

  /// What the shell prints, in order, from the first byte. Single-subscription: it holds what came before anyone listened.
  Stream<List<int>> get output;

  /// Completes with the shell's exit status when it ends, or null when the link to the host ended first.
  Future<int?> get exited;

  /// Types [text] into the shell. Writes reach it in the order they were made.
  Future<void> write(String text);

  Future<void> resize(int cols, int rows);

  /// Ends the shell and what it started.
  Future<void> close();
}

/// Where shells are opened: a host's own terminal, or a stand-in in a test.
abstract interface class TerminalHost {
  /// Starts a shell [cols] by [rows] cells in [cwd] (the host's default when null), running [shell], the id of one [shells] lists
  /// (the host's own choice when null). Throws [ControlFailed] with the host's own words when it refuses.
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows});

  /// The shells the host found, which a new terminal may name; none when it lists none or may not say.
  Future<List<HostShell>> shells();
}

/// A host's `terminal/*` over its control channel. The shells belong to the link they were opened on and end with it.
final class ControlTerminals implements TerminalHost {
  ControlTerminals(this._control);

  final HostControl _control;
  ControlClient? _routed;
  StreamSubscription<ControlPush>? _routing;
  final _live = <int, _ControlTerminal>{};

  /// What arrived for a shell before its `terminal/open` answer was handled, replayed once it is.
  final _early = <int, List<ControlPush>>{};

  /// The most pushes held for one shell not yet known; the host sends nothing before it has answered, so this is slack.
  static const earlyLimit = 256;

  @override
  Future<List<HostShell>> shells() async {
    try {
      final answer = await (await _control.ready()).call('shell/list', const {});
      return [if (answer is Map && answer['shells'] is List) for (final each in answer['shells'] as List) ?HostShell.parse(each)];
    } on ControlFailed {
      // A key that may open a terminal but not list what is installed opens the host's own.
      return const [];
    }
  }

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async {
    final client = await _control.ready();
    _route(client);
    final answer = await client.call('terminal/open', {'cols': cols, 'rows': rows, 'cwd': ?cwd, 'shell': ?shell});
    if (answer is! Map || answer['id'] is! num) throw const ControlFailed(-32603, 'the host did not say which terminal it opened');
    final id = (answer['id'] as num).toInt();
    final made = _ControlTerminal(client, id, '${answer['shell'] ?? ''}', '${answer['cwd'] ?? ''}', () => _live.remove(id));
    _live[id] = made;
    unawaited(client.ended.then((_) => made._ended(null)));
    for (final push in _early.remove(id) ?? const <ControlPush>[]) {
      made._push(push);
    }
    return made;
  }

  void _route(ControlClient client) {
    if (identical(_routed, client)) return;
    unawaited(_routing?.cancel());
    _routed = client;
    _early.clear();
    _routing = client.pushes.listen(_push);
  }

  void _push(ControlPush push) {
    final id = push.params['id'];
    if (id is! num) return;
    final shell = _live[id.toInt()];
    if (shell != null) {
      shell._push(push);
      return;
    }
    final held = _early.putIfAbsent(id.toInt(), () => []);
    if (held.length < earlyLimit) held.add(push);
  }
}

final class _ControlTerminal implements RemoteTerminal {
  _ControlTerminal(this._client, this.id, this.shell, this.folder, this._forget);

  final ControlClient _client;
  final int id;
  final VoidCallback _forget;
  final _bytes = StreamController<List<int>>();
  final _exit = Completer<int?>();
  Future<void> _writing = Future.value();
  bool _over = false;

  @override
  final String shell;
  @override
  final String folder;

  @override
  Stream<List<int>> get output => _bytes.stream;

  @override
  Future<int?> get exited => _exit.future;

  void _push(ControlPush push) {
    switch (push.method) {
      case 'terminal/output':
        final data = push.params['data'];
        if (data is String && !_bytes.isClosed) {
          try {
            _bytes.add(base64Decode(data));
          } on FormatException {
            // A chunk that is not base64 is not output.
          }
        }
      case 'terminal/exit':
        _ended((push.params['code'] as num?)?.toInt() ?? 0);
    }
  }

  void _ended(int? code) {
    if (_over) return;
    _over = true;
    if (!_exit.isCompleted) _exit.complete(code);
    unawaited(_bytes.close());
  }

  /// The most raw bytes one `terminal/write` carries.
  static const writeChunk = 16 * 1024;

  @override
  Future<void> write(String text) {
    final bytes = utf8.encode(text);
    final done = _writing.then((_) async {
      if (_over) return;
      for (var at = 0; at < bytes.length; at += writeChunk) {
        final end = at + writeChunk < bytes.length ? at + writeChunk : bytes.length;
        await _client.call('terminal/write', {'id': id, 'data': base64Encode(bytes.sublist(at, end))});
      }
    });
    // A refused write is the caller's to hear; the next one still goes.
    _writing = done.then((_) {}, onError: (Object _) {});
    return done;
  }

  @override
  Future<void> resize(int cols, int rows) async {
    if (_over) return;
    await _client.call('terminal/resize', {'id': id, 'cols': cols, 'rows': rows});
  }

  @override
  Future<void> close() async {
    final was = _over;
    _ended(null);
    _forget();
    if (was || _client.over) return;
    try {
      await _client.call('terminal/close', {'id': id});
    } on Object {
      // Closed or gone: either way the host no longer holds it for this link.
    }
  }
}

/// Where one shell is in its life.
enum PaneState { starting, running, exited, failed }

/// One shell and the screen that shows it: the emulator, the selection, and the shell on the host. Lives in a tab for as long as
/// the tab does, whatever panel it is drawn in.
class TerminalPane extends ChangeNotifier {
  TerminalPane({required this.host, required this.cwd, this.shell, this.maxLines = 10000, this.clipboard = const SystemTerminalClipboard()}) {
    terminal = Terminal(maxLines: maxLines);
    view = TerminalController();
    terminal.onOutput = _typed;
    terminal.onResize = (cols, rows, _, _) => _resized(cols, rows);
    terminal.onTitleChange = (title) {
      shellTitle = title;
      notifyListeners();
    };
  }

  final TerminalHost host;

  /// The clipboard copy and paste use.
  final TerminalClipboard clipboard;

  /// The folder the shell starts in on its host; null for the host's default.
  final String? cwd;

  /// The id of the shell the host starts, one it listed; null for the host's own choice.
  final String? shell;

  /// The lines the screen keeps above what is showing.
  final int maxLines;

  late final Terminal terminal;

  /// The emulator's own view state: the selection.
  late final TerminalController view;

  /// The keyboard's place on the screen. It is the pane's, so it stays with the shell while the screen is hidden and shown again.
  final focusNode = FocusNode(debugLabel: 'terminal');
  RemoteTerminal? _remote;
  StreamSubscription<String>? _printing;
  PaneState state = PaneState.starting;

  /// Why the shell did not start or was lost, in the host's words.
  String? problem;
  int? exitCode;

  /// The shell's own idea of its title (an escape sequence it printed).
  String? shellTitle;
  bool _closed = false;

  /// Opens the shell. A host that refuses, or a link that fails, leaves [problem].
  Future<void> start() async {
    // A shell that ended still counts against the host's limit until it is closed.
    final before = _remote;
    _remote = null;
    await _printing?.cancel();
    _printing = null;
    if (before != null) unawaited(before.close());
    state = PaneState.starting;
    problem = null;
    exitCode = null;
    notifyListeners();
    try {
      final remote = await host.open(cwd: cwd, shell: shell, cols: terminal.viewWidth, rows: terminal.viewHeight);
      if (_closed) {
        await remote.close();
        return;
      }
      _remote = remote;
      _printing = remote.output.transform(const Utf8Decoder(allowMalformed: true)).listen(terminal.write);
      state = PaneState.running;
      notifyListeners();
      if (_pending.isNotEmpty) {
        final text = _pending.toString();
        _pending.clear();
        terminal.textInput(text);
      }
      unawaited(remote.exited.then((code) {
        if (identical(_remote, remote)) _ended(code);
      }));
      // The view may have been laid out while the shell was starting.
      if (terminal.viewWidth != 80 || terminal.viewHeight != 24) unawaited(_resized(terminal.viewWidth, terminal.viewHeight));
    } on Object catch (error) {
      if (_closed) return;
      state = PaneState.failed;
      problem = error is ControlFailed ? error.message : 'The host could not be reached.';
      notifyListeners();
    }
  }

  void _ended(int? code) {
    if (_closed) return;
    exitCode = code;
    state = code == null ? PaneState.failed : PaneState.exited;
    problem = code == null ? 'The link to the host ended, and the shell with it.' : null;
    notifyListeners();
  }

  void _typed(String data) {
    final remote = _remote;
    if (remote == null || state != PaneState.running) return;
    unawaited(remote.write(data).catchError((Object error) {
      // The host holds only so many keystrokes for a shell that is not reading; say so where the person looks.
      terminal.write('\r\n\x1b[31m[${error is ControlFailed ? error.message : 'the input was not sent'}]\x1b[0m\r\n');
    }));
  }

  Future<void> _resized(int cols, int rows) async {
    final remote = _remote;
    if (remote == null || state != PaneState.running) return;
    try {
      await remote.resize(cols, rows);
    } on Object {
      // A size the host refused leaves the shell at the last one.
    }
  }

  /// Types [text] as the person would, with no Enter after it.
  void type(String text) {
    if (state == PaneState.starting) {
      _pending.write(text);
      return;
    }
    terminal.textInput(text);
  }

  /// What was typed while the shell was starting, sent once it is up.
  final _pending = StringBuffer();

  /// The text selected on the screen, or null.
  String? get selectedText {
    final range = view.selection;
    if (range == null) return null;
    final text = terminal.buffer.getText(range);
    return text.trim().isEmpty ? null : text;
  }

  /// The text selected on the screen exactly as it is, blanks included; null when nothing is selected.
  String? get selection {
    final range = view.selection;
    if (range == null) return null;
    final text = terminal.buffer.getText(range);
    return text.isEmpty ? null : text;
  }

  /// Puts the selected text on the clipboard; false when nothing is selected. [clear] drops the selection after, as Windows
  /// Terminal does when Ctrl+C copies, so that the next Ctrl+C interrupts. Throws [ClipboardRefused].
  Future<bool> copy({bool clear = false}) async {
    final text = selection;
    if (text == null) return false;
    if (clear) view.clearSelection();
    await clipboard.write(text);
    return true;
  }

  /// Selects everything the screen holds.
  void selectAll() {
    final buffer = terminal.buffer;
    view.setSelection(
      buffer.createAnchor(0, 0),
      buffer.createAnchor(terminal.viewWidth, buffer.height - 1),
      mode: SelectionMode.line,
    );
  }

  /// Sends [text] to the shell as one paste: line ends become the Enter key's `\r`, and when the shell asked for bracketed paste the
  /// whole is wrapped in its markers, so it can tell a paste from typing. Before the shell is up it waits, as typing does.
  void paste(String text) {
    if (text.isEmpty || (state != PaneState.running && state != PaneState.starting)) return;
    final bracketed = terminal.bracketedPasteMode && state == PaneState.running;
    // The end marker inside the text would end the paste early and leave the rest to run as typed.
    final lines = text.replaceAll(RegExp(r'\r\n|\n'), '\r');
    final clean = bracketed ? lines.replaceAll(_pasteEnd, '') : lines;
    if (state == PaneState.starting) {
      _pending.write(clean);
    } else {
      terminal.paste(clean);
    }
  }

  static const _pasteEnd = '\x1b[201~';

  /// Pastes the clipboard's text; the problem in words when it could not, or null.
  Future<String?> pasteClipboard() async {
    try {
      final text = await clipboard.read();
      if (text == null) return 'The clipboard holds no text.';
      paste(text);
      return null;
    } on ClipboardRefused catch (refused) {
      return refused.message;
    }
  }

  /// The last [lines] lines the screen holds, with the blank ones at the end dropped; null when it holds nothing.
  String? lastOutput({int lines = 200}) {
    final all = terminal.buffer.getText().split('\n');
    while (all.isNotEmpty && all.last.trim().isEmpty) {
      all.removeLast();
    }
    if (all.isEmpty) return null;
    final from = all.length > lines ? all.length - lines : 0;
    return all.sublist(from).map((line) => line.trimRight()).join('\n');
  }

  /// Ends the shell and what it started.
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    await _printing?.cancel();
    final remote = _remote;
    _remote = null;
    await remote?.close();
  }

  @override
  void dispose() {
    unawaited(close());
    focusNode.dispose();
    super.dispose();
  }
}

/// A tab: one shell, or two side by side.
class TerminalTab {
  TerminalTab(this.id, this.name, TerminalPane first) : panes = [first] {
    focused = first;
  }

  final int id;

  /// What the tab is called until the person renames it.
  String name;
  final List<TerminalPane> panes;
  late TerminalPane focused;
}

/// The shells open for one conversation, in tabs. Held by the surface's controller, so hiding the panel or moving the surface to
/// the other one leaves every shell running with the screen it has.
class TerminalWorkspace extends ChangeNotifier {
  TerminalWorkspace({required this.host, required this.cwd, this.shell, this.clipboard = const SystemTerminalClipboard()});

  final TerminalHost host;
  final String? cwd;

  /// The clipboard every shell here copies to and pastes from.
  final TerminalClipboard clipboard;

  /// The id of the shell each new tab starts, one the host listed; null for the host's own choice.
  final String? shell;
  final List<TerminalTab> tabs = [];
  TerminalTab? _active;
  int _made = 0;
  bool _released = false;
  bool _focusWished = false;

  /// The most shells open at once in one conversation; the host holds 8 to a link.
  static const limit = 8;

  int get shells => [for (final tab in tabs) ...tab.panes].length;
  bool get full => shells >= limit;

  TerminalTab? get active => _active ?? tabs.firstOrNull;

  /// The shell the keyboard is in.
  TerminalPane? get focused => active?.focused;

  /// Whether the person chose a shell since the last call (a new one, a split, another tab), which then takes the keyboard. The
  /// screen asks once it is built, since the tab chosen may not be drawn yet.
  bool takeFocusWish() {
    final wished = _focusWished;
    _focusWished = false;
    return wished;
  }

  TerminalPane _pane([String? own]) {
    final pane = TerminalPane(host: host, cwd: cwd, shell: own ?? shell, clipboard: clipboard);
    unawaited(pane.start());
    return pane;
  }

  /// A tab with a new shell, which becomes the one shown; null at [limit]. [shell] is the id of one the host listed for this tab
  /// alone; the workspace's own choice when null.
  TerminalTab? newTab({String? shell}) {
    if (_released || full) return null;
    final tab = TerminalTab(++_made, 'Shell $_made', _pane(shell));
    tabs.add(tab);
    _active = tab;
    _focusWished = true;
    notifyListeners();
    return tab;
  }

  void activate(TerminalTab tab) {
    if (!tabs.contains(tab)) return;
    _active = tab;
    _focusWished = true;
    notifyListeners();
  }

  void focus(TerminalTab tab, TerminalPane pane) {
    if (!tab.panes.contains(pane) || identical(tab.focused, pane)) return;
    tab.focused = pane;
    notifyListeners();
  }

  void rename(TerminalTab tab, String name) {
    final clean = name.trim();
    if (clean.isEmpty || clean == tab.name) return;
    tab.name = clean;
    notifyListeners();
  }

  /// Puts a second shell beside the tab's; false when it has two already or the limit is reached.
  bool split(TerminalTab tab) {
    if (_released || full || tab.panes.length >= 2 || !tabs.contains(tab)) return false;
    final pane = _pane();
    tab.panes.add(pane);
    tab.focused = pane;
    _focusWished = true;
    notifyListeners();
    return true;
  }

  /// Ends one shell of a tab; the tab goes with its last one.
  void closePane(TerminalTab tab, TerminalPane pane) {
    if (!tab.panes.remove(pane)) return;
    unawaited(pane.close().whenComplete(pane.dispose));
    if (tab.panes.isEmpty) {
      final at = tabs.indexOf(tab);
      tabs.remove(tab);
      if (identical(_active, tab)) _active = tabs.isEmpty ? null : tabs[at.clamp(0, tabs.length - 1)];
    } else if (identical(tab.focused, pane)) {
      tab.focused = tab.panes.first;
    }
    _focusWished = tabs.isNotEmpty;
    notifyListeners();
  }

  void closeTab(TerminalTab tab) {
    for (final pane in [...tab.panes]) {
      closePane(tab, pane);
    }
  }

  /// Ends every shell: the conversation's surface was closed.
  Future<void> release() async {
    if (_released) return;
    _released = true;
    final panes = [for (final tab in tabs) ...tab.panes];
    tabs.clear();
    _active = null;
    await Future.wait([for (final pane in panes) pane.close()]);
    for (final pane in panes) {
      pane.dispose();
    }
  }
}
