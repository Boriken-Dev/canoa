// Letters typed on the keyboard reach a real shell in the desktop app, in both places a terminal opens: a conversation's bottom
// panel and the Terminal section. The app hosts itself from its library on a home in DIR; each shell starts in DIR and is given a
// click on its screen and the keys of `set-content <file> abc`, then Enter, as Windows delivers a person's (windows_input.dart),
// with no test text input in between; the file it writes says whether the shell got the line.
//
//   CANOA_CONFIG=DIR/canoa.cfg [CANOA_TEST_INPUT=device|posted] [CANOA_TEST_TERMINAL_SHELL=pwsh] \
//     flutter test integration_test/terminal_input_test.dart -d windows
//
// DIR is a new empty directory. CANOA_TEST_INPUT `device` (the default) sends through SendInput, as the keyboard and mouse do; it
// needs an unlocked desktop and brings the window in front, so keep hands off the keyboard and mouse while it runs, and it sends
// nothing while another window is in front. `posted` posts the messages to the engine's window instead, which works on a locked
// desktop. The shell is Windows PowerShell unless CANOA_TEST_TERMINAL_SHELL names another of the host's.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import 'windows_input.dart';

/// The text of a file a shell wrote: Windows PowerShell writes UTF-16 with a byte order mark, the others UTF-8.
String _shellText(List<int> bytes) {
  if (bytes.length >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE) {
    return String.fromCharCodes([for (var at = 2; at + 1 < bytes.length; at += 2) bytes[at] | bytes[at + 1] << 8]);
  }
  return utf8.decode(bytes, allowMalformed: true);
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final environment = Platform.environment;
  final cfgPath = environment['CANOA_CONFIG'];
  final mode = WindowsInputMode.values.byName(environment['CANOA_TEST_INPUT'] ?? 'device');
  final skip = !Platform.isWindows
      ? 'the input is Windows\''
      : cfgPath == null
      ? 'CANOA_CONFIG names a canoa.cfg in a new directory'
      : null;

  testWidgets('letters typed on the keyboard reach the shell', skip: skip != null, (tester) async {
    final dir = File(cfgPath!).parent.path;
    expect(Directory(dir).listSync(), isEmpty, reason: 'DIR is a new directory');
    // The agent never starts: a session needs one to be made, and the terminal does not.
    final agent = AgentCommand(label: 'Unstarted', program: Platform.resolvedExecutable);
    final cfg = File(cfgPath)
      ..writeAsStringSync('[host]\n'
          // Windows PowerShell is on every Windows; the host's default, `$SHELL`, would be whatever started the test.
          'shell = "${environment['CANOA_TEST_TERMINAL_SHELL'] ?? 'powershell'}"\n\n'
          '[agents.${agent.label}]\nprogram = ${jsonEncode(agent.program)}\n');
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    final host = HostSupervisor(LibraryHostTool(CanoaNative.open(), home: '$dir${Platform.pathSeparator}home'));
    await host.apply(configPath: cfg.path);
    addTearDown(host.stop);
    await tester.pumpWidget(CanoaApp(store: store, config: CanoaConfig(agents: [agent]), host: host));
    await tester.pumpAndSettle();

    Future<void> until(bool Function() done, String what, {int seconds = 30}) async {
      final deadline = DateTime.now().add(Duration(seconds: seconds));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    // What the framework hears of each key, for the log; it takes none of them.
    final heard = StringBuffer();
    bool listen(KeyEvent event) {
      if (event is KeyDownEvent) heard.write(event.character ?? '<${event.logicalKey.keyLabel}>');
      return false;
    }

    HardwareKeyboard.instance.addHandler(listen);
    addTearDown(() => HardwareKeyboard.instance.removeHandler(listen));
    final input = WindowsInput.ofThisProcess(mode);
    debugPrint('input: ${mode.name}');

    /// Clicks the shell's screen and types a line that writes [name] in DIR; fails unless the shell wrote it.
    Future<void> typeInto(String place, String name) async {
      final screen = find.byType(TerminalView);
      await until(() => screen.evaluate().isNotEmpty, 'the terminal in $place');
      final view = tester.widget<TerminalView>(screen);
      String text() => view.terminal.buffer.getText().trimRight();
      await until(() => text().contains('PS '), 'the prompt in $place', seconds: 60);
      await input.ready();
      // The device's own pointer events reach the app only while this is set.
      binding.shouldPropagateDevicePointerEvents = true;
      try {
        final at = tester.getCenter(screen) * tester.view.devicePixelRatio;
        await input.click(at.dx, at.dy);
        await tester.pump(const Duration(milliseconds: 300));
      } finally {
        binding.shouldPropagateDevicePointerEvents = false;
      }
      debugPrint('$place: after the click the keyboard is with ${FocusManager.instance.primaryFocus}');
      expect(FocusManager.instance.primaryFocus, view.focusNode, reason: 'a click on the screen gives the shell the keyboard');

      heard.clear();
      final file = File('$dir${Platform.pathSeparator}$name');
      final line = 'set-content $name abc';
      await input.type(line);
      await tester.pump(const Duration(milliseconds: 500));
      await input.key('enter');
      final deadline = DateTime.now().add(const Duration(seconds: 15));
      while (!file.existsSync() && DateTime.now().isBefore(deadline)) {
        await tester.pump(const Duration(milliseconds: 200));
      }
      await tester.pump(const Duration(milliseconds: 300));
      debugPrint('$place: keys the framework heard: $heard');
      debugPrint('$place: the screen:\n${text()}');
      expect(file.existsSync(), isTrue, reason: 'the shell in $place ran the line typed into it: $line');
      expect(_shellText(file.readAsBytesSync()).trim(), 'abc');
    }

    // A conversation's bottom panel: a new session in DIR, then the panel's key, which opens the Terminal.
    await tester.tap(find.byTooltip('New session'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('new-session-source')));
    await until(() => find.byKey(Key('source ${agent.label}')).evaluate().isNotEmpty, 'the source');
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(Key('source ${agent.label}')).last);
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    store.updateDraft(store.selected!, cwd: dir);
    await tester.pumpAndSettle();
    await tester.tap(find.descendant(of: find.byKey(const Key('composer')), matching: find.byType(TextField)));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.backquote);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await typeInto('the panel', 'panel.txt');

    // The Terminal section: New shell, in DIR, of the host's own choice.
    await tester.tap(find.byKey(const ValueKey('section terminal')));
    await until(() => find.byKey(const ValueKey('terminal-section start')).evaluate().isNotEmpty, 'the Terminal section');
    await tester.tap(find.byKey(const ValueKey('terminal-section start')));
    await until(() => find.byKey(const ValueKey('terminal-section shell')).evaluate().isNotEmpty, 'the host\'s shells');
    await tester.pumpAndSettle();
    // Each shell the host found is offered once.
    await tester.tap(find.byKey(const ValueKey('terminal-section shell')));
    await tester.pumpAndSettle();
    final offered = [
      for (final item in tester.widgetList<DropdownMenuItem<String?>>(find.byType(DropdownMenuItem<String?>)))
        if (item.child case Text(:final data?)) data,
    ];
    // The closed button holds the chosen item, and the open menu every item.
    final shells = offered.skip(1).toList();
    debugPrint('the shells offered: $shells');
    expect(shells.toSet(), hasLength(shells.length), reason: 'no shell is offered twice');
    await tester.tap(find.text('The host\'s own choice').last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const ValueKey('terminal-section folder')), dir);
    await tester.tap(find.byKey(const ValueKey('terminal-section open')));
    await typeInto('the Terminal section', 'section.txt');
  });
}
