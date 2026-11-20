// The terminal panel as a bare Flutter page for a browser test: one shell whose output the test prints and whose input the test reads,
// so copy and paste run through the engine's real keyboard handling and the browser's own clipboard events. Built beside the app with
//   flutter build web --release --no-web-resources-cdn -t tool/terminal_page.dart -o <dir>
// and driven by `clients/browser/chromium/e2e/terminal_clipboard.mjs`. `?read=1` makes the page read the clipboard on a paste key
// (what the app would do if the browser had no paste event), the control that shows the event is what needs no permission.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/settings/shell_settings.dart' show HostShell;
import 'package:canoa/surfaces/terminal_clipboard.dart';
import 'package:canoa/surfaces/terminal_controller.dart';
import 'package:canoa/surfaces/terminal_surface.dart';
import 'package:flutter/material.dart';

final class _Shell implements RemoteTerminal {
  final output$ = StreamController<List<int>>();
  final writes = <String>[];

  @override
  String get folder => '/work';
  @override
  String get shell => 'page-shell';
  @override
  Stream<List<int>> get output => output$.stream;
  @override
  Future<int?> get exited => Completer<int?>().future;
  @override
  Future<void> write(String text) async => writes.add(text);
  @override
  Future<void> resize(int cols, int rows) async {}
  @override
  Future<void> close() async {}
}

final class _Host implements TerminalHost {
  _Host(this.shell);

  final _Shell shell;

  @override
  Future<RemoteTerminal> open({String? cwd, String? shell, required int cols, required int rows}) async => this.shell;

  @override
  Future<List<HostShell>> shells() async => const [];
}

/// The system clipboard that reads on a paste key instead of waiting for the browser's event.
final class _ReadsOnKey implements TerminalClipboard {
  const _ReadsOnKey(this._real);

  final TerminalClipboard _real;

  @override
  Future<void> write(String text) => _real.write(text);
  @override
  Future<String?> read() => _real.read();
  @override
  VoidCallback onPaste(bool Function(String text) handler) => _real.onPaste(handler);
  @override
  bool get pastesByEvent => false;
}

void main() {
  final shell = _Shell();
  const system = SystemTerminalClipboard();
  final clipboard = Uri.base.queryParameters['read'] == '1' ? const _ReadsOnKey(system) : system;
  final workspace = TerminalWorkspace(host: _Host(shell), cwd: '/work', clipboard: clipboard)..newTab();
  globalContext['terminalWrites'] = (() => jsonEncode(shell.writes).toJS).toJS;
  globalContext['terminalPrint'] = ((JSString text) => shell.output$.add(utf8.encode(text.toDart))).toJS;
  globalContext['terminalSelection'] = (() => (workspace.focused?.selection ?? '').toJS).toJS;
  runApp(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: ThemeData.dark(),
    home: Scaffold(body: TerminalPanel(workspace: workspace, focusOnMount: true)),
  ));
}
