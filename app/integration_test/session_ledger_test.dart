// Sessions from two harnesses outlive the app, are listed by harness, and one opens again, in the app's own window through its
// widgets. The scripted stand-in (scripted_agent.mjs) plays three agents: one reporting Claude's adapter, which the built-in map
// names Claude Code; one reporting a name that Canoa.cfg's [harnesses] maps to Pi; and one that reports no name and offers
// neither session/load nor session/resume, which the list must say it cannot reopen.
//
// Two runs of the app, each its own process, sharing CANOA_TEST_LEDGER_DIR (an empty directory before the first):
//
//   CANOA_TEST_LEDGER_STEP=record flutter test integration_test/session_ledger_test.dart -d windows   (or -d linux)
//   CANOA_TEST_LEDGER_STEP=reopen flutter test integration_test/session_ledger_test.dart -d windows
//
// The second fails if it runs in the process of the first, and deletes the directory when it passes. `node` has to be on PATH.
// CANOA_TEST_SHOTS names a directory for a screenshot of each step; CANOA_TEST_APP is this app's directory (default: the
// working directory).
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/ledger_backend.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

final String? _dir = Platform.environment['CANOA_TEST_LEDGER_DIR'];
final String? _step = Platform.environment['CANOA_TEST_LEDGER_STEP'];
final String? _shots = Platform.environment['CANOA_TEST_SHOTS'];
final String _appDir = (Platform.environment['CANOA_TEST_APP'] ?? Directory.current.path).replaceAll(r'\', '/');

const _claude = 'Stand-in Claude', _pi = 'Stand-in Pi', _plain = 'Stand-in plain';

/// Canoa.cfg for the three agents, each keeping its sessions in a file of its own under [dir].
String _config(String dir) {
  String agent(String label, String state, List<String> extra) =>
      '[agents.${jsonEncode(label)}]\nprogram = "node"\n'
      'args = ${jsonEncode(['$_appDir/integration_test/scripted_agent.mjs', '--state=$dir/$state', ...extra])}\n\n';
  return '${agent(_claude, 'claude.json', ['--name=@agentclientprotocol/claude-agent-acp', '--version=0.99.1'])}'
      '${agent(_pi, 'pi.json', ['--name=stand-in-pi', '--version=0.0.34'])}'
      '${agent(_plain, 'plain.json', ['--no-reopen'])}'
      '[sessions]\nkeep = 50\n\n[harnesses]\n"stand-in-pi" = "Pi"\n';
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('sessions by harness outlive the app, and one opens again', (tester) async {
    if (_dir == null || (_step != 'record' && _step != 'reopen')) {
      fail('Set CANOA_TEST_LEDGER_DIR and CANOA_TEST_LEDGER_STEP=record, then =reopen');
    }
    final dir = _dir!.replaceAll(r'\', '/');
    Directory(dir).createSync(recursive: true);
    final configFile = File('$dir/Canoa.cfg');
    if (_step == 'record') configFile.writeAsStringSync(_config(dir));
    final config = CanoaConfig.parse(configFile.readAsStringSync());
    // The app's own place for the list: beside Canoa.cfg.
    final ledger = SessionLedger(platformLedgerBackend(configPath: configFile.path), settings: config.sessions);
    await ledger.load();
    final store = SessionStore(includePreview: false, ledger: ledger);
    final screen = GlobalKey();
    // The folder dialog is the operating system's: the test answers it with each agent's workspace in turn.
    var folder = dir;
    await tester.pumpWidget(RepaintBoundary(key: screen, child: CanoaApp(store: store, config: config, pickFolder: () async => folder)));
    await tester.pumpAndSettle();

    var shotNumber = 0;
    Future<void> shot(String name) async {
      await tester.pump();
      if (_shots == null) return;
      final boundary = tester.renderObject<RenderRepaintBoundary>(find.byKey(screen));
      final image = await boundary.toImage();
      final png = await image.toByteData(format: ui.ImageByteFormat.png);
      image.dispose();
      final file = File('$_shots/$_step-${(++shotNumber).toString().padLeft(2, '0')}-$name.png');
      file.writeAsBytesSync(png!.buffer.asUint8List());
      debugPrint('SHOT ${file.path}');
    }

    Future<void> until(bool Function() done, String what, {Duration within = const Duration(minutes: 1)}) async {
      final deadline = DateTime.now().add(within);
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) {
          await shot('timed-out');
          fail('timed out waiting for $what');
        }
        await tester.pump(const Duration(milliseconds: 200));
      }
    }

    bool showing(Finder finder) => finder.evaluate().isNotEmpty;
    bool said(String text) => store.selected?.messages.any((message) => message.text == text) ?? false;
    final composer = find.descendant(of: find.byKey(const Key('composer')), matching: find.byType(TextField));

    Future<void> send(String text) async {
      await tester.tap(composer);
      await tester.enterText(composer, text);
      await tester.tap(find.byTooltip('Send'));
      await until(() => said(text.substring('Say '.length)), 'the agent to answer "$text"');
      await tester.pumpAndSettle();
    }

    Finder tileOf(String sessionTitle) => find.ancestor(of: find.text(sessionTitle), matching: find.byType(ListTile)).first;

    /// Scrolls to [target] and taps it once a tap at its centre reaches it, so a tap is never spent on whatever covers it.
    Future<void> tapReachable(Finder target) async {
      // Under WSLg a focused field keeps scrolling itself back into view, so it is left first, as a click elsewhere would.
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pumpAndSettle();
      await tester.ensureVisible(target);
      await tester.pumpAndSettle();
      await until(() {
        final box = tester.renderObject<RenderBox>(target);
        final centre = box.localToGlobal(box.size.center(Offset.zero));
        return tester.hitTestOnBinding(centre).path.any((entry) => entry.target == box);
      }, 'it to be where a tap reaches', within: const Duration(seconds: 10));
      await tester.tap(target);
    }

    /// Presses the button named [tooltip] in the side list's row [tile].
    Future<void> tapIn(Finder tile, String tooltip) =>
        tapReachable(find.descendant(of: tile, matching: find.byTooltip(tooltip)));

    Future<void> closeAll() async {
      while (store.sessions.isNotEmpty) {
        await tester.tap(find.byTooltip('Close conversation'));
        await tester.pumpAndSettle();
      }
    }

    if (_step == 'record') {
      for (final (label, repo) in [(_claude, 'repo-a'), (_pi, 'repo-b'), (_plain, 'repo-c')]) {
        final workspace = Directory('$dir/$repo')..createSync();
        folder = workspace.path;
        // A new session, which starts nothing until its first message.
        await tester.tap(find.byTooltip('New session'));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('new-session-source')));
        await until(() => showing(find.byKey(Key('source $label'))), 'the $label entry to be offered', within: const Duration(seconds: 30));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(Key('source $label')).last);
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('new-session-folder')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('new-session-browse')));
        await tester.pumpAndSettle();
        expect(ledger.entries.where((entry) => entry.entry == label), isEmpty, reason: 'the ledger keeps a session once it starts');
        await send('Say hello from $label');
      }
      await shot('three-open');
      await ledger.flush();
      final written = File('$dir/sessions.json').readAsStringSync();
      final sessions = (jsonDecode(written) as Map)['sessions'] as List;
      debugPrint('RECORDED ${[for (final entry in sessions) '${entry['harness']} ${entry['version'] ?? '-'} ${entry['reopenBy'] ?? 'no reopen'}'].join('; ')}');
      await closeAll();
      await ledger.flush();
      await shot('all-closed');
      File('$dir/first.pid').writeAsStringSync('$pid');
      expect([for (final entry in sessions) entry['harness']], unorderedEquals(['Claude Code', 'Pi', _plain]));
      expect(written, isNot(contains('hello from')), reason: 'no message text unless the person turns that on');
      expect(showing(find.byKey(const ValueKey('harness Claude Code'))), isTrue, reason: 'a closed session is listed by harness');
      return;
    }

    // The second run.
    final firstPid = int.parse(File('$dir/first.pid').readAsStringSync());
    debugPrint('RESTART first run pid $firstPid, this run pid $pid');
    expect(pid, isNot(firstPid), reason: 'the app ended and started again');
    await shot('after-restart');
    final listed = {for (final harness in ['Claude Code', 'Pi', _plain]) harness: showing(find.byKey(ValueKey('harness $harness')))};
    debugPrint('LISTED $listed');
    expect(listed.values, everyElement(isTrue), reason: 'each harness has its group after the restart');
    expect(showing(find.textContaining('claude-agent-acp 0.99.1')), isTrue, reason: 'the version shows, named by its adapter');
    final plainTile = tileOf(_plain);
    expect(showing(find.descendant(of: plainTile, matching: find.textContaining('Cannot be reopened: $_plain offers neither session/load nor session/resume.'))), isTrue);
    expect(showing(find.descendant(of: plainTile, matching: find.byTooltip('Reopen'))), isFalse);

    await tester.tap(find.widgetWithText(ChoiceChip, 'Pi'));
    await tester.pumpAndSettle();
    await shot('filtered-pi');
    // By the sessions' own rows: a harness's chip stays whichever is picked.
    bool row(String title) => showing(find.ancestor(of: find.text(title), matching: find.byType(ListTile)));
    final filtered = (row(_pi), row(_claude), row(_plain));
    debugPrint('FILTER Pi shows (Pi, Claude, plain) $filtered');
    expect(filtered, (true, false, false));
    await tester.tap(find.widgetWithText(ChoiceChip, 'All'));
    await tester.pumpAndSettle();

    final claudeEntry = ledger.entries.firstWhere((entry) => entry.harness == 'Claude Code');
    final usedBefore = claudeEntry.lastUsed;
    await tapIn(tileOf(_claude), 'Reopen');
    await until(() => store.sessions.length == 1 && said('hello from $_claude'), 'the Claude Code session to open again with what was said');
    await tester.pumpAndSettle();
    await shot('reopened');
    await send('Say still here');
    await shot('reopened-answers');

    final piEntry = ledger.entries.firstWhere((entry) => entry.harness == 'Pi');
    await tapIn(tileOf(_pi), 'Remove from this list');
    await tester.pumpAndSettle();
    final warned = showing(find.textContaining('This does not delete the session itself: Pi still has it.'));
    await shot('remove-asks');
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    await ledger.flush();
    final piState = jsonDecode(File('$dir/pi.json').readAsStringSync()) as Map;
    final again = SessionLedger(platformLedgerBackend(configPath: configFile.path), settings: config.sessions);
    await again.load();
    final reopenedEntry = again.find(claudeEntry.peerId, claudeEntry.sessionId);
    debugPrint('AFTER kept ${again.entries.map((entry) => entry.harness).toList()}; Pi agent still has ${piState.keys.toList()}');

    await closeAll();
    await ledger.flush();
    final closedAt = DateTime.now();
    while (true) {
      try {
        Directory(dir).deleteSync(recursive: true);
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closedAt) > const Duration(seconds: 30)) fail('something an agent started is still running');
        await tester.pump(const Duration(milliseconds: 200));
      }
    }

    expect(warned, isTrue, reason: 'removing says the agent keeps its session');
    expect(again.entries.map((entry) => entry.harness), unorderedEquals(['Claude Code', _plain]));
    expect(piState.keys, contains(piEntry.sessionId), reason: 'the agent\'s own session was not deleted');
    expect(reopenedEntry!.lastUsed.isAfter(usedBefore), isTrue, reason: 'reopening counts as use');
  }, skip: !Platform.isWindows && !Platform.isLinux, timeout: const Timeout(Duration(minutes: 15)));
}
